"""Off-chain statistical anomaly detection for oracle source submissions.

Flags a source's price submission as anomalous *before* it can influence
on-chain aggregation. The on-chain median already resists outliers
structurally, but this runs earlier (as submissions stream in) and gives
operators source + deviation context to act on misbehaving sources faster.

Primary detector: a **modified z-score** (median absolute deviation, MAD)
computed cross-sectionally across all sources reporting the same asset in
the same round (Iglewicz & Hoaglin, 1993) — robust to the very outliers it's
trying to find, unlike a mean/stdev z-score.

Secondary, optional detector: **Isolation Forest** (scikit-learn, if
installed) run per-source over its own recent price history, to catch
temporal anomalies (e.g. a single dominant source drifting) that a
cross-sectional check alone can miss. Entirely optional — the service is
fully functional without scikit-learn installed.
"""
from __future__ import annotations

import logging
import statistics
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from typing import Dict, Iterable, List, Optional, Tuple

from services.common.events import EventSource, SubmissionEvent, iter_submissions

log = logging.getLogger("anomaly-detector")

# Iglewicz & Hoaglin's recommended modified z-score threshold for outliers.
DEFAULT_MAD_THRESHOLD = 3.5
# Minimum peer submissions in a round before a statistical judgement is made;
# below this, deviation is noise, not signal.
DEFAULT_MIN_PEERS = 3
# Minimum absolute deviation from the round median, in basis points, before a
# high z-score is even considered. At small round sizes the MAD itself can
# collapse to near zero when peers happen to cluster tightly, which turns
# ordinary rounding noise into enormous (but economically meaningless)
# z-scores; gating on a minimum bps move keeps the detector focused on
# deviations an operator would actually care about.
DEFAULT_MIN_DEVIATION_BPS = 50.0
# 1 / Phi^-1(0.75): converts a MAD into a normal-equivalent stdev estimate.
MAD_SCALE = 1.4826


@dataclass
class AnomalyAlert:
    """An alert with enough source + deviation context for an operator to act on."""

    asset: str
    source: str
    price: int
    round_median: float
    modified_z_score: float
    deviation_bps: float
    timestamp: int
    ledger: int
    reason: str = "mad_outlier"


@dataclass
class DetectorConfig:
    mad_threshold: float = DEFAULT_MAD_THRESHOLD
    min_peers: int = DEFAULT_MIN_PEERS
    min_deviation_bps: float = DEFAULT_MIN_DEVIATION_BPS


def score_round(
    asset: str,
    ledger: int,
    submissions: List[SubmissionEvent],
    config: Optional[DetectorConfig] = None,
) -> List[AnomalyAlert]:
    """Scores one round's worth of same-asset, same-ledger submissions.

    A "round" is every source's submission for `asset` at ledger `ledger`.
    Returns one `AnomalyAlert` per submission whose modified z-score exceeds
    `config.mad_threshold`.
    """
    config = config or DetectorConfig()
    if len(submissions) < config.min_peers:
        return []

    prices = [s.price for s in submissions]
    median = statistics.median(prices)
    abs_devs = [abs(p - median) for p in prices]
    mad = statistics.median(abs_devs)

    alerts: List[AnomalyAlert] = []
    for sub, dev in zip(submissions, abs_devs):
        if mad == 0:
            # No spread among peers at all — any deviation is a hard outlier;
            # the modified z-score is undefined (division by zero) here.
            z = float("inf") if dev > 0 else 0.0
        else:
            z = (sub.price - median) / (MAD_SCALE * mad)

        deviation_bps = ((sub.price - median) / median * 10_000) if median else 0.0
        if abs(z) >= config.mad_threshold and abs(deviation_bps) >= config.min_deviation_bps:
            alerts.append(
                AnomalyAlert(
                    asset=asset,
                    source=sub.source,
                    price=sub.price,
                    round_median=median,
                    modified_z_score=z,
                    deviation_bps=deviation_bps,
                    timestamp=sub.timestamp,
                    ledger=ledger,
                )
            )
    return alerts


def detect_anomalies(
    events: Iterable[SubmissionEvent], config: Optional[DetectorConfig] = None
) -> List[AnomalyAlert]:
    """Batch entry point: scores every round found in `events`."""
    config = config or DetectorConfig()
    rounds: Dict[Tuple[str, int], List[SubmissionEvent]] = defaultdict(list)
    for event in events:
        rounds[(event.asset, event.ledger)].append(event)

    alerts: List[AnomalyAlert] = []
    for (asset, ledger), submissions in rounds.items():
        alerts.extend(score_round(asset, ledger, submissions, config))
    alerts.sort(key=lambda a: (a.ledger, a.asset, a.source))
    return alerts


# ─── Alert delivery ─────────────────────────────────────────────────────────


class AlertSink:
    def emit(self, alert: AnomalyAlert) -> None:
        raise NotImplementedError


class LoggingAlertSink(AlertSink):
    def emit(self, alert: AnomalyAlert) -> None:
        log.warning(
            "anomalous submission: asset=%s source=%s price=%s median=%s z=%.2f dev_bps=%.1f ledger=%s",
            alert.asset,
            alert.source,
            alert.price,
            alert.round_median,
            alert.modified_z_score,
            alert.deviation_bps,
            alert.ledger,
        )


class WebhookAlertSink(AlertSink):
    """Posts each alert as JSON to a webhook URL (e.g. an alerting gateway).

    Delivery failures are logged, never raised — a flaky webhook must not
    take down the detection pipeline.
    """

    def __init__(self, url: str, timeout_secs: float = 5.0):
        self.url = url
        self.timeout_secs = timeout_secs

    def emit(self, alert: AnomalyAlert) -> None:
        import requests

        try:
            requests.post(self.url, json=asdict(alert), timeout=self.timeout_secs)
        except requests.RequestException:
            log.exception("failed to deliver anomaly webhook to %s", self.url)


# ─── Streaming pipeline ─────────────────────────────────────────────────────


@dataclass
class StreamingDetector:
    """Buffers submissions per-asset by ledger round and scores each round as
    soon as it closes (i.e. once a submission for a later ledger of the same
    asset arrives), forwarding alerts to `sinks` as they're found.

    This is the "analysis pipeline" consuming a stream of submission events:
    feed it events in roughly ledger order (as they arrive off an indexer or
    a JSONL tail) via `feed`, or drain a finite/batch source via `run`.
    """

    config: DetectorConfig = field(default_factory=DetectorConfig)
    sinks: List[AlertSink] = field(default_factory=lambda: [LoggingAlertSink()])
    alerts_emitted: int = 0
    rounds_scored: int = 0
    _buffer: Dict[str, Tuple[int, List[SubmissionEvent]]] = field(default_factory=dict)

    def feed(self, event: SubmissionEvent) -> List[AnomalyAlert]:
        flushed: List[AnomalyAlert] = []
        bucket = self._buffer.get(event.asset)
        if bucket is not None and bucket[0] != event.ledger:
            flushed = self._flush(event.asset, *bucket)
            bucket = None
        if bucket is None:
            self._buffer[event.asset] = (event.ledger, [event])
        else:
            bucket[1].append(event)
        return flushed

    def flush_all(self) -> List[AnomalyAlert]:
        """Scores every buffered (possibly still-open) round. Call once the
        underlying stream is exhausted so the final round isn't lost."""
        alerts: List[AnomalyAlert] = []
        for asset in list(self._buffer.keys()):
            ledger, submissions = self._buffer.pop(asset)
            alerts.extend(self._flush(asset, ledger, submissions))
        return alerts

    def run(self, source: EventSource) -> List[AnomalyAlert]:
        alerts: List[AnomalyAlert] = []
        for event in iter_submissions(source):
            alerts.extend(self.feed(event))
        alerts.extend(self.flush_all())
        return alerts

    def _flush(self, asset: str, ledger: int, submissions: List[SubmissionEvent]) -> List[AnomalyAlert]:
        alerts = score_round(asset, ledger, submissions, self.config)
        self.rounds_scored += 1
        for alert in alerts:
            self.alerts_emitted += 1
            for sink in self.sinks:
                sink.emit(alert)
        return alerts


# ─── Optional secondary detector: Isolation Forest ─────────────────────────


def isolation_forest_outliers(
    price_history: List[float], contamination: float = 0.05
) -> Optional[List[bool]]:
    """Flags temporal outliers within a single source's own price history
    using scikit-learn's Isolation Forest, if it's installed.

    Returns `None` (never raises) when scikit-learn is unavailable — this
    detector is a purely optional complement to the MAD-based round scoring
    above, which is what the service relies on by default.
    """
    if len(price_history) < 10:
        return None
    try:
        from sklearn.ensemble import IsolationForest
    except ImportError:
        log.debug("scikit-learn not installed; skipping isolation-forest pass")
        return None

    values = [[p] for p in price_history]
    model = IsolationForest(contamination=contamination, random_state=0)
    predictions = model.fit_predict(values)
    return [p == -1 for p in predictions]
