"""Off-chain price-revision frequency and correction-rate metrics (#499).

Correction frequency is one of the best leading indicators of source or
pipeline degradation — but only when it is measured continuously, attributed
to a cause, and normalized for volume, so that a rarely-updated asset cannot
read as "100 % corrected" after a single incident.

This service is a pure function of the indexed event stream
(``services.common.events``): given the same ``price_aggregated``,
``price_submitted`` and ``price_corrected`` events it always produces the same
reports, so anyone can re-derive the numbers from the same ledger history.

Design notes
------------

**Causes are separated.** Every revision is classified into exactly one
:class:`RevisionCause` bucket, so an operator-driven correction is never mixed
into the same rate as a source-driven or unattributable one
(:func:`classify`).

**Rates are volume-normalized.** The raw ratio ``revisions / publications`` is
unusable on a low-volume asset: one correction on an asset that publishes once
a day is a 100 % rate. The denominator is therefore floored at
``min_publications`` (:func:`normalized_rate`) and the report carries
``low_volume`` so a consumer can refuse to alert on it.

**The metric is observational and cannot be gamed by suppression.** It reads
only the on-chain event stream; it never suppresses a correction and never
writes to the chain. Because the revision chain is append-only and public, a
correction that is *never filed* cannot reduce the measured rate below the
chain's own length: :func:`reconcile` compares the observed corrections with
the on-chain ``get_price_revisions`` length and reports a gap, so hiding a
correction is visible rather than rewarded. See
``docs/price-revision-metrics.md``.
"""
from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from dataclasses import dataclass, field
from enum import Enum
from typing import Dict, Iterable, List, Mapping, Optional, Sequence

from services.common.events import (
    AggregationEvent,
    EventSource,
    RevisionEvent,
    SubmissionEvent,
    iter_aggregations,
    iter_revisions,
    iter_submissions,
)

#: Default rolling window for the "recent" rate, in seconds (24 h).
DEFAULT_WINDOW_SECS = 86_400
#: Default comparison ("previous") window, in seconds. Equal to the recent
#: window, so the trend compares like with like.
DEFAULT_BASELINE_WINDOW_SECS = 86_400
#: Publications required before a rate is considered meaningful. A rate whose
#: denominator is below this is reported with ``low_volume=True`` and is floored
#: at this many publications, so a single correction on a quiet asset reads as
#: 1/N rather than 100 %.
DEFAULT_MIN_PUBLICATIONS = 20
#: Revisions required in the recent window before an increase can alert.
DEFAULT_MIN_REVISIONS = 3
#: A sustained increase means the recent rate is at least this multiple of the
#: baseline rate.
DEFAULT_SUSTAINED_MULTIPLIER = 2.0
#: Absolute floor for the baseline rate used in the comparison, in the same
#: units as :func:`normalized_rate` (per-mille). Prevents a zero baseline from
#: making any single correction look like an infinite increase.
BASELINE_RATE_FLOOR_PERMILLE = 1.0


class RevisionCause(str, Enum):
    """Why a published price was revised.

    The split is the point of the metric: a correction an authorized operator
    filed is a *response* to a known problem, while a revision nobody can
    attribute is itself the data-quality incident.
    """

    #: Filed through the on-chain ``correct_price`` path by the admin or a
    #: ``PriceUpdater`` delegate. Expected, and the only cause that is not an
    #: incident on its own.
    AUTHORIZED_CORRECTION = "authorized_correction"
    #: Filed on behalf of a named oracle source — the source's own upstream
    #: value was wrong. Points at that source, not at the aggregation path.
    SOURCE_CORRECTION = "source_correction"
    #: Neither: no authorized actor and no attributable source. Deliberately
    #: counted, never dropped, so an unattributable revision cannot be made to
    #: disappear by declining to name a cause.
    UNATTRIBUTED = "unattributed"


def classify(event: RevisionEvent, authorized_actors: Iterable[str]) -> RevisionCause:
    """Returns the single cause bucket ``event`` belongs to.

    Precedence is deliberate: an authorized actor is the strongest signal and
    wins even when the pipeline also named a source, because the *filing* path
    is what the operator controls.
    """
    authorized = set(authorized_actors)
    if event.actor in authorized:
        return RevisionCause.AUTHORIZED_CORRECTION
    if event.source:
        return RevisionCause.SOURCE_CORRECTION
    return RevisionCause.UNATTRIBUTED


def normalized_rate(revisions: int, publications: int, min_publications: int) -> float:
    """Revisions per publication, in per-mille, with a floored denominator.

    The floor is the volume normalization: an asset with fewer than
    ``min_publications`` publications in the window is scored against
    ``min_publications`` anyway, so a single correction on a quiet asset cannot
    read as 1000‰ (100 %).
    """
    denominator = max(publications, min_publications)
    return 1000.0 * revisions / denominator


@dataclass(frozen=True)
class Thresholds:
    """Operator-tunable knobs for the revision-rate metric."""

    window_secs: int = DEFAULT_WINDOW_SECS
    baseline_window_secs: int = DEFAULT_BASELINE_WINDOW_SECS
    min_publications: int = DEFAULT_MIN_PUBLICATIONS
    min_revisions: int = DEFAULT_MIN_REVISIONS
    sustained_multiplier: float = DEFAULT_SUSTAINED_MULTIPLIER
    authorized_actors: tuple = ()

    def validate(self) -> List[str]:
        errors: List[str] = []
        if self.window_secs <= 0:
            errors.append("window_secs must be positive")
        if self.baseline_window_secs <= 0:
            errors.append("baseline_window_secs must be positive")
        if self.min_publications <= 0:
            errors.append("min_publications must be positive")
        if self.min_revisions <= 0:
            errors.append("min_revisions must be positive")
        if self.sustained_multiplier < 1.0:
            errors.append("sustained_multiplier must be >= 1.0")
        return errors


@dataclass(frozen=True)
class WindowStats:
    """Raw counts for one time window."""

    publications: int
    revisions: int
    by_cause: Mapping[RevisionCause, int]
    low_volume: bool

    @property
    def rate_permille(self) -> float:
        return 1000.0 * self.revisions / self.publications if self.publications else 0.0


@dataclass(frozen=True)
class AssetRevisionMetrics:
    """Revision frequency for one asset, split by cause."""

    asset: str
    lifetime: WindowStats
    recent: WindowStats
    baseline: WindowStats
    recent_rate_permille: float
    baseline_rate_permille: float
    trend_multiplier: float
    sustained_increase: bool
    unattributed: int
    downstream_affected: int

    @property
    def operator_corrections(self) -> int:
        return self.lifetime.by_cause.get(RevisionCause.AUTHORIZED_CORRECTION, 0)

    def to_dict(self) -> dict:
        return {
            "asset": self.asset,
            "lifetime": {
                "publications": self.lifetime.publications,
                "revisions": self.lifetime.revisions,
                "by_cause": {c.value: n for c, n in sorted(self.lifetime.by_cause.items())},
                "low_volume": self.lifetime.low_volume,
            },
            "recent": {
                "publications": self.recent.publications,
                "revisions": self.recent.revisions,
                "by_cause": {c.value: n for c, n in sorted(self.recent.by_cause.items())},
                "low_volume": self.recent.low_volume,
            },
            "baseline": {
                "publications": self.baseline.publications,
                "revisions": self.baseline.revisions,
            },
            "recent_rate_permille": round(self.recent_rate_permille, 3),
            "baseline_rate_permille": round(self.baseline_rate_permille, 3),
            "trend_multiplier": round(self.trend_multiplier, 3),
            "sustained_increase": self.sustained_increase,
            "unattributed": self.unattributed,
            "downstream_affected": self.downstream_affected,
        }


@dataclass(frozen=True)
class SourceRevisionMetrics:
    """Revision frequency attributable to one oracle source.

    The denominator is *that source's own submissions* over the same window, so
    a source is not penalised for an asset it does not cover.
    """

    source: str
    lifetime: WindowStats
    recent: WindowStats
    baseline: WindowStats
    recent_rate_permille: float
    baseline_rate_permille: float
    trend_multiplier: float
    sustained_increase: bool
    by_asset: Mapping[str, int] = field(default_factory=dict)

    def to_dict(self) -> dict:
        return {
            "source": self.source,
            "lifetime": {
                "submissions": self.lifetime.publications,
                "revisions": self.lifetime.revisions,
                "by_cause": {c.value: n for c, n in sorted(self.lifetime.by_cause.items())},
                "low_volume": self.lifetime.low_volume,
            },
            "recent": {
                "submissions": self.recent.publications,
                "revisions": self.recent.revisions,
                "low_volume": self.recent.low_volume,
            },
            "baseline": {
                "submissions": self.baseline.publications,
                "revisions": self.baseline.revisions,
            },
            "recent_rate_permille": round(self.recent_rate_permille, 3),
            "baseline_rate_permille": round(self.baseline_rate_permille, 3),
            "trend_multiplier": round(self.trend_multiplier, 3),
            "sustained_increase": self.sustained_increase,
            "by_asset": dict(self.by_asset),
        }


@dataclass(frozen=True)
class RevisionAlert:
    """A sustained increase in the correction rate, for one scope."""

    scope: str
    severity: str
    reason: str
    recent_rate_permille: float
    baseline_rate_permille: float
    trend_multiplier: float
    recent_revisions: int
    recent_publications: int
    low_volume: bool


def _stats(
    publications: int, causes: Sequence[RevisionCause], min_publications: int
) -> WindowStats:
    return WindowStats(
        publications=publications,
        revisions=len(causes),
        by_cause=Counter(causes),
        low_volume=publications < min_publications,
    )


def _trend_multiplier(recent: float, baseline: float) -> float:
    """Recent rate relative to the baseline rate, with a non-zero baseline.

    Without the floor a single correction against a zero baseline would be an
    "infinite" increase and every quiet asset would page.
    """
    return recent / max(baseline, BASELINE_RATE_FLOOR_PERMILLE)


def _sustained(
    recent: WindowStats, baseline: WindowStats, thresholds: Thresholds, multiplier: float
) -> bool:
    """A sustained increase, not a one-off. All three conditions are required:

    1. the recent window holds at least ``min_revisions`` revisions — one
       correction is an incident, not a trend;
    2. the recent window has at least ``min_publications`` publications — a
       "100 % correction rate" on two publications a week is an artefact of
       volume, not a degradation signal (this is the volume normalization
       applied to the *alert*, not only to the number); and
    3. the volume-normalized recent rate is at least ``sustained_multiplier``
       times the baseline rate.
    """
    if recent.revisions < thresholds.min_revisions:
        return False
    if recent.publications < thresholds.min_publications:
        return False
    return multiplier >= thresholds.sustained_multiplier


def compute_asset_metrics(
    asset: str,
    aggregations: Sequence[AggregationEvent],
    revisions: Sequence[RevisionEvent],
    thresholds: Thresholds,
    now: int,
) -> AssetRevisionMetrics:
    """Revision rate for one asset over rolling windows."""
    revs = sorted([r for r in revisions if r.asset == asset], key=lambda r: r.timestamp)
    pubs = sorted([a for a in aggregations if a.asset == asset], key=lambda a: a.timestamp)
    recent_from = now - thresholds.window_secs
    baseline_from = recent_from - thresholds.baseline_window_secs

    causes = [classify(r, thresholds.authorized_actors) for r in revs]
    lifetime = _stats(len(pubs), causes, thresholds.min_publications)

    recent_pubs = [a for a in pubs if a.timestamp >= recent_from]
    recent_causes = [c for r, c in zip(revs, causes) if r.timestamp >= recent_from]
    recent = _stats(len(recent_pubs), recent_causes, thresholds.min_publications)

    base_pubs = [a for a in pubs if baseline_from <= a.timestamp < recent_from]
    base_causes = [
        c for r, c in zip(revs, causes) if baseline_from <= r.timestamp < recent_from
    ]
    baseline = _stats(len(base_pubs), base_causes, thresholds.min_publications)

    recent_rate = normalized_rate(
        recent.revisions, recent.publications, thresholds.min_publications
    )
    baseline_rate = normalized_rate(
        baseline.revisions, baseline.publications, thresholds.min_publications
    )
    multiplier = _trend_multiplier(recent_rate, baseline_rate)

    return AssetRevisionMetrics(
        asset=asset,
        lifetime=lifetime,
        recent=recent,
        baseline=baseline,
        recent_rate_permille=recent_rate,
        baseline_rate_permille=baseline_rate,
        trend_multiplier=multiplier,
        sustained_increase=_sustained(recent, baseline, thresholds, multiplier),
        unattributed=sum(1 for c in causes if c is RevisionCause.UNATTRIBUTED),
        downstream_affected=sum(1 for r in revs if r.affects_downstream),
    )


def compute_source_metrics(
    source: str,
    submissions: Sequence[SubmissionEvent],
    revisions: Sequence[RevisionEvent],
    thresholds: Thresholds,
    now: int,
) -> SourceRevisionMetrics:
    """Revision rate for one source, normalized by that source's own volume."""
    revs = sorted([r for r in revisions if r.source == source], key=lambda r: r.timestamp)
    subs = sorted([s for s in submissions if s.source == source], key=lambda s: s.timestamp)
    recent_from = now - thresholds.window_secs
    baseline_from = recent_from - thresholds.baseline_window_secs

    causes = [classify(r, thresholds.authorized_actors) for r in revs]
    lifetime = _stats(len(subs), causes, thresholds.min_publications)

    recent_subs = [s for s in subs if s.timestamp >= recent_from]
    recent_causes = [c for r, c in zip(revs, causes) if r.timestamp >= recent_from]
    recent = _stats(len(recent_subs), recent_causes, thresholds.min_publications)

    base_subs = [s for s in subs if baseline_from <= s.timestamp < recent_from]
    base_causes = [
        c for r, c in zip(revs, causes) if baseline_from <= r.timestamp < recent_from
    ]
    baseline = _stats(len(base_subs), base_causes, thresholds.min_publications)

    recent_rate = normalized_rate(
        recent.revisions, recent.publications, thresholds.min_publications
    )
    baseline_rate = normalized_rate(
        baseline.revisions, baseline.publications, thresholds.min_publications
    )
    multiplier = _trend_multiplier(recent_rate, baseline_rate)

    by_asset: Counter = Counter()
    for r in revs:
        by_asset[r.asset] += 1

    return SourceRevisionMetrics(
        source=source,
        lifetime=lifetime,
        recent=recent,
        baseline=baseline,
        recent_rate_permille=recent_rate,
        baseline_rate_permille=baseline_rate,
        trend_multiplier=multiplier,
        sustained_increase=_sustained(recent, baseline, thresholds, multiplier),
        by_asset=dict(by_asset),
    )


def _rows(events: EventSource) -> List[dict]:
    if isinstance(events, (str, bytes)) or hasattr(events, "__fspath__"):
        with open(events, "r", encoding="utf-8") as handle:  # type: ignore[arg-type]
            return [json.loads(line) for line in handle if line.strip()]
    return list(events)  # type: ignore[arg-type]


def compute_metrics(
    events: EventSource,
    thresholds: Optional[Thresholds] = None,
    now: Optional[int] = None,
) -> Dict[str, object]:
    """Reads the event stream and returns the full metric bundle.

    ``now`` defaults to the newest event timestamp in the stream, so the report
    is a pure function of the events with no wall-clock dependence.
    """
    thresholds = thresholds or Thresholds()
    errors = thresholds.validate()
    if errors:
        raise ValueError("; ".join(errors))

    rows = _rows(events)
    if not rows:
        raise ValueError("event stream is empty")
    if now is None:
        now = max(int(r["timestamp"]) for r in rows)

    aggregations = list(iter_aggregations(rows))
    submissions = list(iter_submissions(rows))
    revisions = list(iter_revisions(rows))

    assets = sorted({a.asset for a in aggregations} | {r.asset for r in revisions})
    sources = sorted({s.source for s in submissions} | {r.source for r in revisions if r.source})

    return {
        "now": now,
        "thresholds": {
            "window_secs": thresholds.window_secs,
            "baseline_window_secs": thresholds.baseline_window_secs,
            "min_publications": thresholds.min_publications,
            "min_revisions": thresholds.min_revisions,
            "sustained_multiplier": thresholds.sustained_multiplier,
        },
        "assets": {
            asset: compute_asset_metrics(asset, aggregations, revisions, thresholds, now)
            for asset in assets
        },
        "sources": {
            source: compute_source_metrics(source, submissions, revisions, thresholds, now)
            for source in sources
        },
        "revisions_total": len(revisions),
        "publications_total": len(aggregations),
    }


def evaluate_alerts(bundle: Mapping[str, object]) -> List[RevisionAlert]:
    """Alerts on every scope whose correction rate rose sustainably.

    A non-zero count of unattributed revisions is itself an alert: a revision
    nobody can attribute to a cause or a source is the data-quality incident
    this metric exists to surface, so it can never be a quiet state.
    """
    thresholds: Mapping[str, object] = bundle["thresholds"]  # type: ignore[assignment]
    window = thresholds["window_secs"]
    alerts: List[RevisionAlert] = []

    for asset, m in bundle["assets"].items():  # type: ignore[union-attr]
        if m.unattributed:
            alerts.append(
                RevisionAlert(
                    scope=f"asset:{asset}",
                    severity="warning",
                    reason=(
                        f"{m.unattributed} revision(s) could not be attributed to an "
                        "authorized correction or to a named source"
                    ),
                    recent_rate_permille=m.recent_rate_permille,
                    baseline_rate_permille=m.baseline_rate_permille,
                    trend_multiplier=m.trend_multiplier,
                    recent_revisions=m.recent.revisions,
                    recent_publications=m.recent.publications,
                    low_volume=m.recent.low_volume,
                )
            )
        if m.sustained_increase:
            alerts.append(
                RevisionAlert(
                    scope=f"asset:{asset}",
                    severity="page",
                    reason=(
                        f"correction rate {m.recent_rate_permille:.1f} per-mille is "
                        f"{m.trend_multiplier:.1f}x the baseline "
                        f"{m.baseline_rate_permille:.1f} per-mille over {window}s"
                    ),
                    recent_rate_permille=m.recent_rate_permille,
                    baseline_rate_permille=m.baseline_rate_permille,
                    trend_multiplier=m.trend_multiplier,
                    recent_revisions=m.recent.revisions,
                    recent_publications=m.recent.publications,
                    low_volume=m.recent.low_volume,
                )
            )

    for source, m in bundle["sources"].items():  # type: ignore[union-attr]
        if m.sustained_increase:
            alerts.append(
                RevisionAlert(
                    scope=f"source:{source}",
                    severity="page",
                    reason=(
                        f"attributed correction rate {m.recent_rate_permille:.1f} per-mille "
                        f"is {m.trend_multiplier:.1f}x the baseline "
                        f"{m.baseline_rate_permille:.1f} per-mille"
                    ),
                    recent_rate_permille=m.recent_rate_permille,
                    baseline_rate_permille=m.baseline_rate_permille,
                    trend_multiplier=m.trend_multiplier,
                    recent_revisions=m.recent.revisions,
                    recent_publications=m.recent.publications,
                    low_volume=m.recent.low_volume,
                )
            )

    return alerts


def reconcile(
    bundle: Mapping[str, object], on_chain_revision_counts: Mapping[str, int]
) -> List[str]:
    """Compares observed corrections against the on-chain revision chains.

    The chain is append-only and public: ``get_price_revisions(asset)`` returns
    the whole chain and index 0 is the original publication. If the chain says
    an asset has N revisions and the event stream shows fewer, corrections are
    being hidden from the metric — precisely the suppression this function
    makes visible. Returns human-readable gaps (empty when the two agree).
    """
    assets: Mapping[str, AssetRevisionMetrics] = bundle["assets"]  # type: ignore[assignment]
    gaps: List[str] = []
    for asset, chain_len in sorted(on_chain_revision_counts.items()):
        on_chain_corrections = max(int(chain_len) - 1, 0)
        observed = assets.get(asset)
        observed_corrections = observed.lifetime.revisions if observed else 0
        if on_chain_corrections != observed_corrections:
            gaps.append(
                f"asset {asset}: on-chain revision chain holds {on_chain_corrections} "
                f"correction(s) but the event stream shows {observed_corrections}"
            )
    return gaps


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Price revision-rate metrics (#499)")
    parser.add_argument("events", help="JSONL export of the oracle event stream")
    parser.add_argument("--window-secs", type=int, default=DEFAULT_WINDOW_SECS)
    parser.add_argument("--min-publications", type=int, default=DEFAULT_MIN_PUBLICATIONS)
    parser.add_argument("--min-revisions", type=int, default=DEFAULT_MIN_REVISIONS)
    parser.add_argument(
        "--sustained-multiplier", type=float, default=DEFAULT_SUSTAINED_MULTIPLIER
    )
    parser.add_argument(
        "--authorized-actor",
        action="append",
        default=[],
        help="address allowed to file corrections (repeatable)",
    )
    parser.add_argument(
        "--on-chain-revisions",
        default=None,
        help='JSON map of asset -> revision-chain length, e.g. \'{"CXXX": 5}\'',
    )
    parser.add_argument("--fail-on-alert", action="store_true")
    args = parser.parse_args(argv)

    thresholds = Thresholds(
        window_secs=args.window_secs,
        baseline_window_secs=args.window_secs,
        min_publications=args.min_publications,
        min_revisions=args.min_revisions,
        sustained_multiplier=args.sustained_multiplier,
        authorized_actors=tuple(args.authorized_actor),
    )
    bundle = compute_metrics(args.events, thresholds=thresholds)
    alerts = evaluate_alerts(bundle)
    gaps = (
        reconcile(bundle, json.loads(args.on_chain_revisions))
        if args.on_chain_revisions
        else []
    )

    json.dump(
        {
            "assets": {a: m.to_dict() for a, m in bundle["assets"].items()},  # type: ignore[union-attr]
            "sources": {s: m.to_dict() for s, m in bundle["sources"].items()},  # type: ignore[union-attr]
            "alerts": [vars(a) for a in alerts],
            "reconciliation_gaps": gaps,
        },
        sys.stdout,
        indent=2,
        sort_keys=True,
    )
    sys.stdout.write("\n")
    return 1 if (alerts and args.fail_on_alert) or gaps else 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
