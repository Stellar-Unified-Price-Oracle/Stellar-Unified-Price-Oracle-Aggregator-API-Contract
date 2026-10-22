#!/usr/bin/env python3
"""Source accuracy calibration runner for the Stellar Unified Price Oracle.

This is the off-chain half of the calibration framework in
contracts/price-oracle/src/calibration.rs: on a schedule, for each configured
asset it

1. fetches an external reference price (CoinGecko by default),
2. pushes it on-chain via `calibration_set_benchmark`,
3. reads each configured source's last submission via the existing
   `get_source_price` query,
4. records an accuracy sample for that source via `calibration_record_sample`
   (which updates the source's rolling EMA accuracy score on-chain), and
5. fetches `calibration_report` and prints a per-source accuracy report.

Runs against testnet by default (see calibration_runner_config.example.toml),
using the `stellar` CLI for both view calls and signed transactions, following
the same subprocess-based invocation pattern as scripts/price-submission-bot.py.
"""
from __future__ import annotations

import argparse
import json
import logging
import subprocess
import time
import tomllib
from dataclasses import dataclass, field
from typing import Optional

import requests

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("calibration-runner")

COINGECKO_URL = "https://api.coingecko.com/api/v3/simple/price"


@dataclass
class SourceConfig:
    name: str
    address: str


@dataclass
class AssetConfig:
    address: str
    coingecko_id: str
    decimals: int = 8
    sources: list[SourceConfig] = field(default_factory=list)


@dataclass
class RunnerConfig:
    contract_id: str
    network: str = "testnet"
    admin_identity: str = "admin"
    interval_secs: int = 300
    assets: dict[str, AssetConfig] = field(default_factory=dict)


def load_config(path: str) -> RunnerConfig:
    with open(path, "rb") as f:
        raw = tomllib.load(f)

    oracle = raw["oracle"]
    assets: dict[str, AssetConfig] = {}
    for asset_name, asset_raw in raw.get("assets", {}).items():
        sources = [SourceConfig(name=s["name"], address=s["address"]) for s in asset_raw.get("sources", [])]
        assets[asset_name] = AssetConfig(
            address=asset_raw["address"],
            coingecko_id=asset_raw["coingecko_id"],
            decimals=asset_raw.get("decimals", 8),
            sources=sources,
        )

    return RunnerConfig(
        contract_id=oracle["contract_id"],
        network=oracle.get("network", "testnet"),
        admin_identity=oracle.get("admin_identity", "admin"),
        interval_secs=raw.get("schedule", {}).get("interval_secs", 300),
        assets=assets,
    )


def fetch_reference_price(coingecko_id: str) -> Optional[float]:
    try:
        resp = requests.get(COINGECKO_URL, params={"ids": coingecko_id, "vs_currencies": "usd"}, timeout=10)
        return resp.json()[coingecko_id]["usd"]
    except Exception as exc:
        log.warning("reference price fetch failed for %s: %s", coingecko_id, exc)
        return None


def _run_stellar(args: list[str], timeout: int = 30) -> str:
    result = subprocess.run(args, check=True, capture_output=True, timeout=timeout, text=True)
    return result.stdout.strip()


def invoke_tx(cfg: RunnerConfig, function: str, *fn_args: str, retries: int = 3) -> bool:
    """Invokes a state-changing (admin-signed) contract function, with retry/backoff."""
    for attempt in range(1, retries + 1):
        try:
            _run_stellar(
                [
                    "stellar", "contract", "invoke",
                    "--id", cfg.contract_id, "--source", cfg.admin_identity, "--network", cfg.network,
                    "--", function, *fn_args,
                ]
            )
            return True
        except Exception as exc:
            backoff = 2 ** attempt
            log.warning("%s attempt %d/%d failed: %s (retrying in %ds)", function, attempt, retries, exc, backoff)
            time.sleep(backoff)
    return False


def invoke_view(cfg: RunnerConfig, function: str, *fn_args: str):
    """Invokes a read-only contract function and returns its parsed JSON result, or None on failure."""
    try:
        stdout = _run_stellar(
            [
                "stellar", "contract", "invoke",
                "--id", cfg.contract_id, "--source", cfg.admin_identity, "--network", cfg.network,
                "--", function, *fn_args,
            ]
        )
        return json.loads(stdout) if stdout else None
    except Exception as exc:
        log.warning("%s view call failed: %s", function, exc)
        return None


def run_asset_cycle(cfg: RunnerConfig, asset_name: str, asset: AssetConfig) -> list[dict]:
    reference = fetch_reference_price(asset.coingecko_id)
    if reference is None:
        log.error("skipping %s: no reference price available this cycle", asset_name)
        return []

    reference_scaled = int(round(reference * (10 ** asset.decimals)))
    now = int(time.time())

    if not invoke_tx(
        cfg, "calibration_set_benchmark",
        "--asset", asset.address,
        "--reference_price", str(reference_scaled),
        "--decimals", str(asset.decimals),
        "--timestamp", str(now),
    ):
        log.error("failed to set benchmark for %s; skipping sample recording", asset_name)
        return []

    for source in asset.sources:
        entry = invoke_view(cfg, "get_source_price", "--asset", asset.address, "--source", source.address)
        if entry is None or "price" not in entry:
            log.warning("no submission found for source %s on %s this cycle", source.name, asset_name)
            continue

        source_price = int(entry["price"])
        if source_price <= 0:
            continue

        invoke_tx(
            cfg, "calibration_record_sample",
            "--asset", asset.address,
            "--source", source.address,
            "--source_price", str(source_price),
        )

    report = invoke_view(cfg, "calibration_report", "--asset", asset.address)
    return report or []


def print_report(asset_name: str, sources_by_address: dict[str, str], report: list[dict]) -> None:
    if not report:
        log.info("[%s] no calibration scores yet", asset_name)
        return
    log.info("[%s] per-source accuracy report:", asset_name)
    for score in report:
        source_addr = score.get("source", "?")
        label = sources_by_address.get(source_addr, source_addr)
        log.info(
            "    %-20s accuracy=%3d%% samples=%-4d last_updated=%s",
            label, score.get("rolling_accuracy", 0), score.get("sample_count", 0),
            score.get("last_updated", 0),
        )


def run_once(cfg: RunnerConfig) -> None:
    for asset_name, asset in cfg.assets.items():
        sources_by_address = {s.address: s.name for s in asset.sources}
        report = run_asset_cycle(cfg, asset_name, asset)
        print_report(asset_name, sources_by_address, report)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", default="scripts/calibration_runner_config.example.toml")
    parser.add_argument("--once", action="store_true", help="run a single calibration cycle and exit")
    args = parser.parse_args()

    cfg = load_config(args.config)
    log.info(
        "calibration runner starting: contract=%s network=%s assets=%s",
        cfg.contract_id, cfg.network, list(cfg.assets),
    )

    if args.once:
        run_once(cfg)
        return

    while True:
        run_once(cfg)
        time.sleep(cfg.interval_secs)


if __name__ == "__main__":
    main()
