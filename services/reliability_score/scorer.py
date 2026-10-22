"""Computes a per-source reliability score from indexed oracle events,
combining submission uptime, freshness, and accuracy vs. the aggregate —
the transparency mechanism underpinning source incentives (see the
on-chain stake/reputation system in contracts/price-oracle/src/reputation.rs).

The score is a pure function of the indexed `PriceSubmittedEvent` /
`PriceAggregatedEvent` stream: given the same events, `compute_scores`
always returns the same scores — "reproducible" in the sense the issue
asks for, and independently auditable by anyone re-running it against the
same indexed history.
"""
from __future__ import annotations

import statistics
from collections import defaultdict
from dataclasses import dataclass
from typing import Dict, Iterable, List, Optional, Set, Tuple

from services.common.events import AggregationEvent, SubmissionEvent

DEFAULT_UPTIME_WEIGHT = 0.30
DEFAULT_FRESHNESS_WEIGHT = 0.20
DEFAULT_ACCURACY_WEIGHT = 0.50

# A submission arriving this many seconds (or more) after the round's
# aggregation timestamp scores 0 on freshness; 0 lag scores 100, linear
# between.
DEFAULT_MAX_LAG_SECS = 300.0
# A submission deviating this many basis points (or more) from the round's
# aggregate price scores 0 on accuracy; mirrors the 50% cliff used by the
# on-chain reputation system in reputation.rs.
DEFAULT_MAX_DEVIATION_BPS = 5_000.0


@dataclass(frozen=True)
class ScoreWeights:
    uptime: float = DEFAULT_UPTIME_WEIGHT
    freshness: float = DEFAULT_FRESHNESS_WEIGHT
    accuracy: float = DEFAULT_ACCURACY_WEIGHT

    def __post_init__(self):
        total = self.uptime + self.freshness + self.accuracy
        if not 0.999 <= total <= 1.001:
            raise ValueError(f"weights must sum to 1.0, got {total}")


@dataclass(frozen=True)
class SourceReliabilityScore:
    asset: str
    source: str
    uptime_score: float
    freshness_score: float
    accuracy_score: float
    composite_score: float
    rounds_expected: int
    rounds_participated: int


def derive_aggregations(submissions: Iterable[SubmissionEvent]) -> List[AggregationEvent]:
    """Derives one `AggregationEvent` per (asset, ledger) round as the
    median of that round's submissions, for deployments that index raw
    submissions but not a separate aggregation event stream.

    Mirrors the on-chain aggregate: price is the round's median, timestamp
    is the most recent contributing submission's timestamp (see
    `PriceAggregatedEvent` in contracts/price-oracle/src/events.rs).
    """
    rounds: Dict[Tuple[str, int], List[SubmissionEvent]] = defaultdict(list)
    for sub in submissions:
        rounds[(sub.asset, sub.ledger)].append(sub)

    aggregations: List[AggregationEvent] = []
    for (asset, ledger), subs in rounds.items():
        prices = sorted(s.price for s in subs)
        median_price = statistics.median(prices)
        latest_ts = max(s.timestamp for s in subs)
        aggregations.append(
            AggregationEvent(
                ledger=ledger,
                timestamp=latest_ts,
                contract_id=subs[0].contract_id,
                asset=asset,
                price=int(median_price),
                num_sources=len(subs),
            )
        )
    return aggregations


def compute_scores(
    submissions: Iterable[SubmissionEvent],
    aggregations: Iterable[AggregationEvent],
    weights: ScoreWeights = ScoreWeights(),
    max_lag_secs: float = DEFAULT_MAX_LAG_SECS,
    max_deviation_bps: float = DEFAULT_MAX_DEVIATION_BPS,
) -> List[SourceReliabilityScore]:
    """Computes one `SourceReliabilityScore` per (asset, source) pair seen
    in `submissions`.

    * **Uptime** — the fraction of the asset's aggregated rounds the source
      actually submitted a price for.
    * **Freshness** — how promptly (relative to `max_lag_secs`) the source's
      submissions arrived ahead of/around the round's aggregation.
    * **Accuracy** — how close (relative to `max_deviation_bps`) the
      source's submitted prices were to each round's aggregate price.

    All three sub-scores and the weighted composite are in `[0, 100]`.
    """
    aggs_by_round: Dict[Tuple[str, int], AggregationEvent] = {}
    rounds_by_asset: Dict[str, Set[int]] = defaultdict(set)
    for agg in aggregations:
        aggs_by_round[(agg.asset, agg.ledger)] = agg
        rounds_by_asset[agg.asset].add(agg.ledger)

    subs_by_round_source: Dict[Tuple[str, int, str], SubmissionEvent] = {}
    sources_by_asset: Dict[str, Set[str]] = defaultdict(set)
    for sub in submissions:
        subs_by_round_source[(sub.asset, sub.ledger, sub.source)] = sub
        sources_by_asset[sub.asset].add(sub.source)

    scores: List[SourceReliabilityScore] = []
    for asset, expected_ledgers in rounds_by_asset.items():
        expected_ledgers_sorted = sorted(expected_ledgers)
        n_expected = len(expected_ledgers_sorted)
        if n_expected == 0:
            continue

        for source in sorted(sources_by_asset.get(asset, ())):
            participated = 0
            lag_scores: List[float] = []
            accuracy_scores: List[float] = []

            for ledger in expected_ledgers_sorted:
                sub = subs_by_round_source.get((asset, ledger, source))
                if sub is None:
                    continue
                participated += 1
                agg = aggs_by_round[(asset, ledger)]

                lag = max(0.0, agg.timestamp - sub.timestamp)
                lag_score = 100.0 if max_lag_secs <= 0 else max(0.0, 100.0 * (1.0 - lag / max_lag_secs))
                lag_scores.append(lag_score)

                if agg.price != 0:
                    deviation_bps = abs(sub.price - agg.price) / abs(agg.price) * 10_000
                else:
                    deviation_bps = 0.0 if sub.price == agg.price else max_deviation_bps
                accuracy_score = max(0.0, 100.0 * (1.0 - deviation_bps / max_deviation_bps))
                accuracy_scores.append(accuracy_score)

            uptime_score = 100.0 * participated / n_expected
            freshness_score = statistics.mean(lag_scores) if lag_scores else 0.0
            accuracy_score = statistics.mean(accuracy_scores) if accuracy_scores else 0.0
            composite = (
                weights.uptime * uptime_score
                + weights.freshness * freshness_score
                + weights.accuracy * accuracy_score
            )

            scores.append(
                SourceReliabilityScore(
                    asset=asset,
                    source=source,
                    uptime_score=uptime_score,
                    freshness_score=freshness_score,
                    accuracy_score=accuracy_score,
                    composite_score=composite,
                    rounds_expected=n_expected,
                    rounds_participated=participated,
                )
            )

    return scores


def get_source_score(
    scores: Iterable[SourceReliabilityScore], asset: str, source: str
) -> Optional[SourceReliabilityScore]:
    """Looks up a single (asset, source) score from a previously-computed
    `compute_scores` result — the off-chain equivalent of an on-chain
    `get_source_score(asset, source)` view (see docs/source-reliability-score.md)."""
    for score in scores:
        if score.asset == asset and score.source == source:
            return score
    return None
