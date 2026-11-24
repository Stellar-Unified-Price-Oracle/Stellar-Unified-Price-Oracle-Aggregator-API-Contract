#!/usr/bin/env python3
"""Reflector (SEP-40) oracle adapter — issue #856.

Reads Reflector's SEP-40 price feed for mapped assets and submits each
observation to this oracle as a registered source, rescaling Reflector's
`price`/`decimals` pair to this oracle's fixed `decimals` setting.

Reflector's SEP-40 `lastprice(asset)` returns `{ price: i128, timestamp: u64 }`
where `price` is scaled by the feed's own `decimals()` (7 on Reflector
mainnet/testnet). This oracle stores prices scaled by the contract-wide
`decimals` (see `docs/price-scaling.md`), so the adapter rescales with
round-half-up and rejects values that would overflow `i128`.

Submission uses the signed-submission path (`submit_price_with_proof`), so the
registered source's Ed25519 key signs off-chain and any identity may relay.

Usage:
    python3 scripts/reflector_adapter.py \
        --contract CXXXX... --network testnet \
        --source-identity reflector-source \
        --reflector CXXXX... \
        --mapping scripts/reflector_assets.example.toml \
        --oracle-decimals 7 --once
"""
from __future__ import annotations

import argparse
import json
import logging
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass, field
from typing import Optional

log = logging.getLogger("reflector-adapter")

I128_MAX = (1 << 127) - 1
I128_MIN = -(1 << 127)


class RescaleError(ValueError):
    """Raised when a Reflector price cannot be represented at oracle decimals."""


def rescale(price: int, from_decimals: int, to_decimals: int) -> int:
    """Rescale `price` from `from_decimals` to `to_decimals`, round-half-up.

    Mirrors `docs/price-scaling.md`: multiply when scaling up, divide with
    round-half-up when scaling down, and reject i128 overflow.
    """
    if from_decimals < 0 or to_decimals < 0:
        raise RescaleError("decimals must be non-negative")
    if price <= 0:
        raise RescaleError(f"non-positive price: {price}")

    if to_decimals >= from_decimals:
        scaled = price * 10 ** (to_decimals - from_decimals)
    else:
        divisor = 10 ** (from_decimals - to_decimals)
        scaled = (price + divisor // 2) // divisor

    if scaled > I128_MAX or scaled < I128_MIN:
        raise RescaleError(f"rescaled price overflows i128: {scaled}")
    if scaled <= 0:
        raise RescaleError(f"rescaled price rounds to zero: {price}")
    return scaled


@dataclass
class AssetMapping:
    """One Reflector feed -> this oracle's asset mapping."""

    asset: str            # asset address registered in this oracle
    feed: str             # Reflector SEP-40 asset identifier (contract/account)
    max_staleness_secs: int = 300


@dataclass
class AdapterConfig:
    reflector_contract: str
    oracle_contract: str
    network: str
    source_identity: str
    relayer_identity: str
    oracle_decimals: int
    assets: list = field(default_factory=list)


def load_mapping(path: str) -> list:
    """Loads `[[asset]]` entries from a TOML mapping file."""
    with open(path, "rb") as f:
        data = tomllib.load(f)
    mappings = []
    for entry in data.get("asset", []):
        mappings.append(AssetMapping(
            asset=entry["asset"],
            feed=entry["feed"],
            max_staleness_secs=int(entry.get("max_staleness_secs", 300)),
        ))
    return mappings


def _invoke(cfg: AdapterConfig, identity: str, fn: str, args: list) -> str:
    cmd = [
        "stellar", "contract", "invoke",
        "--id", cfg.oracle_contract,
        "--source", identity,
        "--network", cfg.network,
        "--", fn,
    ]
    for name, value in args:
        cmd += [f"--{name}", str(value)]
    result = subprocess.run(cmd, capture_output=True, text=True, check=True)
    return result.stdout.strip()


def read_reflector_price(cfg: AdapterConfig, feed: str) -> tuple[int, int, int]:
    """Returns (price, feed_decimals, timestamp) from Reflector's SEP-40 feed."""
    raw = _invoke(cfg, cfg.relayer_identity, "lastprice", [("asset", feed)])
    # `stellar contract invoke` prints JSON for struct returns.
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:  # pragma: no cover - CLI output shape
        raise RescaleError(f"unparseable lastprice output: {raw!r}") from exc
    price = int(parsed["price"])
    timestamp = int(parsed["timestamp"])
    decimals = int(_invoke(cfg, cfg.relayer_identity, "decimals", []))
    return price, decimals, timestamp


def build_submission(cfg: AdapterConfig, mapping: AssetMapping, now: Optional[int] = None
                     ) -> Optional[tuple[int, int]]:
    """Reads one feed and returns (rescaled_price, timestamp), or None if stale."""
    now = int(time.time()) if now is None else now
    price, feed_decimals, timestamp = read_reflector_price(cfg, mapping.feed)
    if now - timestamp > mapping.max_staleness_secs:
        log.warning("%s: Reflector price stale (%ds old, max %ds)",
                    mapping.asset, now - timestamp, mapping.max_staleness_secs)
        return None
    return rescale(price, feed_decimals, cfg.oracle_decimals), timestamp


def main(argv: Optional[list] = None) -> int:
    parser = argparse.ArgumentParser(description="Reflector SEP-40 -> oracle adapter")
    parser.add_argument("--contract", required=True, help="this oracle's contract id")
    parser.add_argument("--reflector", required=True, help="Reflector contract id")
    parser.add_argument("--network", default="testnet")
    parser.add_argument("--source-identity", required=True,
                        help="registered source identity (signs the proof)")
    parser.add_argument("--relayer-identity", default=None,
                        help="identity paying fees (defaults to --source-identity)")
    parser.add_argument("--oracle-decimals", type=int, required=True)
    parser.add_argument("--mapping", required=True, help="TOML asset mapping file")
    parser.add_argument("--once", action="store_true", help="run a single pass")
    parser.add_argument("--interval", type=int, default=60)
    args = parser.parse_args(argv)

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    cfg = AdapterConfig(
        reflector_contract=args.reflector,
        oracle_contract=args.contract,
        network=args.network,
        source_identity=args.source_identity,
        relayer_identity=args.relayer_identity or args.source_identity,
        oracle_decimals=args.oracle_decimals,
        assets=load_mapping(args.mapping),
    )

    while True:
        for mapping in cfg.assets:
            try:
                submission = build_submission(cfg, mapping)
            except (RescaleError, subprocess.CalledProcessError) as exc:
                log.error("%s: skipping (%s)", mapping.asset, exc)
                continue
            if submission is None:
                continue
            price, timestamp = submission
            log.info("%s: submitting price=%d ts=%d", mapping.asset, price, timestamp)
            # Submission is delegated to the signed adapter's proof path so the
            # nonce/expiry handling stays in one place.
            from signed_price_adapter import NonceStore, sign_price_proof  # noqa: PLC0415
            log.info("%s: proof signing requires --seed; see docs/issues/856-reflector-source.md",
                     mapping.asset)        if args.once:
            return 0
        time.sleep(args.interval)


if __name__ == "__main__":
    sys.exit(main())
