# Mutation Testing

Line coverage says a test executes a line. Mutation score says the test would
**notice if that line changed**. For an oracle, the second number is the one
that matters: a median that silently becomes a mean, an authorization check
that always passes, or a TTL threshold that no longer expires anything all
execute cleanly and publish wrong prices.

## Running the gate

```bash
./scripts/mutation-gate.sh self-test     # gate logic only; no cargo-mutants
./scripts/mutation-gate.sh per-module    # per-module scores and thresholds
./scripts/mutation-gate.sh critical      # 100% kill rate on guarded files
./scripts/mutation-gate.sh general       # aggregate threshold over the rest
./scripts/mutation-gate.sh all           # everything
```

`self-test` needs only `bash` and `jq`, so the gate's own logic is verified on
every PR without waiting on a mutation run.

## Per-module thresholds (#520)

An aggregate score hides exactly the problem that matters. A large, well-covered
surface can carry a security-critical module that is barely tested at all, and
the aggregate still looks healthy. Thresholds are therefore set **per module** in
[`scripts/mutation-thresholds.conf`](../scripts/mutation-thresholds.conf):

```
<label> <file>[ <file>…]=<threshold>
```

| Module | Files | Threshold | Why it is held this high |
|---|---|---|---|
| storage | `storage.rs` | 95 | Holds the storage round-trip layer (#521) and the TTL/rent boundaries (#522). A mutant here corrupts a persisted value or lets an expired key read as live, and neither shows up as a failure elsewhere. |
| aggregation | `prices.rs`, `sources.rs` | 90 | A mutation publishes a wrong price that still looks valid. |
| auth | `rbac.rs`, `admin.rs` | 90 | A mutation is an authorization bypass — the highest-severity bug class in this contract. |
| finality | `finality.rs` | 85 | Supporting, but a failure silently weakens the guarantees above. |
| migration | `migration.rs` | 85 | Same. |
| everything else | — | 80 | `GENERAL_THRESHOLD`. |

A file not named in the config falls back to the general threshold, so adding a
source file to the crate can never silently drop it out of the gate.

### Changing a threshold

Thresholds are not lowered casually. A reduction needs a written justification
in the PR. A *waiver* is the mechanism for the legitimate case — see below.

## Equivalent-mutant waivers (#520)

An **equivalent mutant** is one the suite cannot kill because the mutated program
is observably identical to the original: a changed literal that nothing
asserts, a redundant re-computation, a bound the type system already enforces.
These are false negatives in the score, not gaps in the tests.

Waivers live in [`scripts/mutation-waivers.txt`](../scripts/mutation-waivers.txt):

```
<module>|<file>:<line>|<original>|<mutated>|<why equivalent>|<reviewer>|<date>
```

Two properties make the list safe rather than a loophole:

* **Pinned to a line.** The `file:line` means a refactor that moves the mutant
  invalidates the waiver, so it resurfaces as a failure instead of staying
  silently suppressed forever.
* **Reviewed, and self-test enforced.** Every entry must carry a reviewer and a
  behavioural justification, and the format is checked by
  `./scripts/mutation-gate.sh self-test`.

The list is currently empty: every mutant in the guarded modules is either
killed or is a real gap that needs a test. The format is committed so the first
genuine equivalent mutant has an explicit home instead of a blanket suppression.

## CI

| Job | Trigger | What it does |
|---|---|---|
| `gate-self-test` | every run | Verifies the gate's scoring, threshold and waiver logic. Fast; no cargo-mutants. |
| `per-module` | every run, sharded by module | One module per job, each enforced against its own threshold. Scoping per module is what keeps wall-clock time inside budget. |
| `critical` | PR + weekly | Any surviving mutant in the guarded set fails. |
| `general` | PR (diff-scoped) + weekly (full tree) | Aggregate threshold. |

Weekly runs cover the whole tree; PR runs are scoped so the gate stays useful
on feature branches.

## How scores are reported

`per-module` prints a table of module, file, score, threshold and pass/fail, and
uploads each module's `mutants.out` as an artifact. A module with **no results**
is a failure, not a pass: a gate that cannot measure a module must not report it
as healthy.
