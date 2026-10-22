# Soak and Load Rig

**Issue:** #523 — soak rig with a sustained adversarial mix, memory ceilings and
bounded state growth.

The headline load tests (`make load-test`, `load_v2_tests.rs`) measure a **burst**.
This rig measures the **slog**: a long, seeded run of a production-like submission
mix with an adversarial component, where the quantities that eventually turn into
an outage are the ones that matter — latency drift, memory, and state growth.

Everything here is pure-Python and hermetic (`services/soak/rig.py`), so the rig
runs in CI in seconds and the leak drill is part of the normal test suite. The
scheduled run in `.github/workflows/soak.yml` uses the same code path with a
longer `--rounds`, so a schedule failure is a code failure, never a drift
between "what CI checks" and "what the soak checks".

```bash
# Local run (defaults: 2,000 rounds, 7 sources, 3 assets, seed 20260928)
python -m services.soak.rig --rounds 2000

# Machine-readable outputs for the scheduled run
python -m services.soak.rig --rounds 20000 \
  --json soak.json --report soak-report.md --prometheus soak.prom
```

---

## 1. What is measured

| Quantity | How it is obtained | Why |
|---|---|---|
| Round latency (p50/p95/p99/max) | `round_latency()` per round, nearest-rank percentiles | A flat p50 with a rising p99 is the classic slow-burn signature |
| Latency drift | Median of the last tenth of rounds vs. the first tenth | Catches degradation a snapshot cannot see |
| Tail ratio | p99 / p50 | Catches tail growth a flat p99 could hide |
| Retained state | `StateModel.bytes_retained`, sampled once per round | State growth must be *bounded*, not merely slow |
| State growth rate | Least-squares slope of state bytes vs. submissions, per 1k, fitted after warmup | Turns "looks flat" into a number with a ceiling |
| Process memory | `tracemalloc` peak, sampled once per round | Host memory ceiling (see §3) |

Memory is deliberately sampled with `tracemalloc` rather than RSS: the rig
asserts a *ceiling on retained state and traced memory*, which is portable
between a local laptop and a runner. Per-source heap retention of the state model
itself is out of scope for this rig.

## 2. Workload mix

`DEFAULT_MIX` in `services/soak/rig.py` is the documented, production-like
distribution. Weights sum to 1.0.

| Class | Weight | p99 deviation | Behaviour |
|---|---|---|---|
| `routine` | 62% | 35 bps | Normal source traffic |
| `diurnal_burst` | 16% | 60 bps | 50% of its weight arrives as a same-ledger burst |
| `slow_drift` | 8% | 400 bps | Gradual, plausible one-sided drift |
| `price_gap` | 5% | 2,500 bps | Large but survivable gap (venue outage) |
| `stale_resubmit` | 4% | 50 bps | 70% duplicate/stale resubmission |
| `thundering_herd` | 3% | 80 bps | Whole source set, one ledger |
| `flash_spike` | 2% | 9,000 bps | Rogue outlier, 90% off-market |

**Realism guard.** `adversarial_fraction()` treats everything outside
`routine`/`diurnal_burst` as adversarial. It is asserted to stay **above 10%**
(the run is not trivially clean) and **below 34%** (the run still resembles
production — a 100%-adversarial soak measures a different system). The routine
class is asserted to hold the majority of the weight.

**Distributions.** Per-class deviation is drawn from a **logistic body** scaled
so its 99th percentile equals the class's `p99_bps`: oracle deviation is
empirically heavy-tailed (the common case is a few bps, the tail is a large
plausible move), and a normal body would either understate the tail or make the

## 3. Memory ceilings differ between host and native execution

This is called out in the issue as a risk, so it is explicit rather than
implicit. `Thresholds` carries separate ceilings per execution mode and the rig
never conflates them:

| Ceiling | Default | Applies to |
|---|---|---|
| `max_state_bytes` | 8 MiB | Retained oracle state (per-asset history). The on-chain ledger budget is far tighter; the host figure here is the *ingest-side* ceiling, which is what a submission bot actually holds |
| `max_memory_bytes` | 512 MiB | Traced process memory of the ingest/rig host |
| `max_p99_latency_ms` | 25 ms | Host-side end-to-end round |

Both are constructor arguments. A native/WASM run passes tighter values; the
soak report prints the ceiling next to the measurement so a breach is always
readable as "over *this* budget", never as an unexplained red mark.

## 4. Thresholds asserted

```
max_p99_latency_ms          25.0    p99 round latency
max_tail_ratio               4.0    p99 / p50
max_latency_drift           0.25    median drift, last tenth vs first
max_state_bytes        8 MiB        peak retained state
max_state_growth_per_1k     0.0    bytes per 1k submissions — must be bounded
max_memory_bytes      512 MiB       peak traced memory
```

`max_state_growth_per_1k = 0.0` is the load-bearing assertion: a pruning window
means the retained set is O(sources × assets × window) and the slope is exactly
zero. Any change that stops pruning shows up here as a non-zero slope long
before the absolute ceiling is reached.

The growth slope is fitted only after a warmup (`growth_warmup_rounds`,
default 200 rounds, clamped to a quarter of the run so short runs still fit a
slope). Without it, a *correctly* pruning model would be flagged for its
fill-up ramp — a false positive that would train everyone to ignore the alarm.

## 5. The leak drill — a deliberately leaky change is detected, demonstrated

`LeakyStateModel` is identical to `StateModel` except that it never prunes. It
exists so "the rig would catch a leak" is a demonstrated fact rather than a
claim:

```bash
python -m services.soak.rig --rounds 1500 --model leaky   # exits 1
python -m services.soak.rig --rounds 1500 --model leaky --expect-breach  # exits 0
```

Observed on the default config (`--rounds 1500`):

| | bounded | leaky |
|---|---|---|
| state growth | 0.0 B/1k | 64,000 B/1k (= `entry_bytes` × 1,000) |
| median latency drift | +0.3% | +29.8% |
| verdict | PASS | FAIL (growth **and** drift) |

The leak trips **two independent assertions**: the state-growth slope (the
direct signal) and the latency-drift ceiling (the indirect one, because a
growing retained set makes every subsequent round more expensive). The
detection is asserted across four seeds in
`services/soak/test/test_rig.py::test_leak_detection_is_robust_across_seeds`.

## 6. Scheduled runs and reports

`.github/workflows/soak.yml`:

- **Schedule** — nightly, a long run (20,000 rounds ≈ 160k submissions) that
  writes `soak.json`, `soak-report.md` and `soak.prom`, publishes the report
  into the run summary, and uploads all three as artifacts.
- **Pull request** — a short run (2,000 rounds) plus the leak drill, so a
  change that would leak is caught at review time rather than overnight.
- The job fails on any threshold breach (`main()` returns non-zero), so a
  regression pages through the normal CI-failure path.

`soak.prom` exposes `soak_latency_ms`, `soak_state_bytes`,
`soak_state_growth_bytes_per_1k`, `soak_memory_peak_bytes` and
`soak_threshold_breaches`, so soak history can be graphed next to the
production metrics in `docs/monitoring/`.

## 7. Out of scope

Capacity provisioning, and multi-region failover (that is #526, see
`docs/multi-region-ingest.md`).

## 8. Tests

```bash
python -m pytest services/soak -q    # 16 tests
```

Covers: mix weighting and the realism guard; nearest-rank percentiles; both
state models; workload determinism and class coverage; thresholds held over a
long run; state growth measured *and* bounded; memory sampled; the leak
detected; detection robust across seeds; the latency ceiling actually firing;
report/metric contents; and the CLI's exit codes and artifacts.

routine class absurd. The reference price follows a cyclic drift schedule
(`SoakConfig.drift_bps`), so rounds are not monotonically trending.

**Seeding.** The entire run is a pure function of `SoakConfig.seed`
(`random.Random(seed)`), reported in the report header. Any soak result —
including a CI failure — is reproducible with a single flag.
