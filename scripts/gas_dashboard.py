#!/usr/bin/env python3
"""Gas cost attribution dashboard (#417).

Reads `GAS_SAMPLE,<endpoint>,<caller>,<paid>,<imposed>` lines on stdin
(emitted by `gas_amplification_tests.rs`) and writes a Markdown report with
per-endpoint mean / p95 / p99 / worst-case cost, the amplification ratio
(imposed / paid), sustained high-amplification callers, and a rent-inclusive
budget projection.

    cargo test -p price-oracle --lib gas_amplification -- --nocapture --test-threads=1 \
      | python3 scripts/gas_dashboard.py > gas-dashboard.md
"""

import math
import os
import sys
from collections import defaultdict

THRESHOLD = float(os.environ.get("AMPLIFICATION_THRESHOLD", "1.0"))
WINDOWS = int(os.environ.get("SUSTAINED_WINDOWS", "3"))
CALLS_PER_DAY = int(os.environ.get("CALLS_PER_DAY", "17280"))  # one per ledger
# Rent: persistent entries written per call, bumped every TTL window.
RENT_ENTRIES_PER_CALL = float(os.environ.get("RENT_ENTRIES_PER_CALL", "1"))
RENT_CPU_PER_ENTRY_BUMP = int(os.environ.get("RENT_CPU_PER_ENTRY_BUMP", "50000"))
TTL_BUMPS_PER_DAY = float(os.environ.get("TTL_BUMPS_PER_DAY", "1"))


def pct(values, p):
    s = sorted(values)
    return s[min(len(s) - 1, max(0, math.ceil(p / 100 * len(s)) - 1))]


def ratio(imposed, paid):
    return math.inf if paid == 0 and imposed > 0 else (imposed / paid if paid else 0.0)


def sustained(samples):
    streak, flagged = defaultdict(int), []
    for caller, paid, imposed in samples:
        streak[caller] = streak[caller] + 1 if ratio(imposed, paid) > THRESHOLD else 0
        if streak[caller] >= WINDOWS and caller not in flagged:
            flagged.append(caller)
    return flagged


def main():
    by_endpoint = defaultdict(list)
    ordered = []
    for line in sys.stdin:
        if not line.startswith("GAS_SAMPLE,"):
            continue
        _, endpoint, caller, paid, imposed = line.strip().split(",")
        paid, imposed = int(paid), int(imposed)
        by_endpoint[endpoint].append((caller, paid, imposed))
        ordered.append((f"{endpoint}:{caller}", paid, imposed))

    if not by_endpoint:
        sys.exit("no GAS_SAMPLE lines on stdin")

    out = ["# Gas Cost Dashboard", ""]
    out.append("Means hide the tail: a single adversarial call can cost many times the "
               "average, so worst-case and p99 are the numbers budgets must cover.")
    out += ["", "| Endpoint | Caller | n | mean CPU | p95 | p99 | worst | "
            "amplification (mean) | amplification (max) |",
            "|---|---|---|---|---|---|---|---|---|"]
    for endpoint, rows in sorted(by_endpoint.items()):
        paid = [r[1] for r in rows]
        ratios = [ratio(r[2], r[1]) for r in rows]
        out.append(f"| {endpoint} | {rows[0][0]} | {len(rows)} | {sum(paid) // len(paid):,} | "
                   f"{pct(paid, 95):,} | {pct(paid, 99):,} | {max(paid):,} | "
                   f"{sum(ratios) / len(ratios):.3f} | {max(ratios):.3f} |")

    flagged = sustained(ordered)
    out += ["", f"## Sustained amplifiers (ratio > {THRESHOLD} for {WINDOWS} consecutive samples)", ""]
    out += [f"- **{c}**" for c in flagged] or ["None."]

    out += ["", f"## Budget projection ({CALLS_PER_DAY:,} calls/day, rent included)", "",
            "| Endpoint | worst-case CPU/day | rent CPU/day | total/day | total/30d |",
            "|---|---|---|---|---|"]
    for endpoint, rows in sorted(by_endpoint.items()):
        worst = max(r[1] for r in rows) * CALLS_PER_DAY
        rent = int(RENT_ENTRIES_PER_CALL * CALLS_PER_DAY * RENT_CPU_PER_ENTRY_BUMP * TTL_BUMPS_PER_DAY)
        out.append(f"| {endpoint} | {worst:,} | {rent:,} | {worst + rent:,} | {(worst + rent) * 30:,} |")

    print("\n".join(out))
    if flagged and os.environ.get("FAIL_ON_AMPLIFIER"):
        sys.exit(1)


if __name__ == "__main__":
    main()
