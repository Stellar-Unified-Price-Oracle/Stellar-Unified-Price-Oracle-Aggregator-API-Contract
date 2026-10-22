# Coverage-guided fuzzing

> Issue **#514**. See also the [attack-regression corpus](attack-regression-corpus.md).

Ad-hoc fuzzing discovers a bug once and then forgets it. This setup keeps the
inputs that found bugs **in version control**, replays them on every PR, and
schedules long runs that refresh the corpus.

## Targets

| Target | Layer | What it pins |
|---|---|---|
| `fuzz_aggregation` | aggregation | `core_pricing::*` and the SDK-`Vec` wrappers in `storage.rs` must agree bit-for-bit |
| `fuzz_quickselect` | aggregation | `quickselect_core` matches a sorted reference, with a correct partition |
| `fuzz_aggregation_invariants` | aggregation | trimmed mean / VWAP / weighted median security invariants |
| `fuzz_storage_layer` | storage | the encode/decode round-trip and the bounds that guard it |

`fuzz_storage_layer` (added in #514) covers what the aggregation targets cannot
reach: a `PriceEntry` is written to Soroban persistent storage and read back,
and the decoded values are driven through the storage-backed computations. Its
invariants are stated as things an attacker cannot violate, not as a
re-implementation of the contract:

| Invariant | Meaning |
|---|---|
| `INV-STORE-ROUNDTRIP` | a value written to storage reads back bit-identical — a lossy or aliasing encoding would let one source's price be served as another's |
| `INV-NO-PANIC` | no decoded input may panic the contract (a panic is a DoS reachable from one call) |
| `INV-BOUNDED` | every aggregate lies within the `[min, max]` of the values that influenced it |
| `INV-VWAP-NONNEGATIVE-VOLUME` | non-positive volume carries no weight |
| `INV-CONFIDENCE-NONNEGATIVE` | confidence is a non-negative ratio, never above 100% |
| `INV-TIMESTAMP-ORDER` | a newer submission never lowers the recorded ledger timestamp |

Prices and volumes are decoded as `i32` so sums and products cannot saturate,
which keeps the invariants exact rather than approximate.

## The corpus

Corpora live in `fuzz/corpus/<target>/` and are **committed**. Each is small and
reviewed:

| Target | Entries | Size |
|---|---|---|
| `fuzz_aggregation` | 3 | 16–40 B |
| `fuzz_quickselect` | 2 | 17–25 B |
| `fuzz_aggregation_invariants` | 5 | 25–41 B |
| `fuzz_storage_layer` | 7 | 25–41 B |

## The gate

```sh
make fuzz-gate                    # size checks + corpus replay
FUZZ_SKIP_REPLAY=1 make fuzz-gate # size checks only (no toolchain needed)
```

`scripts/fuzz-corpus-gate.sh` does two things:

1. **Size budget.** Every corpus must exist, be non-empty, stay under
   `CORPUS_MAX_ENTRIES` (64) entries, and every entry must be under
   `CORPUS_MAX_BYTES` (4096). This is what keeps the corpus *reviewable* — a
   corpus that grows without bound stops being something anyone reads, and an
   empty corpus that passes is worse than no corpus at all.

2. **Replay.** Every committed corpus entry is re-run through its target. Any
   crash fails the gate. Replaying the *committed* corpus (rather than fresh
   random input) is what makes a crash reproducible: the bytes that found it are
   in version control.

In CI this is the `corpus-gate` job in `.github/workflows/fuzz.yml`, which runs on
every PR to `main`.

## Long runs

`scripts/fuzz-corpus-gate.sh` is the fast, deterministic, per-PR half. The deep
coverage-guided runs are the `long-run` job in the same workflow, on a **weekly**
schedule (`17 3 * * 1`) and on `workflow_dispatch`:

* 5,000,000 iterations or a 60-minute cap per target, sharded across the four
  targets.
* **Crash artifacts are uploaded** on failure (`-error_exitcode=1` makes the step
  fail too), so a crash cannot pass silently.
* The **refreshed corpus is uploaded** on success for review, so newly
  discovered paths are not thrown away.

## Committing a new crash

Per the acceptance criteria, a crash reproducer is committed automatically as a
regression seed:

1. The `long-run` job fails and uploads `fuzz/artifacts/` and the corpus.
2. Minimize the reproducer so the corpus stays small:

   ```sh
   cargo fuzz cmin fuzz_storage_layer
   ```

3. Move the minimized artifact into `fuzz/corpus/<target>/` with a descriptive
   name (the existing seeds use `seed_<what-it-exercises>`), and commit it.
4. Re-run `make fuzz-gate` to confirm the corpus is still within budget and the
   crash is now pinned.

The same reproducer should normally *also* become an entry in the
[attack-regression corpus](attack-regression-corpus.md), so the attack is
documented and classified rather than existing only as an opaque byte string.

## Reproducing a failure

libFuzzer failures are reproducible by construction — the input is the artifact:

```sh
cargo fuzz run fuzz_storage_layer fuzz/artifacts/fuzz_storage_layer/crash-<hash>
```

## Out of scope

Third-party bridge code is not fuzzed here; only this contract's aggregation and
storage layers are.
