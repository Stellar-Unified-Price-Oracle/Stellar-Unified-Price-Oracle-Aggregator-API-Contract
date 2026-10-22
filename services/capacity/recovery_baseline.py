"""Cold-start and recovery performance baseline (#703).

Measures wall-clock time, CPU consumption, memory footprint, and storage growth
required to rebuild local derived state (sources, submissions, aggregates, asset index)
from a fresh deployment (streaming from genesis) or a point-in-time snapshot.

Enforces documented regression thresholds to guarantee recovery time stays within
operational SLA RTO budgets.
"""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
import sys
import time
import tracemalloc
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

# Default recovery performance budgets based on SLA RTO limits:
# Fresh rebuild must sustain at least 500 events/sec (<= 2.0s per 1k events).
# Snapshot catch-up must sustain at least 1000 events/sec (<= 1.0s per 1k events).
DEFAULT_MAX_SECONDS_PER_1K_EVENTS = 2.0
DEFAULT_SNAPSHOT_MAX_SECONDS_PER_1K_EVENTS = 1.0
DEFAULT_MAX_MEMORY_MB = 256.0
DEFAULT_MAX_TOTAL_RECOVERY_SECONDS = 300.0  # 5 minute budget for 100k events


class RecoveryRegressionError(Exception):
    """Raised when cold-start or recovery performance exceeds budgeted thresholds."""


@dataclass(frozen=True)
class RecoveryBudget:
    """Documented performance thresholds for state rebuild."""

    max_seconds_per_1k_events: float = DEFAULT_MAX_SECONDS_PER_1K_EVENTS
    max_memory_mb: float = DEFAULT_MAX_MEMORY_MB
    max_total_recovery_seconds: float = DEFAULT_MAX_TOTAL_RECOVERY_SECONDS

    def check(self, metrics: "RecoveryMetrics") -> List[str]:
        """Check metrics against thresholds, returning list of violations."""
        violations = []
        if metrics.events_processed > 0:
            rate_per_1k = (metrics.wall_time_seconds / metrics.events_processed) * 1000.0
            if rate_per_1k > self.max_seconds_per_1k_events:
                violations.append(
                    f"Recovery rate regression: {rate_per_1k:.3f}s/1k events "
                    f"exceeds threshold of {self.max_seconds_per_1k_events:.3f}s/1k events"
                )
        if metrics.peak_memory_mb > self.max_memory_mb:
            violations.append(
                f"Peak memory regression: {metrics.peak_memory_mb:.2f} MB "
                f"exceeds threshold of {self.max_memory_mb:.2f} MB"
            )
        if metrics.wall_time_seconds > self.max_total_recovery_seconds:
            violations.append(
                f"Total recovery time regression: {metrics.wall_time_seconds:.3f}s "
                f"exceeds budget of {self.max_total_recovery_seconds:.3f}s"
            )
        return violations


@dataclass
class RecoveryMetrics:
    """Resource and timing measurements for derived state reconstruction."""

    mode: str
    events_processed: int
    ledgers_processed: int
    wall_time_seconds: float
    cpu_time_seconds: float
    peak_memory_mb: float
    derived_state_bytes: int
    events_per_second: float = 0.0
    passed_threshold: bool = True
    violations: List[str] = field(default_factory=list)

    def __post_init__(self):
        if self.wall_time_seconds > 0:
            self.events_per_second = round(self.events_processed / self.wall_time_seconds, 2)


SCHEMA = """
CREATE TABLE IF NOT EXISTS sources (
    source TEXT PRIMARY KEY,
    name TEXT,
    added_ledger INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS submissions (
    ledger INTEGER NOT NULL,
    event_index INTEGER NOT NULL,
    asset TEXT NOT NULL,
    source TEXT NOT NULL,
    price TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    PRIMARY KEY (ledger, event_index)
);
CREATE TABLE IF NOT EXISTS aggregates (
    ledger INTEGER NOT NULL,
    asset TEXT NOT NULL,
    price TEXT NOT NULL,
    source_count INTEGER NOT NULL,
    timestamp INTEGER NOT NULL,
    PRIMARY KEY (ledger, asset)
);
CREATE TABLE IF NOT EXISTS recovery_metadata (
    key TEXT PRIMARY KEY,
    value TEXT
);
"""


def init_db(conn: sqlite3.Connection) -> None:
    """Initialize SQLite schema for derived tables."""
    conn.executescript(SCHEMA)
    conn.commit()


def process_event(conn: sqlite3.Connection, event: Dict[str, Any]) -> None:
    """Ingest a single event into derived state tables."""
    topic = event.get("topic")
    ledger = int(event.get("ledger", 0))
    idx = int(event.get("event_index", 0))
    ts = int(event.get("timestamp", 0))
    data = event.get("data", {})

    if topic == "source_added":
        conn.execute(
            "INSERT OR REPLACE INTO sources (source, name, added_ledger) VALUES (?, ?, ?)",
            (data.get("source", ""), data.get("name", ""), ledger),
        )
    elif topic == "price_submitted":
        conn.execute(
            "INSERT OR REPLACE INTO submissions (ledger, event_index, asset, source, price, timestamp) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (ledger, idx, data.get("asset", ""), data.get("source", ""), str(data.get("price", "0")), ts),
        )
    elif topic == "price_aggregated":
        conn.execute(
            "INSERT OR REPLACE INTO aggregates (ledger, asset, price, source_count, timestamp) "
            "VALUES (?, ?, ?, ?, ?)",
            (ledger, data.get("asset", ""), str(data.get("price", "0")), int(data.get("source_count", 1)), ts),
        )


def benchmark_rebuild(
    events: Iterable[Dict[str, Any]],
    db_path: Path,
    mode: str = "fresh",
    budget: Optional[RecoveryBudget] = None,
    initial_snapshot_events: Optional[Iterable[Dict[str, Any]]] = None,
) -> RecoveryMetrics:
    """Benchmark the reconstruction of local derived state.

    Args:
        events: Stream of raw events to process.
        db_path: Target SQLite database file path.
        mode: 'fresh' for cold start from genesis, or 'snapshot' for snapshot catch-up.
        budget: Performance thresholds to evaluate against.
    """
    if budget is None:
        budget = RecoveryBudget(
            max_seconds_per_1k_events=(
                DEFAULT_SNAPSHOT_MAX_SECONDS_PER_1K_EVENTS
                if mode == "snapshot"
                else DEFAULT_MAX_SECONDS_PER_1K_EVENTS
            )
        )

    # Clean target DB if fresh
    if mode == "fresh" and db_path.exists():
        db_path.unlink()

    conn = sqlite3.connect(str(db_path))
    conn.execute("PRAGMA synchronous = NORMAL")
    conn.execute("PRAGMA journal_mode = WAL")
    init_db(conn)

    # Pre-populate snapshot if in snapshot recovery mode
    if mode == "snapshot" and initial_snapshot_events:
        with conn:
            for ev in initial_snapshot_events:
                process_event(conn, ev)

    tracemalloc.start()
    t0_wall = time.perf_counter()
    t0_cpu = time.process_time()

    event_count = 0
    highest_ledger = 0

    with conn:
        for ev in events:
            process_event(conn, ev)
            event_count += 1
            l_num = int(ev.get("ledger", 0))
            if l_num > highest_ledger:
                highest_ledger = l_num

        conn.execute(
            "INSERT OR REPLACE INTO recovery_metadata (key, value) VALUES (?, ?)",
            ("last_recovery_timestamp", str(int(time.time()))),
        )
        conn.execute(
            "INSERT OR REPLACE INTO recovery_metadata (key, value) VALUES (?, ?)",
            ("highest_ledger", str(highest_ledger)),
        )

    t1_wall = time.perf_counter()
    t1_cpu = time.process_time()
    _, peak_bytes = tracemalloc.get_traced_memory()
    tracemalloc.stop()

    conn.close()

    wall_time = t1_wall - t0_wall
    cpu_time = t1_cpu - t0_cpu
    peak_mb = peak_bytes / (1024 * 1024)
    state_bytes = db_path.stat().st_size if db_path.exists() else 0

    metrics = RecoveryMetrics(
        mode=mode,
        events_processed=event_count,
        ledgers_processed=highest_ledger,
        wall_time_seconds=round(wall_time, 4),
        cpu_time_seconds=round(cpu_time, 4),
        peak_memory_mb=round(peak_mb, 3),
        derived_state_bytes=state_bytes,
    )

    violations = budget.check(metrics)
    metrics.violations = violations
    metrics.passed_threshold = len(violations) == 0

    return metrics


def generate_synthetic_events(count: int, start_ledger: int = 1000) -> List[Dict[str, Any]]:
    """Generate reproducible event streams for benchmarking."""
    events = []
    assets = ["XLM", "USDC", "BTC", "ETH", "AQUA"]
    sources = ["source-alpha", "source-beta", "source-gamma", "source-delta"]

    # Source registrations
    for idx, s in enumerate(sources):
        events.append({
            "topic": "source_added",
            "ledger": start_ledger,
            "event_index": idx,
            "timestamp": 1700000000,
            "data": {"source": s, "name": f"Oracle {s}"},
        })

    # Submissions and aggregations
    curr_ledger = start_ledger + 1
    idx_counter = 0
    for i in range(count):
        asset = assets[i % len(assets)]
        source = sources[i % len(sources)]
        base_price = 10000000000000000000 + (i * 1000000)
        ts = 1700000000 + (i * 5)

        events.append({
            "topic": "price_submitted",
            "ledger": curr_ledger,
            "event_index": idx_counter,
            "timestamp": ts,
            "data": {"asset": asset, "source": source, "price": base_price},
        })
        idx_counter += 1

        if (i + 1) % len(sources) == 0:
            events.append({
                "topic": "price_aggregated",
                "ledger": curr_ledger,
                "event_index": idx_counter,
                "timestamp": ts,
                "data": {"asset": asset, "price": base_price, "source_count": len(sources)},
            })
            idx_counter += 1
            curr_ledger += 1

    return events


def prometheus_metrics(metrics: RecoveryMetrics) -> str:
    """Format recovery measurements as Prometheus metrics."""
    mode = metrics.mode
    lines = [
        f'# HELP oracle_recovery_wall_time_seconds Time taken to rebuild derived state',
        f'# TYPE oracle_recovery_wall_time_seconds gauge',
        f'oracle_recovery_wall_time_seconds{{mode="{mode}"}} {metrics.wall_time_seconds}',
        f'# HELP oracle_recovery_cpu_time_seconds CPU time consumed during rebuild',
        f'# TYPE oracle_recovery_cpu_time_seconds gauge',
        f'oracle_recovery_cpu_time_seconds{{mode="{mode}"}} {metrics.cpu_time_seconds}',
        f'# HELP oracle_recovery_events_per_second Event processing throughput',
        f'# TYPE oracle_recovery_events_per_second gauge',
        f'oracle_recovery_events_per_second{{mode="{mode}"}} {metrics.events_per_second}',
        f'# HELP oracle_recovery_peak_memory_mb Peak memory during rebuild',
        f'# TYPE oracle_recovery_peak_memory_mb gauge',
        f'oracle_recovery_peak_memory_mb{{mode="{mode}"}} {metrics.peak_memory_mb}',
        f'# HELP oracle_recovery_derived_state_bytes Resulting database file size in bytes',
        f'# TYPE oracle_recovery_derived_state_bytes gauge',
        f'oracle_recovery_derived_state_bytes{{mode="{mode}"}} {metrics.derived_state_bytes}',
        f'# HELP oracle_recovery_budget_pass 1 if within performance threshold, 0 if violated',
        f'# TYPE oracle_recovery_budget_pass gauge',
        f'oracle_recovery_budget_pass{{mode="{mode}"}} {1 if metrics.passed_threshold else 0}',
    ]
    return "\n".join(lines) + "\n"


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Cold-start and recovery performance baseline")
    parser.add_argument("--mode", choices=["fresh", "snapshot"], default="fresh", help="Rebuild mode")
    parser.add_argument("--events", type=int, default=2000, help="Number of events to benchmark")
    parser.add_argument("--events-file", type=Path, help="JSON file containing raw events")
    parser.add_argument("--db-path", type=Path, default=Path("recovery_test.db"), help="Target SQLite DB")
    parser.add_argument("--max-seconds-per-1k", type=float, default=DEFAULT_MAX_SECONDS_PER_1K_EVENTS)
    parser.add_argument("--max-memory-mb", type=float, default=DEFAULT_MAX_MEMORY_MB)
    parser.add_argument("--json", action="store_true", help="Output JSON result")
    parser.add_argument("--metrics", action="store_true", help="Output Prometheus metrics")
    parser.add_argument("--fail-on-regression", action="store_true", help="Exit 1 on threshold violation")

    args = parser.parse_args(argv)

    if args.events_file and args.events_file.exists():
        events = json.loads(args.events_file.read_text())
    else:
        events = generate_synthetic_events(args.events)

    budget = RecoveryBudget(
        max_seconds_per_1k_events=args.max_seconds_per_1k,
        max_memory_mb=args.max_memory_mb,
    )

    initial_snapshot = None
    if args.mode == "snapshot":
        # First 20% forms the snapshot, rest is catch-up replay
        split_idx = max(1, len(events) // 5)
        initial_snapshot = events[:split_idx]
        replay_events = events[split_idx:]
    else:
        replay_events = events

    metrics = benchmark_rebuild(
        events=replay_events,
        db_path=args.db_path,
        mode=args.mode,
        budget=budget,
        initial_snapshot_events=initial_snapshot,
    )

    if args.json:
        print(json.dumps(asdict(metrics), indent=2))
    elif args.metrics:
        print(prometheus_metrics(metrics), end="")
    else:
        status = "PASSED" if metrics.passed_threshold else "FAILED"
        print(f"=== Cold-Start & Recovery Baseline ({metrics.mode.upper()}) ===")
        print(f"Events processed:    {metrics.events_processed}")
        print(f"Wall time:           {metrics.wall_time_seconds:.3f} s ({metrics.events_per_second:.1f} events/s)")
        print(f"CPU time:            {metrics.cpu_time_seconds:.3f} s")
        print(f"Peak memory:         {metrics.peak_memory_mb:.2f} MB")
        print(f"State size:          {metrics.derived_state_bytes} bytes")
        print(f"Threshold status:    {status}")
        if metrics.violations:
            print("Violations:")
            for v in metrics.violations:
                print(f"  - {v}")

    # Clean up test db unless explicitly preserved
    if args.db_path == Path("recovery_test.db") and args.db_path.exists():
        args.db_path.unlink()

    if args.fail_on_regression and not metrics.passed_threshold:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
