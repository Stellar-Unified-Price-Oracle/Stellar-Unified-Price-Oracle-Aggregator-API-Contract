#!/usr/bin/env python3
"""Signed off-chain adapter for centralized-exchange / aggregator price feeds.

Fetches prices for configured assets from first-party CEX/aggregator
connectors (CoinGecko, Binance, Coinbase), with per-asset provider failover
and staleness handling, then submits each observation through the existing
signed-submission path (#216: `submit_price_with_proof`) instead of a
plain `require_auth` transaction — so the registered source's Ed25519 key
signs the price off-chain and *any* address may relay the resulting
transaction on-chain.

See docs/signed-price-adapters.md for the full design, the provider
failover/staleness policy, and one-time source setup (registering the
Ed25519 submission key via `register_submission_key`).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import subprocess
import time
import tomllib
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, HTTPServer
from threading import Lock, Thread
from typing import Optional

import requests
from nacl.signing import SigningKey

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("signed-price-adapter")

METRICS = {
    "submissions_total": 0,
    "submission_errors_total": 0,
    "fetch_errors_total": {"coingecko": 0, "binance": 0, "coinbase": 0},
    "stale_skips_total": {"coingecko": 0, "binance": 0, "coinbase": 0},
    "failover_events_total": 0,
    "no_price_available_total": 0,
    "last_submission_ts": {},
    "last_provider_used": {},
}

# Each fetcher returns (price, observed_at_unix_ts | None). `observed_at`
# is the provider's own reported update time when available (CoinGecko);
# providers that don't expose one return None and the caller treats the
# request time as the observation time.
PROVIDER_FETCHERS = {
    "coingecko": lambda feed_id: _fetch_coingecko(feed_id),
    "binance": lambda feed_id: (float(requests.get(
        "https://api.binance.com/api/v3/ticker/price",
        params={"symbol": feed_id}, timeout=10,
    ).json()["price"]), None),
    "coinbase": lambda feed_id: (float(requests.get(
        f"https://api.coinbase.com/v2/prices/{feed_id}/spot", timeout=10,
    ).json()["data"]["amount"]), None),
}


def _fetch_coingecko(feed_id: str) -> tuple[float, Optional[int]]:
    resp = requests.get(
        "https://api.coingecko.com/api/v3/simple/price",
        params={"ids": feed_id, "vs_currencies": "usd", "include_last_updated_at": "true"},
        timeout=10,
    ).json()
    entry = resp[feed_id]
    return float(entry["usd"]), entry.get("last_updated_at")


@dataclass
class AssetConfig:
    contract_address: str
    providers: list = field(default_factory=list)  # priority-ordered
    feed_ids: dict = field(default_factory=dict)  # provider -> feed id
    interval_secs: int = 30


class NonceStore:
    """Persists the last accepted nonce per source across restarts.

    `submit_price_with_proof` requires a strictly increasing nonce; losing
    track of it across a restart (or racing two adapter instances for the
    same source) would produce rejected submissions, so state is flushed to
    disk after every increment.
    """

    def __init__(self, path: str):
        self._path = path
        self._lock = Lock()
        self._nonces: dict[str, int] = {}
        if os.path.exists(path):
            with open(path) as f:
                self._nonces = json.load(f)

    def next(self, source_address: str) -> int:
        with self._lock:
            nonce = self._nonces.get(source_address, 0) + 1
            self._nonces[source_address] = nonce
            with open(self._path, "w") as f:
                json.dump(self._nonces, f)
            return nonce


def sign_price_proof(seed_hex: str, nonce: int, price: int, timestamp: int,
                      expiration_ledger: int) -> tuple[str, str]:
    """Reproduces contracts/price-oracle/src/signed_submission.rs's digest and
    signs it, returning (public_key_hex, signature_hex).

    Digest: sha256(b"price_proof_v1" || nonce_le(8) || price_le(16, as u128)
                   || timestamp_le(8) || expiration_ledger_le(4))
    """
    signing_key = SigningKey(bytes.fromhex(seed_hex))
    buf = (
        b"price_proof_v1"
        + nonce.to_bytes(8, "little")
        + price.to_bytes(16, "little", signed=False)
        + timestamp.to_bytes(8, "little")
        + expiration_ledger.to_bytes(4, "little")
    )
    digest = hashlib.sha256(buf).digest()
    signature = signing_key.sign(digest).signature
    return signing_key.verify_key.encode().hex(), signature.hex()


def fetch_with_failover(asset_name: str, asset: AssetConfig, max_staleness_secs: int
                         ) -> Optional[tuple[float, str]]:
    """Tries each configured provider in priority order, skipping one that
    errors or reports a price older than `max_staleness_secs`. Returns the
    first usable (price, provider_name), or None if every provider failed.
    """
    now = int(time.time())
    for i, provider in enumerate(asset.providers):
        feed_id = asset.feed_ids.get(provider)
        fetcher = PROVIDER_FETCHERS.get(provider)
        if not feed_id or not fetcher:
            continue
        try:
            price, observed_at = fetcher(feed_id)
        except Exception as exc:  # noqa: BLE001 - one provider's outage must not block failover
            METRICS["fetch_errors_total"][provider] += 1
            log.warning("%s: fetch failed from %s: %s", asset_name, provider, exc)
            continue

        if observed_at is not None and now - observed_at > max_staleness_secs:
            METRICS["stale_skips_total"][provider] += 1
            log.warning("%s: %s price is stale (%ds old, max %ds) — trying next provider",
                        asset_name, provider, now - observed_at, max_staleness_secs)
            continue

        if i > 0:
            METRICS["failover_events_total"] += 1
            log.info("%s: failed over to provider %s", asset_name, provider)
        return price, provider

    return None


def submit_price_with_proof(contract_id: str, network: str, relayer_identity: str,
                             source_address: str, asset_address: str, price: int,
                             timestamp: int, nonce: int, expiration_ledger: int,
                             signature_hex: str, retries: int = 3) -> bool:
    """Relays a pre-signed price proof on-chain. `relayer_identity` only pays
    the transaction fee — the signed proof carries the source's authority
    (verified on-chain against the key registered via `register_submission_key`),
    so the relaying identity need not be the registered source itself.
    """
    for attempt in range(1, retries + 1):
        try:
            subprocess.run(
                [
                    "stellar", "contract", "invoke",
                    "--id", contract_id, "--source", relayer_identity, "--network", network,
                    "--", "submit_price_with_proof",
                    "--source", source_address,
                    "--asset", asset_address,
                    "--price", str(price),
                    "--timestamp", str(timestamp),
                    "--nonce", str(nonce),
                    "--expiration_ledger", str(expiration_ledger),
                    "--signature", signature_hex,
                ],
                check=True, capture_output=True, timeout=30,
            )
            return True
        except Exception as exc:  # noqa: BLE001 - retry regardless of failure kind
            backoff = 2 ** attempt
            log.warning("submit attempt %d/%d failed: %s (retrying in %ds)", attempt, retries, exc, backoff)
            time.sleep(backoff)
    return False


def get_latest_ledger(rpc_url: str) -> int:
    from stellar_sdk import SorobanServer
    with SorobanServer(rpc_url) as server:
        return server.get_latest_ledger().sequence


def register_submission_key(contract_id: str, network: str, source_identity: str, seed_hex: str) -> None:
    """One-time setup: registers the adapter's Ed25519 public key on-chain for
    `source_identity` (a normal Soroban-authorized transaction, unrelated to
    the per-submission proof signature)."""
    signing_key = SigningKey(bytes.fromhex(seed_hex))
    public_key_hex = signing_key.verify_key.encode().hex()
    subprocess.run(
        [
            "stellar", "contract", "invoke",
            "--id", contract_id, "--source", source_identity, "--network", network,
            "--", "register_submission_key",
            "--source", source_identity,
            "--public_key", public_key_hex,
        ],
        check=True,
    )
    log.info("registered submission key %s for source %s", public_key_hex, source_identity)


class HealthHandler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 - required by BaseHTTPRequestHandler
        if self.path == "/health":
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


def run_asset_loop(contract_id: str, network: str, relayer_identity: str, source_address: str,
                    seed_hex: str, rpc_url: str, expiration_window_ledgers: int,
                    max_staleness_secs: int, decimals: int, nonces: NonceStore,
                    asset_name: str, asset: AssetConfig) -> None:
    while True:
        result = fetch_with_failover(asset_name, asset, max_staleness_secs)
        if result is None:
            METRICS["no_price_available_total"] += 1
            log.error("%s: no provider returned a usable price this cycle", asset_name)
            time.sleep(asset.interval_secs)
            continue

        price, provider = result
        METRICS["last_provider_used"][asset_name] = provider
        scaled_price = int(price * 10**decimals)
        timestamp = int(time.time())
        nonce = nonces.next(source_address)

        try:
            expiration_ledger = get_latest_ledger(rpc_url) + expiration_window_ledgers
        except Exception as exc:  # noqa: BLE001 - RPC hiccup shouldn't crash the loop
            log.error("%s: failed to fetch latest ledger: %s", asset_name, exc)
            time.sleep(asset.interval_secs)
            continue

        _, signature_hex = sign_price_proof(
            seed_hex, nonce, scaled_price, timestamp, expiration_ledger,
        )

        ok = submit_price_with_proof(
            contract_id, network, relayer_identity, source_address, asset.contract_address,
            scaled_price, timestamp, nonce, expiration_ledger, signature_hex,
        )
        if ok:
            METRICS["submissions_total"] += 1
            METRICS["last_submission_ts"][asset_name] = timestamp
            log.info("%s: submitted %s (via %s, nonce=%d)", asset_name, price, provider, nonce)
        else:
            METRICS["submission_errors_total"] += 1
            log.error("%s: submission failed after retries", asset_name)

        time.sleep(asset.interval_secs)


def load_config(path: str) -> dict:
    with open(path, "rb") as f:
        return tomllib.load(f)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", default=os.environ.get("ADAPTER_CONFIG", "signed_adapter_config.toml"))
    parser.add_argument("--register-key", action="store_true",
                         help="Register the source's Ed25519 submission key on-chain and exit.")
    args = parser.parse_args()

    config = load_config(args.config)
    oracle = config["oracle"]
    source = config["source"]
    seed_hex = os.environ[source["ed25519_seed_env"]]

    if args.register_key:
        register_submission_key(oracle["contract_id"], oracle["network"], source["identity"], seed_hex)
        return

    nonces = NonceStore(config.get("state", {}).get("nonce_file", "adapter_nonces.json"))
    health_port = int(config.get("health", {}).get("port", 9101))
    schedule = config.get("schedule", {})
    max_staleness_secs = int(schedule.get("max_staleness_secs", 120))
    expiration_window_ledgers = int(schedule.get("expiration_window_ledgers", 100))
    decimals = int(oracle.get("decimals", 14))

    Thread(target=run_health_server, args=(health_port,), daemon=True).start()
    log.info("health endpoint listening on :%d", health_port)

    threads = []
    for asset_name, asset_cfg in config["assets"].items():
        asset = AssetConfig(
            contract_address=asset_cfg["contract_address"],
            providers=asset_cfg["providers"],
            feed_ids=asset_cfg["feed_ids"],
            interval_secs=asset_cfg.get("interval_secs", schedule.get("interval_secs", 30)),
        )
        t = Thread(
            target=run_asset_loop,
            args=(oracle["contract_id"], oracle["network"], source["relayer_identity"],
                  source["address"], seed_hex, oracle["rpc_url"], expiration_window_ledgers,
                  max_staleness_secs, decimals, nonces, asset_name, asset),
            daemon=True,
        )
        t.start()
        threads.append(t)

    for t in threads:
        t.join()


if __name__ == "__main__":
    main()
