#!/usr/bin/env python3
"""Production price submission bot for the Stellar Unified Price Oracle.

Fetches prices for configured assets from multiple exchanges (CoinGecko,
Binance, Kraken), aggregates a mid-price, and submits to the oracle
contract on a per-asset schedule with retry/backoff. Exposes a Prometheus
health/metrics endpoint. Configuration follows docs/price-submission-bot.md.
"""
from __future__ import annotations

import json
import logging
import os
import subprocess
import time
import tomllib
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, HTTPServer
from threading import Thread
from typing import Optional

import requests

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("oracle-bot")

METRICS = {
    "submissions_total": 0,
    "submission_errors_total": 0,
    "fetch_errors_total": {"coingecko": 0, "binance": 0, "kraken": 0},
    "last_submission_ts": {},
}

FETCHERS = {
    "coingecko": lambda feed_id: requests.get(
        "https://api.coingecko.com/api/v3/simple/price",
        params={"ids": feed_id, "vs_currencies": "usd"}, timeout=10,
    ).json()[feed_id]["usd"],
    "binance": lambda feed_id: float(requests.get(
        "https://api.binance.com/api/v3/ticker/price",
        params={"symbol": feed_id}, timeout=10,
    ).json()["price"]),
    "kraken": lambda feed_id: float(next(iter(requests.get(
        "https://api.kraken.com/0/public/Ticker",
        params={"pair": feed_id}, timeout=10,
    ).json()["result"].values()))["c"][0]),
}


@dataclass
class AssetConfig:
    contract_address: str
    feed_ids: dict = field(default_factory=dict)  # exchange -> feed id
    interval_secs: int = 60


def load_config(path: str) -> dict:
    with open(path, "rb") as f:
        return tomllib.load(f)


def fetch_mid_price(asset: AssetConfig) -> Optional[float]:
    prices = []
    for exchange, feed_id in asset.feed_ids.items():
        fetcher = FETCHERS.get(exchange)
        if not fetcher:
            continue
        try:
            prices.append(fetcher(feed_id))
        except Exception as exc:  # network/parse errors from a single source shouldn't halt the cycle
            METRICS["fetch_errors_total"][exchange] += 1
            log.warning("fetch failed for %s/%s: %s", exchange, feed_id, exc)
    if not prices:
        return None
    return sum(prices) / len(prices)


def submit_price(contract_id: str, network: str, source_identity: str, asset_address: str,
                  price: int, timestamp: int, retries: int = 3) -> bool:
    for attempt in range(1, retries + 1):
        try:
            subprocess.run(
                [
                    "stellar", "contract", "invoke",
                    "--id", contract_id, "--source", source_identity, "--network", network,
                    "--", "submit_price",
                    "--source", source_identity, "--asset", asset_address,
                    "--price", str(price), "--timestamp", str(timestamp),
                ],
                check=True, capture_output=True, timeout=30,
            )
            return True
        except Exception as exc:
            backoff = 2 ** attempt
            log.warning("submit attempt %d/%d failed: %s (retrying in %ds)", attempt, retries, exc, backoff)
            time.sleep(backoff)
    return False


class HealthHandler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 - required by BaseHTTPRequestHandler
        if self.path == "/metrics":
            lines = [
                f"oracle_bot_submissions_total {METRICS['submissions_total']}",
                f"oracle_bot_submission_errors_total {METRICS['submission_errors_total']}",
            ]
            for exchange, count in METRICS["fetch_errors_total"].items():
                lines.append(f'oracle_bot_fetch_errors_total{{exchange="{exchange}"}} {count}')
            body = "\n".join(lines) + "\n"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(body.encode())
        elif self.path == "/health":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({"status": "ok", "metrics": METRICS}).encode())
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, fmt, *args):  # silence default request logging
        pass


def run_health_server(port: int) -> None:
    HTTPServer(("0.0.0.0", port), HealthHandler).serve_forever()


def run_asset_loop(contract_id: str, network: str, source_identity: str,
                    asset_name: str, cfg: AssetConfig) -> None:
    while True:
        price = fetch_mid_price(cfg)
        if price is not None:
            scaled = int(price * 10**14)
            ok = submit_price(contract_id, network, source_identity, cfg.contract_address,
                               scaled, int(time.time()))
            if ok:
                METRICS["submissions_total"] += 1
                METRICS["last_submission_ts"][asset_name] = int(time.time())
                log.info("submitted %s = %s", asset_name, price)
            else:
                METRICS["submission_errors_total"] += 1
                log.error("submission failed for %s after retries", asset_name)
        else:
            log.error("no price data available for %s", asset_name)
        time.sleep(cfg.interval_secs)


def main() -> None:
    config_path = os.environ.get("BOT_CONFIG", "bot_config.toml")
    config = load_config(config_path)

    oracle = config["oracle"]
    source_identity = config["source"]["name"]
    health_port = int(config.get("health", {}).get("port", 9100))

    Thread(target=run_health_server, args=(health_port,), daemon=True).start()
    log.info("health/metrics endpoint listening on :%d", health_port)

    threads = []
    for asset_name, asset_cfg in config["assets"].items():
        cfg = AssetConfig(
            contract_address=asset_cfg["contract_address"],
            feed_ids=asset_cfg["feed_ids"],
            interval_secs=asset_cfg.get("interval_secs", config.get("schedule", {}).get("interval_secs", 60)),
        )
        t = Thread(target=run_asset_loop, args=(oracle["contract_id"], oracle["network"],
                                                  source_identity, asset_name, cfg), daemon=True)
        t.start()
        threads.append(t)

    for t in threads:
        t.join()


if __name__ == "__main__":
    main()
