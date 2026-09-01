"""Synthetic submission-event generation, shared by the demo entrypoints and
test suites of the anomaly-detection, volatility-forecast, and
reliability-score services.

Produces canonical event envelopes (see `services.common.events`) so the
generated data can be fed through the exact same code path as a real
indexed event stream.
"""
from __future__ import annotations

import math
import random
from typing import Dict, Iterable, List, Optional, Set

from services.common.events import TOPIC_PRICE_AGGREGATED, TOPIC_PRICE_SUBMITTED

DEFAULT_CONTRACT_ID = "CDEMOCONTRACTIDXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX"


def generate_submissions(
    asset: str = "CDEMOASSETXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
    sources: Iterable[str] = ("SOURCE_A", "SOURCE_B", "SOURCE_C"),
    n_rounds: int = 200,
    base_price: int = 100_000_000,
    start_ts: int = 1_700_000_000,
    interval_secs: int = 60,
    noise_bps: float = 15.0,
    drift_bps_per_round: float = 0.0,
    seed: int = 1234,
    anomalies: Optional[Dict[int, Dict[str, float]]] = None,
    missing: Optional[Dict[str, Set[int]]] = None,
    contract_id: str = DEFAULT_CONTRACT_ID,
    ledger_start: int = 1,
) -> List[dict]:
    """Generates `n_rounds` of per-source submissions for a single asset.

    * `noise_bps` — stdev of normal per-source noise around the "true" price,
      in basis points.
    * `drift_bps_per_round` — slow deterministic drift applied to the true
      price each round, to give volatility/forecast tests a realistic trend.
    * `anomalies` — `{round_idx: {source: deviation_bps}}`. The named source's
      submission in that round is shifted by `deviation_bps` (signed) on top
      of its normal noise — this is the "seeded anomaly" consumed by the
      anomaly-detection tests.
    * `missing` — `{source: {round_idx, ...}}`. The named source submits
      nothing in those rounds, simulating downtime for uptime scoring.
    """
    rng = random.Random(seed)
    sources = list(sources)
    anomalies = anomalies or {}
    missing = missing or {}

    events: List[dict] = []
    true_price = float(base_price)
    ledger = ledger_start

    for round_idx in range(n_rounds):
        ts = start_ts + round_idx * interval_secs
        true_price *= 1.0 + (drift_bps_per_round / 10_000.0)

        for source in sources:
            if round_idx in missing.get(source, ()):
                continue

            noise = rng.gauss(0.0, noise_bps) / 10_000.0
            price = true_price * (1.0 + noise)

            deviation_bps = anomalies.get(round_idx, {}).get(source)
            if deviation_bps is not None:
                price = true_price * (1.0 + deviation_bps / 10_000.0)

            events.append(
                {
                    "ledger": ledger,
                    "timestamp": ts,
                    "contract_id": contract_id,
                    "topic": TOPIC_PRICE_SUBMITTED,
                    "data": {"asset": asset, "source": source, "price": int(round(price))},
                }
            )
        ledger += 1

    return events


def generate_price_series(
    asset: str = "CDEMOASSETXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
    n_points: int = 200,
    start_price: float = 100_000_000.0,
    interval_secs: int = 3600,
    start_ts: int = 1_700_000_000,
    period_volatility: float = 0.01,
    drift_per_period: float = 0.0,
    seed: int = 1234,
    contract_id: str = DEFAULT_CONTRACT_ID,
    ledger_start: int = 1,
) -> List[dict]:
    """Generates a geometric-Brownian-motion aggregate price series as
    `price_aggregated` event envelopes, for volatility-forecast and
    reliability-score tests/demos.

    `period_volatility` is the per-period (per `interval_secs`) log-return
    stdev — e.g. `0.01` means roughly 1% typical move between points.
    """
    rng = random.Random(seed)
    price = start_price
    events: List[dict] = []

    for i in range(n_points):
        if i > 0:
            shock = rng.gauss(drift_per_period, period_volatility)
            price *= math.exp(shock)
        events.append(
            {
                "ledger": ledger_start + i,
                "timestamp": start_ts + i * interval_secs,
                "contract_id": contract_id,
                "topic": TOPIC_PRICE_AGGREGATED,
                "data": {"asset": asset, "price": int(round(price)), "num_sources": 3},
            }
        )

    return events
