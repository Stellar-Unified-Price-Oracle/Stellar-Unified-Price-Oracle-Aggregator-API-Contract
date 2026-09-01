#!/usr/bin/env python3
"""Prometheus metrics exporter for the Stellar Unified Price Oracle (monitoring v2).

Polls the Soroban RPC `getEvents` endpoint for one or more deployed oracle
contracts, derives the v1 metrics documented in docs/monitoring/README.md plus
the v2 freshness/deviation/source-health metrics consumed by
docs/monitoring/alerts-v2.yml, and serves them on a `/metrics` HTTP endpoint.

This is the "event index" -> Prometheus exporter referenced by the monitoring
v2 issue: event parsing lives in `EventIndex`, which is deliberately
RPC/HTTP-free so it can be unit tested (see scripts/test_metrics_exporter.py)
independent of a live network.

Usage:
    python3 scripts/metrics_exporter.py --config scripts/metrics_exporter_config.example.toml

Configuration follows the same tomllib-based pattern as
scripts/price-submission-bot.py / scripts/executor_config.example.json.
"""
from __future__ import annotations

import argparse
import logging
import time
import tomllib
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, HTTPServer
from threading import Lock, Thread
from typing import Optional

import requests

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("oracle-metrics-exporter")

# Error name lookup mirrors contracts/price-oracle/src/errors.rs; kept in sync
# manually since the exporter only sees the numeric discriminant on-chain.
ERROR_NAMES = {
    0: "NotAuthorized",
    1: "AlreadyInitialized",
    2: "AssetNotRegistered",
    3: "AssetAlreadyRegistered",
    4: "SourceAlreadyExists",
    5: "SourceNotFound",
    6: "InsufficientSources",
    7: "InvalidPrice",
    8: "NoData",
}

# Deviation threshold used to flag a submission (SLA §3 default max_price_deviation).
DEFAULT_MAX_DEVIATION_BPS = 500


@dataclass
class ContractConfig:
    contract_id: str
    label: str = ""


@dataclass
class ExporterConfig:
    rpc_url: str = "https://soroban-testnet.stellar.org"
    poll_interval_secs: int = 15
    listen_host: str = "0.0.0.0"
    listen_port: int = 9464
    contracts: list[ContractConfig] = field(default_factory=list)


def load_config(path: str) -> ExporterConfig:
    with open(path, "rb") as f:
        raw = tomllib.load(f)
    contracts = [ContractConfig(**c) for c in raw.get("contracts", [])]
    return ExporterConfig(
        rpc_url=raw.get("rpc_url", "https://soroban-testnet.stellar.org"),
        poll_interval_secs=raw.get("poll_interval_secs", 15),
        listen_host=raw.get("listen_host", "0.0.0.0"),
        listen_port=raw.get("listen_port", 9464),
        contracts=contracts,
    )


class EventIndex:
    """Pure in-memory event -> metric derivation, no network I/O.

    Kept separate from the RPC polling loop so the freshness/deviation/source
    health math can be exercised by synthetic events in unit tests without a
    live RPC endpoint.
    """

    def __init__(self, contract_id: str, max_deviation_bps: int = DEFAULT_MAX_DEVIATION_BPS):
        self.contract_id = contract_id
        self.max_deviation_bps = max_deviation_bps
        self._lock = Lock()

        # Gauges
        self.registered_sources_total = 0
        self.active_sources_total = 0
        self.registered_assets_total = 0
        self.min_sources_required = 1
        self.paused = 0
        self.latest_price: dict[str, int] = {}
        self.last_price_timestamp: dict[str, int] = {}
        self.source_price: dict[tuple[str, str], int] = {}
        self.source_last_submission_ts: dict[tuple[str, str], int] = {}
        self.source_deviation_bps: dict[tuple[str, str], int] = {}

        # Counters
        self.price_submissions_total: dict[tuple[str, str], int] = {}
        self.price_updated_events_total: dict[str, int] = {}
        self.errors_total: dict[int, int] = {}
        self.config_change_events_total: dict[str, int] = {}

    def handle_event(self, topics: list, data: dict, ledger_close_time: int) -> None:
        """Applies one decoded contract event to the running metric state.

        `topics` is the decoded topic list (event name first); `data` is the
        decoded event payload dict; `ledger_close_time` is the event's ledger
        close unix timestamp (used as the metric observation time, since it is
        deterministic and doesn't depend on exporter poll cadence).
        """
        if not topics:
            return
        name = topics[0]
        with self._lock:
            if name == "PriceSubmitted":
                asset, source_name = topics[1], topics[2]
                price = int(data["price"])
                self.price_submissions_total[(asset, source_name)] = (
                    self.price_submissions_total.get((asset, source_name), 0) + 1
                )
                self.source_price[(asset, source_name)] = price
                self.source_last_submission_ts[(asset, source_name)] = ledger_close_time

                current = self.latest_price.get(asset)
                if current:
                    deviation_bps = abs(price - current) * 10_000 // max(current, 1)
                    self.source_deviation_bps[(asset, source_name)] = deviation_bps

            elif name == "PriceUpdated":
                asset = topics[1]
                new_price = int(data["new_price"])
                self.price_updated_events_total[asset] = self.price_updated_events_total.get(asset, 0) + 1
                self.latest_price[asset] = new_price
                self.last_price_timestamp[asset] = ledger_close_time

            elif name == "SourceAdded":
                self.registered_sources_total += 1
                self.active_sources_total += 1
                self.config_change_events_total["SourceAdded"] = (
                    self.config_change_events_total.get("SourceAdded", 0) + 1
                )

            elif name == "SourceRemoved":
                self.registered_sources_total = max(0, self.registered_sources_total - 1)
                self.active_sources_total = max(0, self.active_sources_total - 1)
                self.config_change_events_total["SourceRemoved"] = (
                    self.config_change_events_total.get("SourceRemoved", 0) + 1
                )

            elif name == "SourceSuspended":
                self.active_sources_total = max(0, self.active_sources_total - 1)

            elif name == "SourceReinstated":
                self.active_sources_total = min(self.registered_sources_total, self.active_sources_total + 1)

            elif name == "AssetRegistered":
                self.registered_assets_total += 1
                self.config_change_events_total["AssetRegistered"] = (
                    self.config_change_events_total.get("AssetRegistered", 0) + 1
                )

            elif name == "AssetUnregistered":
                self.registered_assets_total = max(0, self.registered_assets_total - 1)
                self.config_change_events_total["AssetUnregistered"] = (
                    self.config_change_events_total.get("AssetUnregistered", 0) + 1
                )

            elif name == "ContractPaused":
                self.paused = 1

            elif name == "ContractUnpaused":
                self.paused = 0

            elif name == "AdminChanged":
                self.config_change_events_total["AdminChanged"] = (
                    self.config_change_events_total.get("AdminChanged", 0) + 1
                )

            elif name == "ContractUpgraded":
                self.config_change_events_total["ContractUpgraded"] = (
                    self.config_change_events_total.get("ContractUpgraded", 0) + 1
                )

            elif name == "MinSourcesRequiredChanged":
                self.min_sources_required = int(data["value"])

            elif name == "Error":
                code = int(data.get("code", -1))
                self.errors_total[code] = self.errors_total.get(code, 0) + 1

    def render(self, now: float) -> str:
        cid = self.contract_id
        lines: list[str] = []

        def g(metric: str, value, **labels):
            label_str = ",".join([f'contract_id="{cid}"'] + [f'{k}="{v}"' for k, v in labels.items()])
            lines.append(f"{metric}{{{label_str}}} {value}")

        with self._lock:
            g("oracle_registered_sources_total", self.registered_sources_total)
            g("oracle_active_sources_total", self.active_sources_total)
            g("oracle_registered_assets_total", self.registered_assets_total)
            g("oracle_config_min_sources_required", self.min_sources_required)
            g("oracle_paused", self.paused)

            for asset, price in self.latest_price.items():
                g("oracle_latest_price", price, asset=asset)
            for asset, ts in self.last_price_timestamp.items():
                g("oracle_last_price_timestamp_seconds", ts, asset=asset)
            for (asset, source_name), price in self.source_price.items():
                g("oracle_source_price", price, asset=asset, source_name=source_name)
            for (asset, source_name), ts in self.source_last_submission_ts.items():
                g("oracle_source_last_submission_timestamp_seconds", ts, asset=asset, source_name=source_name)
            for (asset, source_name), bps in self.source_deviation_bps.items():
                g("oracle_source_deviation_bps", bps, asset=asset, source_name=source_name)
            for (asset, source_name), count in self.price_submissions_total.items():
                g("oracle_price_submissions_total", count, asset=asset, source_name=source_name)
            for asset, count in self.price_updated_events_total.items():
                g("oracle_price_updated_events_total", count, asset=asset)
            for code, count in self.errors_total.items():
                g("oracle_errors_total", count, error_code=code, error_name=ERROR_NAMES.get(code, "Unknown"))
            for event_type, count in self.config_change_events_total.items():
                g("oracle_config_change_events_total", count, event_type=event_type)

        return "\n".join(lines) + "\n"


def poll_events(index: EventIndex, rpc_url: str, start_ledger: int) -> int:
    """Fetches new events since `start_ledger` and applies them to `index`.

    Returns the next `start_ledger` to poll from. Isolated from EventIndex so
    the RPC/decoding boundary is a single, mockable function.
    """
    resp = requests.post(
        rpc_url,
        json={
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getEvents",
            "params": {
                "startLedger": start_ledger,
                "filters": [{"type": "contract", "contractIds": [index.contract_id]}],
                "pagination": {"limit": 100},
            },
        },
        timeout=15,
    )
    resp.raise_for_status()
    result = resp.json().get("result", {})
    for event in result.get("events", []):
        topics = [_decode_scval(t) for t in event.get("topic", [])]
        data = _decode_scval(event.get("value", {})) or {}
        index.handle_event(topics, data if isinstance(data, dict) else {}, event.get("ledgerClosedAt", 0))
    return result.get("latestLedger", start_ledger)


def _decode_scval(_scval) -> Optional[dict]:
    """Placeholder XDR ScVal decoder.

    Production deployments should use `stellar_sdk.scval` to decode topics and
    values into native Python types. Left unimplemented here to keep this
    script dependency-light; see docs/monitoring/README.md#v2 for the
    stellar-sdk-based decoding snippet.
    """
    raise NotImplementedError("wire up stellar_sdk.scval.to_native() here for a live deployment")


_indices: dict[str, EventIndex] = {}


class MetricsHandler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 - required by BaseHTTPRequestHandler
        if self.path != "/metrics":
            self.send_response(404)
            self.end_headers()
            return
        now = time.time()
        body = "".join(idx.render(now) for idx in _indices.values())
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; version=0.0.4")
        self.end_headers()
        self.wfile.write(body.encode())

    def log_message(self, *args):  # silence default request logging
        pass


def run_poll_loop(config: ExporterConfig) -> None:
    start_ledgers = {c.contract_id: 0 for c in config.contracts}
    while True:
        for contract in config.contracts:
            index = _indices[contract.contract_id]
            try:
                start_ledgers[contract.contract_id] = poll_events(
                    index, config.rpc_url, start_ledgers[contract.contract_id]
                )
            except Exception as exc:  # a single contract's poll failure shouldn't kill the loop
                log.warning("poll failed for %s: %s", contract.contract_id, exc)
        time.sleep(config.poll_interval_secs)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True)
    args = parser.parse_args()

    config = load_config(args.config)
    for contract in config.contracts:
        _indices[contract.contract_id] = EventIndex(contract.contract_id)

    server = HTTPServer((config.listen_host, config.listen_port), MetricsHandler)
    Thread(target=server.serve_forever, daemon=True).start()
    log.info("metrics exporter listening on %s:%d/metrics", config.listen_host, config.listen_port)

    run_poll_loop(config)


if __name__ == "__main__":
    main()
