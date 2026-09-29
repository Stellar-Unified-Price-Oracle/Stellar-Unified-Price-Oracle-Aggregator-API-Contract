"""SLA monitor v2 — machine-enforceable checks for docs/SLA.md (#416).

Availability alone cannot catch an oracle that is *up and wrong*. Every
published aggregate is therefore explained from its inputs and checked
against the clauses in ``docs/SLA.md`` §9:

* **Integrity** — the aggregate is independently recomputed from the raw
  counted submissions (same median as ``core_pricing::median_core``) and any
  divergence is reported in the ledger it is observed.
* **Accuracy / agreement** — the published aggregate must agree with the
  independent reference set, so a minority that shifts the median while
  every uptime gauge stays green still fires.
* **Liveness** — flatlined aggregates (value unchanged for N rounds while the
  inputs move) and implausible or inverted price relationships.
* **Source participation** — sources that are admitted but never counted, and
  sources that gain influence without an admission event.
* **SLO burn rate** — multi-window burn-rate alerting on the freshness SLO.

Every check maps to exactly one clause in :data:`SLA_CLAUSES`; the table in
``docs/SLA.md`` §9 is parsed by :func:`check_sla_consistency` so the document
and the monitor cannot drift apart silently.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict, deque
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Deque, Dict, Iterable, List, Mapping, Optional, Sequence, Set, Tuple

BPS = 10_000


@dataclass(frozen=True)
class Clause:
    clause: str
    metric: str
    owner: str
    threshold: str
    description: str


# Single source of truth for clause → metric → owner. Must match docs/SLA.md §9.
SLA_CLAUSES: Dict[str, Clause] = {
    c.clause: c
    for c in (
        Clause("1.2", "oracle_sla_price_age_seconds", "oracle-ops", "60",
               "Aggregate refreshed within 60 s of source submissions"),
        Clause("1.3", "oracle_sla_timestamp_skew_seconds", "oracle-ops", "300",
               "Submission timestamps within ±300 s of the ledger clock"),
        Clause("2", "oracle_sla_counted_sources", "source-onboarding", "2",
               "At least 2 active sources counted in every aggregate"),
        Clause("3", "oracle_sla_reference_deviation_bps", "risk", "500",
               "Aggregate within max_price_deviation of the reference set"),
        Clause("4.1", "oracle_sla_freshness_burn_rate", "oracle-ops", "14.4",
               "99.5 % freshness SLO; fast-burn alert on 1h/5m windows"),
        Clause("4.2", "oracle_sla_pause_duration_seconds", "governance", "7200",
               "Emergency pauses shorter than 2 hours"),
        Clause("6.3", "oracle_sla_accuracy_breach_bps", "risk", "1000",
               "Accuracy breach: deviation > 2x max_price_deviation"),
        Clause("9.1", "oracle_sla_aggregate_divergence", "core-contracts", "0",
               "Published aggregate equals independent recomputation"),
        Clause("9.2", "oracle_sla_flatline_rounds", "oracle-ops", "10",
               "Aggregate not flatlined while inputs move"),
        Clause("9.3", "oracle_sla_relationship_error_bps", "risk", "200",
               "Inverse/cross price relationships remain consistent"),
        Clause("9.4", "oracle_sla_uncounted_admitted_sources", "source-onboarding", "0",
               "No admitted source is silently excluded for a full window"),
        Clause("9.5", "oracle_sla_unadmitted_influence", "security", "0",
               "No source gains influence without an admission event"),
    )
}


@dataclass
class Round:
    """One published aggregate plus everything needed to explain it."""

    asset: str
    ledger: int
    timestamp: int
    published_price: int
    # source -> submitted price for the submissions the contract counted.
    counted: Dict[str, int]
    # source -> submission timestamp (optional; enables clause 1.3).
    submission_timestamps: Dict[str, int] = field(default_factory=dict)
    # Sources admitted on-chain (SourceAdded events) at this ledger.
    admitted: Set[str] = field(default_factory=set)
    # Independent reference prices (other venues / oracles), same decimals.
    reference_prices: Sequence[int] = ()
    # Whether the aggregate was fresh (age ≤ 60 s) when sampled.
    fresh: bool = True
    paused_seconds: int = 0


@dataclass
class Violation:
    clause: str
    metric: str
    owner: str
    asset: str
    ledger: int
    value: float
    detail: str

    def to_dict(self) -> dict:
        return asdict(self)


def recompute_median(prices: Iterable[int]) -> int:
    """Bit-exact port of ``core_pricing::median_core``."""
    xs = sorted(prices)
    n = len(xs)
    if n == 0:
        return 0
    if n % 2 == 1:
        return xs[n // 2]
    a, b = xs[n // 2 - 1], xs[n // 2]
    return a + (b - a) // 2  # b >= a, so floor == Rust truncation


def deviation_bps(a: int, b: int) -> int:
    if b == 0:
        return BPS if a != 0 else 0
    return abs(a - b) * BPS // abs(b)


def burn_rate(bad: Sequence[bool], budget: float) -> float:
    """Error rate over ``bad`` divided by the SLO error budget."""
    if not bad or budget <= 0:
        return 0.0
    return (sum(bad) / len(bad)) / budget


@dataclass
class MonitorConfig:
    max_deviation_bps: int = 500
    min_sources: int = 2
    max_price_age_secs: int = 60
    timestamp_threshold_secs: int = 300
    max_pause_secs: int = 7200
    flatline_rounds: int = 10
    participation_window: int = 20
    freshness_slo: float = 0.995
    burn_short_window: int = 5
    burn_long_window: int = 60
    burn_threshold: float = 14.4
    relationship_tolerance_bps: int = 200


class SlaMonitor:
    def __init__(self, config: Optional[MonitorConfig] = None) -> None:
        self.cfg = config or MonitorConfig()
        self._history: Dict[str, Deque[Round]] = defaultdict(
            lambda: deque(maxlen=max(self.cfg.participation_window, self.cfg.flatline_rounds))
        )
        self._fresh: Deque[bool] = deque(maxlen=self.cfg.burn_long_window)
        self._admitted_ever: Set[str] = set()
        self._latest: Dict[str, int] = {}
        self.violations: List[Violation] = []

    # -- helpers --------------------------------------------------------
    def _v(self, clause: str, r: Round, value: float, detail: str) -> Violation:
        c = SLA_CLAUSES[clause]
        return Violation(clause, c.metric, c.owner, r.asset, r.ledger, value, detail)

    # -- per-round checks ----------------------------------------------
    def observe(self, r: Round) -> List[Violation]:
        """Checks one round; violations are reported in the same ledger."""
        cfg = self.cfg
        out: List[Violation] = []
        prices = list(r.counted.values())

        # 9.1 independent recomputation
        expected = recompute_median(prices)
        if expected != r.published_price:
            out.append(self._v("9.1", r, r.published_price - expected,
                               f"published {r.published_price} != recomputed {expected}"))

        # 2 counted sources
        if len(prices) < cfg.min_sources:
            out.append(self._v("2", r, len(prices),
                               f"{len(prices)} counted sources < {cfg.min_sources}"))

        # 1.2 freshness
        if not r.fresh:
            out.append(self._v("1.2", r, cfg.max_price_age_secs, "aggregate stale"))

        # 1.3 timestamp skew
        for src, ts in r.submission_timestamps.items():
            skew = abs(ts - r.timestamp)
            if skew > cfg.timestamp_threshold_secs:
                out.append(self._v("1.3", r, skew, f"{src} timestamp skew {skew}s"))

        # 3 / 6.3 agreement with the independent reference set
        if r.reference_prices:
            ref = recompute_median(r.reference_prices)
            dev = deviation_bps(r.published_price, ref)
            if dev > cfg.max_deviation_bps:
                out.append(self._v("3", r, dev, f"aggregate {r.published_price} vs reference {ref}"))
            if dev > 2 * cfg.max_deviation_bps:
                out.append(self._v("6.3", r, dev, "accuracy breach (> 2x max deviation)"))

        # 4.2 pause duration
        if r.paused_seconds > cfg.max_pause_secs:
            out.append(self._v("4.2", r, r.paused_seconds, "pause exceeds 2h target"))

        # 9.5 influence without admission
        self._admitted_ever |= r.admitted
        for src in r.counted:
            if src not in r.admitted:
                out.append(self._v("9.5", r, 1, f"{src} counted without admission event"))

        hist = self._history[r.asset]
        hist.append(r)
        self._latest[r.asset] = r.published_price

        # 9.2 flatline while inputs move
        n = cfg.flatline_rounds
        if len(hist) >= n:
            tail = list(hist)[-n:]
            if len({x.published_price for x in tail}) == 1:
                inputs = {tuple(sorted(x.counted.items())) for x in tail}
                if len(inputs) > 1:
                    out.append(self._v("9.2", r, n, f"aggregate flat for {n} rounds while inputs moved"))

        # 9.4 admitted but never counted within the window
        w = cfg.participation_window
        if len(hist) >= w:
            window = list(hist)[-w:]
            counted_any = set().union(*(x.counted.keys() for x in window))
            for src in sorted(r.admitted - counted_any):
                out.append(self._v("9.4", r, w, f"{src} admitted but not counted in last {w} rounds"))

        # 4.1 multi-window freshness burn rate
        self._fresh.append(not r.fresh)
        budget = 1.0 - cfg.freshness_slo
        bad = list(self._fresh)
        short = burn_rate(bad[-cfg.burn_short_window:], budget)
        long_ = burn_rate(bad, budget)
        if short > cfg.burn_threshold and long_ > cfg.burn_threshold:
            out.append(self._v("4.1", r, long_, f"freshness burn rate {long_:.1f}x (short {short:.1f}x)"))

        self.violations.extend(out)
        return out

    def check_relationships(
        self, ledger: int, relations: Sequence[Tuple[str, str, str, str]], decimals: int = 7
    ) -> List[Violation]:
        """Checks ``(kind, a, b, c)`` relations against the latest aggregates.

        * ``("inverse", "X/Y", "Y/X", "")`` — X/Y * Y/X ≈ 1
        * ``("cross", "A/USD", "B/USD", "A/B")`` — A/USD / B/USD ≈ A/B
        """
        scale = 10**decimals
        out: List[Violation] = []
        for kind, a, b, c in relations:
            pa, pb = self._latest.get(a), self._latest.get(b)
            if pa is None or pb is None or pb == 0:
                continue
            if kind == "inverse":
                got, want = pa * pb // scale, scale
                name = f"{a}*{b}"
            else:
                pc = self._latest.get(c)
                if pc is None:
                    continue
                got, want = pa * scale // pb, pc
                name = f"{a}/{b} vs {c}"
            err = deviation_bps(got, want)
            if err > self.cfg.relationship_tolerance_bps:
                r = Round(asset=name, ledger=ledger, timestamp=0, published_price=got, counted={})
                out.append(self._v("9.3", r, err, f"{kind} relationship off by {err} bps"))
        self.violations.extend(out)
        return out

    # -- reporting ------------------------------------------------------
    def prometheus(self) -> str:
        counts: Dict[str, int] = defaultdict(int)
        for v in self.violations:
            counts[v.clause] += 1
        lines = ["# TYPE oracle_sla_violations_total counter"]
        for clause in SLA_CLAUSES:
            lines.append(f'oracle_sla_violations_total{{clause="{clause}"}} {counts[clause]}')
        return "\n".join(lines) + "\n"

    def report(self) -> str:
        if not self.violations:
            return "# SLA violation report\n\nNo violations.\n"
        rows = ["# SLA violation report", "",
                "| Ledger | Asset | Clause | Metric | Owner | Value | Detail |",
                "|---|---|---|---|---|---|---|"]
        for v in self.violations:
            rows.append(f"| {v.ledger} | {v.asset} | §{v.clause} | `{v.metric}` | {v.owner} "
                        f"| {v.value} | {v.detail} |")
        return "\n".join(rows) + "\n"


_ROW = re.compile(r"^\|\s*§?([0-9.]+)\s*\|\s*`([a-z_]+)`\s*\|\s*([a-z-]+)\s*\|\s*([0-9.]+)\s*\|")


def parse_sla_clause_map(sla_md: str) -> Dict[str, Tuple[str, str, str]]:
    """Parses the §9 clause map table of docs/SLA.md."""
    section = sla_md.split("## 9.", 1)[-1] if "## 9." in sla_md else ""
    out: Dict[str, Tuple[str, str, str]] = {}
    for line in section.splitlines():
        m = _ROW.match(line.strip())
        if m:
            out[m.group(1)] = (m.group(2), m.group(3), m.group(4))
    return out


def check_sla_consistency(sla_md: str) -> List[str]:
    """Returns human-readable mismatches between docs/SLA.md and SLA_CLAUSES."""
    doc = parse_sla_clause_map(sla_md)
    errors: List[str] = []
    for cid, c in SLA_CLAUSES.items():
        if cid not in doc:
            errors.append(f"clause {cid} monitored but missing from docs/SLA.md §9")
        elif doc[cid] != (c.metric, c.owner, c.threshold):
            errors.append(f"clause {cid}: doc {doc[cid]} != monitor {(c.metric, c.owner, c.threshold)}")
    for cid in doc:
        if cid not in SLA_CLAUSES:
            errors.append(f"clause {cid} documented but not monitored")
    return errors


def load_rounds(path: Path) -> List[Round]:
    rounds = []
    for line in path.read_text().splitlines():
        if line.strip():
            d = json.loads(line)
            d["admitted"] = set(d.get("admitted", []))
            rounds.append(Round(**d))
    return rounds


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--sla", type=Path, default=Path("docs/SLA.md"))
    p.add_argument("--rounds", type=Path, help="JSONL file of Round objects")
    p.add_argument("--check-sla", action="store_true", help="only check SLA.md consistency")
    args = p.parse_args(argv)

    errors = check_sla_consistency(args.sla.read_text())
    for e in errors:
        print(f"SLA consistency: {e}", file=sys.stderr)
    if errors or args.check_sla:
        return 1 if errors else 0

    mon = SlaMonitor()
    for r in load_rounds(args.rounds) if args.rounds else []:
        mon.observe(r)
    print(mon.report())
    return 1 if mon.violations else 0


if __name__ == "__main__":
    sys.exit(main())
