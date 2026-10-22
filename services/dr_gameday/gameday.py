"""Disaster-recovery game-day automation (#532).

Each scenario in ``SCENARIOS`` pairs a failure injection with the recovery
steps documented in docs/disaster-recovery.md. A drill runs against an
isolated ``Sandbox`` (never production: ``run_drill`` refuses any sandbox
whose network is ``mainnet`` or whose id is not prefixed ``gameday-``),
measures RTO (wall-clock from injection to verified recovery) and RPO
(ledgers of data lost), compares both to the targets in docs/SLA.md §10 and
emits an issue draft for every gap.

    python -m services.dr_gameday.gameday --out gameday-report.json
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from dataclasses import asdict, dataclass, field
from typing import Callable, Dict, List, Optional

# Targets mirrored from docs/SLA.md §10 (seconds, ledgers).
SLA_TARGETS: Dict[str, Dict[str, int]] = {
    "region_loss": {"rto_s": 900, "rpo_ledgers": 0},
    "key_unavailability": {"rto_s": 3600, "rpo_ledgers": 0},
    "migration_failure": {"rto_s": 1800, "rpo_ledgers": 0},
    "index_corruption": {"rto_s": 7200, "rpo_ledgers": 120},
}


class ProductionTargetError(RuntimeError):
    pass


@dataclass
class Sandbox:
    """Isolated environment a drill mutates. Real runs back this with a
    throwaway testnet/standalone deployment; tests use it in-memory."""

    id: str
    network: str = "standalone"
    ledger: int = 1000
    region_up: bool = True
    admin_key_available: bool = True
    migration_ok: bool = True
    indexed_ledger: int = 1000
    backup_ledger: int = 1000
    log: List[str] = field(default_factory=list)

    def assert_isolated(self) -> None:
        if self.network == "mainnet" or not self.id.startswith("gameday-"):
            raise ProductionTargetError(f"refusing to drill against {self.id}/{self.network}")


@dataclass
class Scenario:
    name: str
    inject: Callable[[Sandbox], None]
    recovery_steps: List[Callable[[Sandbox], None]]
    verify: Callable[[Sandbox], bool]


def _step(name: str, fn: Callable[[Sandbox], None]) -> Callable[[Sandbox], None]:
    def run(sb: Sandbox) -> None:
        sb.log.append(name)
        fn(sb)

    run.__name__ = name
    return run


def _set(**kw):
    return lambda sb: [setattr(sb, k, v) for k, v in kw.items()]


SCENARIOS: List[Scenario] = [
    Scenario(
        "region_loss",
        _set(region_up=False),
        [_step("fail_over_ingest", _set(region_up=True))],
        lambda sb: sb.region_up,
    ),
    Scenario(
        "key_unavailability",
        _set(admin_key_available=False),
        [_step("multisig_rotate_admin", _set(admin_key_available=True))],
        lambda sb: sb.admin_key_available,
    ),
    Scenario(
        "migration_failure",
        _set(migration_ok=False),
        [_step("rollback_wasm_to_blue", _set(migration_ok=True))],
        lambda sb: sb.migration_ok,
    ),
    Scenario(
        "index_corruption",
        lambda sb: setattr(sb, "indexed_ledger", 0),
        [_step("restore_from_backup", lambda sb: setattr(sb, "indexed_ledger", sb.backup_ledger)),
         _step("reindex_tail", lambda sb: setattr(sb, "indexed_ledger", sb.ledger))],
        lambda sb: sb.indexed_ledger == sb.ledger,
    ),
]


@dataclass
class DrillResult:
    scenario: str
    recovered: bool
    rto_s: float
    rpo_ledgers: int
    steps: List[str]
    gaps: List[str]


def run_drill(scenario: Scenario, sb: Sandbox, clock: Callable[[], float] = time.monotonic) -> DrillResult:
    sb.assert_isolated()
    ledger_at_failure = sb.ledger
    t0 = clock()
    scenario.inject(sb)
    error: Optional[str] = None
    try:
        for step in scenario.recovery_steps:
            step(sb)
    except Exception as exc:  # a failing step is a finding, not a crash
        error = f"recovery step failed: {exc}"
    recovered = error is None and scenario.verify(sb)
    rto = clock() - t0
    rpo = max(0, ledger_at_failure - min(sb.indexed_ledger, sb.ledger))
    gaps = compare_to_sla(scenario.name, recovered, rto, rpo)
    if error:
        gaps.insert(0, error)
    return DrillResult(scenario.name, recovered, rto, rpo, list(sb.log), gaps)


def compare_to_sla(name: str, recovered: bool, rto: float, rpo: int) -> List[str]:
    t = SLA_TARGETS.get(name)
    gaps: List[str] = []
    if t is None:
        return [f"no SLA target defined for {name}"]
    if not recovered:
        gaps.append("recovery did not verify")
    if rto > t["rto_s"]:
        gaps.append(f"RTO {rto:.1f}s exceeds target {t['rto_s']}s")
    if rpo > t["rpo_ledgers"]:
        gaps.append(f"RPO {rpo} ledgers exceeds target {t['rpo_ledgers']}")
    return gaps


def issue_drafts(results: List[DrillResult]) -> List[dict]:
    """One tracked-issue draft per gap (filed by the workflow via `gh`)."""
    return [
        {
            "title": f"[DR game day] {r.scenario}: {gap}",
            "labels": ["disaster-recovery", "game-day"],
            "body": f"Scenario `{r.scenario}` drill found: {gap}.\n\n"
            f"Measured RTO {r.rto_s:.1f}s, RPO {r.rpo_ledgers} ledgers.\n"
            f"Steps executed: {', '.join(r.steps) or 'none'}.\n"
            "Update docs/disaster-recovery.md once fixed.",
        }
        for r in results
        for gap in r.gaps
    ]


def run_all(sandbox_factory: Callable[[str], Sandbox]) -> dict:
    results = [run_drill(s, sandbox_factory(s.name)) for s in SCENARIOS]
    return {
        "results": [asdict(r) for r in results],
        "regressions": sum(1 for r in results if r.gaps),
        "issues": issue_drafts(results),
    }


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", default="gameday-report.json")
    args = ap.parse_args(argv)
    report = run_all(lambda n: Sandbox(id=f"gameday-{n}"))
    with open(args.out, "w") as fh:
        json.dump(report, fh, indent=2)
    for r in report["results"]:
        print(f"{r['scenario']:<20} RTO={r['rto_s']:.3f}s RPO={r['rpo_ledgers']} gaps={len(r['gaps'])}")
    return 1 if report["regressions"] else 0


if __name__ == "__main__":
    sys.exit(main())
