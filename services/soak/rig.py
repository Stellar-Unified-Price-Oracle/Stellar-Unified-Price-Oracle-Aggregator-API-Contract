"""Sustained soak / load rig for the oracle ingest path (#523).

Headline load tests measure a burst. This rig measures the *slog*: a long,
seeded run of a realistic-plus-adversarial submission mix whose latency,
memory and state growth are tracked per round and asserted against ceilings.
A deliberately leaky state model ships alongside the real one so the
"unbounded growth is detected" claim is demonstrated, not just asserted.

See ``docs/soak-rig.md`` for the workload mix, the distributions, the ceilings
and the scheduled run.
"""
from __future__ import annotations

import argparse
import json
import math
import random
import statistics
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence, Set, Tuple

# --------------------------------------------------------------------------
# Workload mix (#523: "realistic + adversarial", adversarial must not dominate)
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class MixEntry:
    """One class of submission in the sustained mix.

    ``weight`` is the relative frequency; ``weight`` sums to 1 across the whole
    mix. ``p99_bps`` is how far above the reference price this class trades —
    the adversarial classes are the plausible-but-wrong ones a rogue source
    actually sends, not noise of arbitrary size.
    """

    name: str
    weight: float
    p99_bps: float
    #: Fraction of the weight that arrives as a same-ledger burst (thundering
    #: herd) rather than spread across the round.
    burst_fraction: float = 0.0
    #: Fraction of the weight that is a duplicate/stale resubmission of a
    #: value already counted this round.
    duplicate_fraction: float = 0.0
    #: Extra state bytes retained per submission (history entry size).
    state_bytes: int = 0


#: The documented production-like mix. Realistic traffic is the bulk; the
#: adversarial classes together stay well under a third of the weight so the
#: run keeps resembling production (``docs/soak-rig.md`` §2).
DEFAULT_MIX: Tuple[MixEntry, ...] = (
    MixEntry("routine", weight=0.62, p99_bps=35.0),
    MixEntry("diurnal_burst", weight=0.16, p99_bps=60.0, burst_fraction=0.5),
    MixEntry("slow_drift", weight=0.08, p99_bps=400.0, burst_fraction=0.0),
    MixEntry("price_gap", weight=0.05, p99_bps=2_500.0, burst_fraction=0.0),
    MixEntry("stale_resubmit", weight=0.04, p99_bps=50.0, duplicate_fraction=0.7),
    MixEntry("thundering_herd", weight=0.03, p99_bps=80.0, burst_fraction=1.0),
    MixEntry("flash_spike", weight=0.02, p99_bps=9_000.0, burst_fraction=0.0),
)


def mix_weight(mix: Sequence[MixEntry], name: str) -> float:
    return sum(e.weight for e in mix if e.name == name)


def adversarial_fraction(mix: Sequence[MixEntry]) -> float:
    """Weight not attributable to the ``routine``/``diurnal_burst`` classes."""
    realistic = {"routine", "diurnal_burst"}
    return sum(e.weight for e in mix if e.name not in realistic)


# --------------------------------------------------------------------------
# State models — the thing whose growth the rig bounds
# --------------------------------------------------------------------------


class StateModel:
    """Per-(asset, source) submission state with a bounded retention window.

    The realistic model keeps only the most recent ``window`` submissions per
    key, exactly as the contract prunes history, so state is O(sources).
    """

    #: Name reported in the rig output.
    name = "bounded"
    leaky = False

    def __init__(self, window: int, state_bytes: int = 0) -> None:
        self.window = window
        self.state_bytes = state_bytes
        self._entries: Dict[Tuple[str, str], List[int]] = {}
        self.bytes_retained = 0

    def apply(self, asset: str, source: str, price: int) -> None:
        key = (asset, source)
        buf = self._entries.get(key)
        if buf is None:
            buf = self._entries[key] = []
        buf.append(price)
        if len(buf) > self.window:
            # Pruned: the entry is overwritten, so retained bytes are flat.
            del buf[: len(buf) - self.window]
        self.bytes_retained = sum(
            self.state_bytes for k in self._entries for _ in self._entries[k]
        )


class LeakyStateModel(StateModel):
    """Deliberately leaky variant: every submission is retained forever.

    Used by the rig's own regression test (and by ``--model leaky``) to
    demonstrate that unbounded retention is *detected* rather than passing a
    soak unnoticed.
    """

    name = "leaky"
    leaky = True

    def apply(self, asset: str, source: str, price: int) -> None:
        self._entries.setdefault((asset, source), []).append(price)
        # No pruning: retained bytes grow linearly with the submission count.
        self.bytes_retained = sum(
            self.state_bytes for k in self._entries for _ in self._entries[k]
        )


# --------------------------------------------------------------------------
# Workload generation
# --------------------------------------------------------------------------


@dataclass
class Submission:
    round_idx: int
    asset: str
    source: str
    price: int
    kind: str
    duplicate: bool = False
    burst: bool = False


def _logistic(rng: random.Random, p99: float) -> float:
    """Draw a deviation in bps whose 99th percentile is ``p99``.

    A logistic body is used because oracle deviation is empirically heavy
    tailed: the common case is a few bps, the tail is a large plausible move.
    ``scale`` is chosen so the logistic 99th percentile lands on ``p99``.
    """
    scale = p99 / 4.59512  # logistic(0, s): 99th percentile = s * ln(99)
    return scale / 2.0 * _logit(rng.random())


def _logit(u: float) -> float:
    u = min(max(u, 1e-6), 1 - 1e-6)
    return math.log(u / (1.0 - u))


def _cumulative(mix: Sequence[MixEntry]) -> List[Tuple[float, MixEntry]]:
    """Normalises ``mix`` weights to 1 and returns ``(upper_bound, entry)``."""
    total = sum(e.weight for e in mix)
    if total <= 0:
        raise ValueError("mix weights must sum to a positive value")
    acc, out = 0.0, []
    for e in mix:
        acc += e.weight
        out.append((acc / total, e))
    return out


def build_workload(cfg: "SoakConfig", rng: random.Random) -> Iterable[Submission]:
    """Yields the sustained submission stream, round by round.

    Each round emits ``cfg.sources`` base submissions, each drawn from the
    documented mix, plus any same-ledger burst for the classes that specify
    one. The price path follows ``cfg.drift_bps`` cyclically, so the run is
    deterministic and reproducible from ``cfg.seed`` alone.
    """
    cumulative = _cumulative(cfg.mix)
    prices = [round(cfg.reference_price * (1 + d / 10_000.0)) for d in cfg.drift_bps]

    for rnd in range(cfg.rounds):
        price = prices[rnd % len(prices)]
        for s in range(cfg.sources):
            # Sources report across assets, so every (asset, source) pair is
            # exercised: the retained set is the full sources x assets grid.
            asset = f"ASSET{(s + rnd) % cfg.assets}"
            source = f"SRC{s:02d}"
            kind, dev_bps, burst, dup = _draw(cumulative, rng)
            value = round(price * (1 + dev_bps / 10_000.0))
            yield Submission(rnd, asset, source, value, kind, duplicate=dup, burst=burst)
            if burst and kind in ("thundering_herd", "diurnal_burst"):
                # Same-ledger herd: part of the source set follows at once.
                for t in range(1, max(1, cfg.sources // 3)):
                    k2, d2, _, _ = _draw(cumulative, rng)
                    v2 = round(price * (1 + d2 / 10_000.0))
                    yield Submission(
                        rnd, asset, f"SRC{(s + t) % cfg.sources:02d}", v2, k2, burst=True
                    )
            if dup:
                yield Submission(rnd, asset, source, value, kind, duplicate=True, burst=burst)


def _draw(
    cumulative: Sequence[Tuple[float, MixEntry]], rng: random.Random
) -> Tuple[str, float, bool, bool]:
    u = rng.random()
    entry = cumulative[-1][1]
    for bound, e in cumulative:
        if u <= bound:
            entry = e
            break
    dev_bps = _logistic(rng, entry.p99_bps) if entry.p99_bps else 0.0
    burst = rng.random() < entry.burst_fraction
    dup = rng.random() < entry.duplicate_fraction
    return entry.name, dev_bps, burst, dup


# --------------------------------------------------------------------------
# Latency model
# --------------------------------------------------------------------------


def round_latency(
    state: StateModel, distinct_sources: int, submissions: int = 0
) -> float:
    """Cost of one aggregation round, in the rig's latency units (ms).

    Three terms, all of which the soak must keep flat over a long run:

    * a sort over the counted submissions, O(n log n) in the number of
      *counted* sources;
    * a term proportional to every submission the round had to ingest,
      including the duplicates and same-ledger herd members that are not
      counted but still cost work (this is what the adversarial classes move);
    * a term proportional to the state the round has to touch, so a leaking
      state model is *also* slower in the tail — the coupling the soak is
      meant to catch.
    """
    n = max(distinct_sources, 1)
    sort_term = n * math.log2(n) if n > 1 else 1.0
    ingest_term = 0.06 * max(submissions, n)
    state_term = state.bytes_retained / 4_096.0
    return 2.0 + 0.45 * sort_term + ingest_term + 0.02 * state_term


def percentile(samples: Sequence[float], p: float) -> float:
    """Nearest-rank percentile; ``p`` in [0, 100]."""
    if not samples:
        return 0.0
    ordered = sorted(samples)
    rank = max(1, math.ceil(p / 100.0 * len(ordered)))
    return ordered[min(rank, len(ordered)) - 1]


# --------------------------------------------------------------------------
# Configuration, thresholds, result
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class Thresholds:
    """Ceilings the run is asserted against (``docs/soak-rig.md`` §4)."""

    #: p99 round latency, ms.
    max_p99_latency_ms: float = 25.0
    #: p99/p50 ratio — catches tail growth a flat p99 could hide.
    max_tail_ratio: float = 4.0
    #: Allowed relative drift of median round latency, last tenth vs first.
    max_latency_drift: float = 0.25
    #: State ceiling, bytes. Host executions have a far higher ceiling than
    #: the on-chain ledger budget, so both are configurable.
    max_state_bytes: int = 8 * 1024 * 1024
    #: State growth per 1k submissions, bytes. Non-zero means the retained
    #: set is not bounded by the pruning window.
    max_state_growth_per_1k: float = 0.0
    #: Rounds discarded before the growth slope is fitted, so the bounded
    #: model's fill-up phase is not mistaken for growth.
    growth_warmup_rounds: int = 200
    #: p99 of the process's sampled memory (tracemalloc), bytes.
    max_memory_bytes: int = 512 * 1024 * 1024


@dataclass
class SoakConfig:
    rounds: int = 2_000
    sources: int = 7
    assets: int = 3
    reference_price: int = 100_000_000
    #: Cyclic drift schedule, bps, applied to the reference price per round.
    drift_bps: Tuple[int, ...] = (0, 3, -5, 12, -18, 40, -25, 8)
    seed: int = 20260928
    mix: Tuple[MixEntry, ...] = DEFAULT_MIX
    #: History entries retained per (asset, source); the pruning window.
    history_window: int = 8
    #: Bytes one retained history entry occupies (price + timestamp + source
    #: key, rounded up to the ledger's storage granularity).
    entry_bytes: int = 64
    #: Use the deliberately leaky state model (demonstrates detection).
    leaky: bool = False
    thresholds: Thresholds = field(default_factory=Thresholds)


@dataclass
class SoakResult:
    config: SoakConfig
    rounds: int
    submissions: int
    latencies: List[float]
    state_samples: List[int]
    #: Cumulative submissions at the moment each state sample was taken, so
    #: the growth slope is per submission rather than per round.
    submission_marks: List[int]
    memory_samples: List[int]
    kind_counts: Dict[str, int]
    breaches: List[str] = field(default_factory=list)

    @property
    def state_model(self) -> str:
        return "leaky" if self.config.leaky else "bounded"

    def latency_percentiles(self) -> Dict[str, float]:
        return {
            "p50": percentile(self.latencies, 50),
            "p95": percentile(self.latencies, 95),
            "p99": percentile(self.latencies, 99),
            "max": max(self.latencies) if self.latencies else 0.0,
        }

    def memory_peak(self) -> int:
        return max(self.memory_samples) if self.memory_samples else 0

    def state_growth_per_1k(self) -> float:
        """Least-squares slope of state bytes vs submissions, per 1k.

        Fitted only after ``growth_warmup_rounds``: a bounded model fills its
        pruning window during the first rounds, and that ramp is not growth.
        """
        t = self.config.thresholds
        pairs = list(zip(self.submission_marks, self.state_samples))
        warmup = min(t.growth_warmup_rounds, len(pairs) // 4)
        pairs = pairs[warmup:]
        if len(pairs) < 2:
            return 0.0
        ys = [float(b) for _, b in pairs]
        xs = [float(s) for s, _ in pairs]
        mx, my = statistics.fmean(xs), statistics.fmean(ys)
        denom = sum((x - mx) ** 2 for x in xs)
        if denom == 0:
            return 0.0
        slope = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / denom
        return slope * 1_000.0

    def latency_drift(self) -> float:
        """Relative median-latency change, last tenth of rounds vs first."""
        if len(self.latencies) < 20:
            return 0.0
        tenth = max(1, len(self.latencies) // 10)
        first = statistics.median(self.latencies[:tenth])
        last = statistics.median(self.latencies[-tenth:])
        return (last - first) / first if first else 0.0

    def check(self) -> List[str]:
        """Returns threshold breaches; empty list means the soak passed."""
        t = self.config.thresholds
        p = self.latency_percentiles()
        out: List[str] = []
        if p["p99"] > t.max_p99_latency_ms:
            out.append(
                f"latency p99 {p['p99']:.2f}ms over ceiling {t.max_p99_latency_ms}ms"
            )
        if p["p50"] > 0 and p["p99"] / p["p50"] > t.max_tail_ratio:
            out.append(f"tail ratio p99/p50 {p['p99'] / p['p50']:.2f} over {t.max_tail_ratio}")
        drift = self.latency_drift()
        if abs(drift) > t.max_latency_drift:
            out.append(
                f"median latency drifted {drift:+.1%} over the run "
                f"(ceiling ±{t.max_latency_drift:.0%})"
            )
        peak_state = max(self.state_samples) if self.state_samples else 0
        if peak_state > t.max_state_bytes:
            out.append(f"state peaked at {peak_state} B over ceiling {t.max_state_bytes} B")
        growth = self.state_growth_per_1k()
        if growth > t.max_state_growth_per_1k:
            out.append(
                f"state grew {growth:.1f} B/1k submissions — retained set is not "
                f"bounded (ceiling {t.max_state_growth_per_1k})"
            )
        peak_mem = self.memory_peak()
        if peak_mem > t.max_memory_bytes:
            out.append(f"memory peaked at {peak_mem} B over ceiling {t.max_memory_bytes} B")
        self.breaches = out
        return out

    @property
    def passed(self) -> bool:
        return not self.breaches

    def to_dict(self) -> dict:
        p = self.latency_percentiles()
        return {
            "state_model": self.state_model,
            "rounds": self.rounds,
            "submissions": self.submissions,
            "seed": self.config.seed,
            "latency_ms": p,
            "latency_drift": round(self.latency_drift(), 4),
            "state_peak_bytes": max(self.state_samples) if self.state_samples else 0,
            "state_growth_bytes_per_1k": round(self.state_growth_per_1k(), 3),
            "memory_peak_bytes": self.memory_peak(),
            "mix_counts": self.kind_counts,
            "breaches": self.breaches,
        }


# --------------------------------------------------------------------------
# Runner
# --------------------------------------------------------------------------


def run_soak(
    cfg: Optional[SoakConfig] = None, track_memory: bool = True
) -> SoakResult:
    """Drives the sustained mix through the state model and measures it.

    Samples are taken once per round: round latency, retained state bytes and
    (optionally) the process's traced memory. The run is fully deterministic
    for a given ``cfg.seed``.
    """
    cfg = cfg or SoakConfig()
    rng = random.Random(cfg.seed)
    state = (
        LeakyStateModel(cfg.history_window, cfg.entry_bytes)
        if cfg.leaky
        else StateModel(cfg.history_window, cfg.entry_bytes)
    )

    tracer = None
    if track_memory:
        try:
            import tracemalloc

            tracemalloc.start()
            tracer = tracemalloc
        except ImportError:  # pragma: no cover - tracemalloc is stdlib
            tracer = None

    latencies: List[float] = []
    state_samples: List[int] = []
    submission_marks: List[int] = []
    memory_samples: List[int] = []
    counts: Dict[str, int] = {}
    total = 0
    counted_this_round: Set[str] = set()
    ingested_this_round = 0
    current_round = -1

    def _close_round() -> None:
        """Records one round's samples and starts the next round."""
        nonlocal counted_this_round, ingested_this_round
        latencies.append(
            round_latency(state, len(counted_this_round), ingested_this_round)
        )
        state_samples.append(state.bytes_retained)
        submission_marks.append(total)
        if tracer is not None:
            memory_samples.append(tracer.get_traced_memory()[1])
        counted_this_round = set()
        ingested_this_round = 0

    for sub in build_workload(cfg, rng):
        if sub.round_idx != current_round and counted_this_round:
            _close_round()
        current_round = sub.round_idx
        counts[sub.kind] = counts.get(sub.kind, 0) + 1
        state.apply(sub.asset, sub.source, sub.price)
        if not sub.duplicate:
            counted_this_round.add(sub.source)
        ingested_this_round += 1
        total += 1

    if counted_this_round:
        _close_round()
    if tracer is not None:
        tracer.stop()

    result = SoakResult(
        config=cfg,
        rounds=cfg.rounds,
        submissions=total,
        latencies=latencies,
        state_samples=state_samples,
        submission_marks=submission_marks,
        memory_samples=memory_samples,
        kind_counts=dict(sorted(counts.items())),
    )
    result.check()
    return result


# --------------------------------------------------------------------------
# Report
# --------------------------------------------------------------------------


def render_report(result: SoakResult) -> str:
    """Markdown report, written to the run summary and the artifact."""
    p = result.latency_percentiles()
    t = result.config.thresholds
    status = "PASS" if result.passed else "FAIL"
    rows = [
        f"# Soak report — {status}",
        "",
        f"- state model: `{result.state_model}`",
        f"- rounds: {result.rounds}, submissions: {result.submissions}",
        f"- seed: {result.config.seed} (reproduce with `--seed {result.config.seed}`)",
        f"- adversarial share: {adversarial_fraction(result.config.mix):.1%}",
        "",
        "| Metric | Value | Ceiling | Verdict |",
        "|---|---|---|---|",
        f"| latency p50 | {p['p50']:.2f} ms | — | — |",
        f"| latency p95 | {p['p95']:.2f} ms | — | — |",
        f"| latency p99 | {p['p99']:.2f} ms | {t.max_p99_latency_ms} ms | "
        f"{'ok' if p['p99'] <= t.max_p99_latency_ms else 'BREACH'} |",
        f"| latency drift | {result.latency_drift():+.1%} | ±{t.max_latency_drift:.0%} | "
        f"{'ok' if abs(result.latency_drift()) <= t.max_latency_drift else 'BREACH'} |",
        f"| state peak | {max(result.state_samples)} B | {t.max_state_bytes} B | "
        f"{'ok' if max(result.state_samples) <= t.max_state_bytes else 'BREACH'} |",
        f"| state growth | {result.state_growth_per_1k():.1f} B/1k | "
        f"{t.max_state_growth_per_1k} B/1k | "
        f"{'ok' if result.state_growth_per_1k() <= t.max_state_growth_per_1k else 'BREACH'} |",
        f"| memory peak | {result.memory_peak()} B | {t.max_memory_bytes} B | "
        f"{'ok' if result.memory_peak() <= t.max_memory_bytes else 'BREACH'} |",
        "",
        "## Workload mix (observed)",
        "",
        "| class | submissions |",
        "|---|---|",
    ]
    rows += [f"| {k} | {v} |" for k, v in result.kind_counts.items()]
    if result.breaches:
        rows += ["", "## Breaches", ""] + [f"- {b}" for b in result.breaches]
    return "\n".join(rows) + "\n"


def prometheus(result: SoakResult) -> str:
    lines = [
        "# HELP soak_latency_ms Round latency quantiles from the soak rig (#523).",
        "# TYPE soak_latency_ms gauge",
    ]
    for q, v in result.latency_percentiles().items():
        lines.append(f'soak_latency_ms{{quantile="{q}",model="{result.state_model}"}} {v:.4f}')
    lines += [
        "# HELP soak_state_bytes Retained state bytes after the run.",
        "# TYPE soak_state_bytes gauge",
        f'soak_state_bytes{{model="{result.state_model}"}} {max(result.state_samples)}',
        "# HELP soak_state_growth_bytes_per_1k Least-squares state growth slope.",
        "# TYPE soak_state_growth_bytes_per_1k gauge",
        f'soak_state_growth_bytes_per_1k{{model="{result.state_model}"}} '
        f"{result.state_growth_per_1k():.4f}",
        "# HELP soak_memory_peak_bytes Peak traced memory over the run.",
        "# TYPE soak_memory_peak_bytes gauge",
        f'soak_memory_peak_bytes{{model="{result.state_model}"}} {result.memory_peak()}',
        "# HELP soak_threshold_breaches Threshold breaches detected (1 if any).",
        "# TYPE soak_threshold_breaches gauge",
        f'soak_threshold_breaches{{model="{result.state_model}"}} {len(result.breaches)}',
    ]
    return "\n".join(lines) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--rounds", type=int, default=SoakConfig.rounds)
    p.add_argument("--seed", type=int, default=SoakConfig.seed)
    p.add_argument("--sources", type=int, default=SoakConfig.sources)
    p.add_argument("--assets", type=int, default=SoakConfig.assets)
    p.add_argument(
        "--model",
        choices=("bounded", "leaky"),
        default="bounded",
        help="'leaky' swaps in the unbounded-retention model to demonstrate detection",
    )
    p.add_argument("--json", type=Path, help="write the machine-readable result here")
    p.add_argument("--report", type=Path, help="write the markdown report here")
    p.add_argument("--prometheus", type=Path, help="write the metrics exposition here")
    p.add_argument(
        "--expect-breach",
        action="store_true",
        help="exit 0 only if a threshold was breached (used by the leak drill)",
    )
    args = p.parse_args(argv)

    cfg = SoakConfig(
        rounds=args.rounds,
        sources=args.sources,
        assets=args.assets,
        seed=args.seed,
        leaky=args.model == "leaky",
    )
    result = run_soak(cfg)
    report = render_report(result)
    print(report)

    if args.json:
        args.json.write_text(json.dumps(result.to_dict(), indent=2) + "\n")
    if args.report:
        args.report.write_text(report)
    if args.prometheus:
        args.prometheus.write_text(prometheus(result))

    if args.expect_breach:
        return 0 if result.breaches else 1
    return 0 if result.passed else 1


if __name__ == "__main__":
    sys.exit(main())
