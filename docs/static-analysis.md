# Static Analysis and SAST in CI

**Issue:** #500 — wire static analysis into CI (cargo-audit, cargo-deny,
cargo-geiger, and a source-level SAST pass) with a committed baseline and a
CI-enforced policy for new findings.

Dependency advisories and unsafe-code creep are silent regressions. Without a
baseline and a gate, every advisory is rediscovered manually — if at all. This
document is the operating procedure; the parts that matter are **enforced by
code**, not by convention.

| Claim | Enforced by |
|---|---|
| CI fails on a new (deliberately introduced) vulnerable dependency | `policy.check_advisories` + `gate.gate`, asserted in `test_a_deliberately_introduced_vulnerable_dependency_fails_the_gate` and `test_advisory_with_no_fix_needs_an_owned_time_boxed_allowlist_entry` |
| CI fails on unsafe additions beyond the baseline | `policy.exceedances` / `policy.check_unsafe`, asserted in `test_new_unsafe_code_in_a_baselined_file_fails_the_gate` and `test_a_new_file_containing_unsafe_code_fails_the_gate` |
| Every allowlisted finding has an owner and an expiry | `policy.AllowlistEntry.errors`, asserted in `test_entry_without_owner_is_rejected`, `test_expired_entry_is_rejected`, `test_entry_must_be_time_boxed` |
| The baseline cannot grow without review | `policy.baseline_digest` + `UnsafeBaseline.errors`, asserted in `test_edited_counts_without_reapproval_fail` and `test_rebaseline_records_the_reviewer_and_a_matching_digest` |
| False positives are tracked, not blanket-suppressed | path scope is mandatory and `**` is rejected, asserted in `test_blanket_path_scope_is_rejected_as_a_blanket_suppression` and `test_allowlisting_a_specific_file_keeps_scanning_everywhere_else` |
| A report is attached to every run | `policy.render_report` + the CI artifact upload, asserted in `test_gate_produces_a_human_readable_report` |

```bash
make sast         # or: python -m services.static_analysis.gate \
                  #        --report security-artifacts/static-analysis.md
```

---

## 1. The tools, and what each one covers

| Tool | Covers | Where |
|---|---|---|
| **cargo-audit** | RustSec advisories for the whole resolved graph (transitive included) | invoked by `gate.run_cargo_audit`, JSON parsed by `policy.parse_cargo_audit` |
| **cargo-deny** | Advisories, licenses, bans, duplicate and source checks | `deny.toml` + the existing `deny` CI job |
| **cargo-geiger** | Which crates actually use `unsafe` / FFI, and whether the *workspace* code does | the `geiger` CI step, summarised by the SAST rules below |
| **SAST pass** | Source-level rules over `contracts/**/*.rs` | `services/static_analysis/sast.py` |

`cargo-audit` is invoked when available and skipped (not silently passed) when
the tool is missing, so the SAST and baseline gates still run on a machine
without the Rust toolchain. CI installs it explicitly, so there the advisory
check is always live.

## 2. The SAST rules

`services/static_analysis/sast.py` is a narrow, deterministic rule set — the
constructs that matter in a `no_std` Soroban contract:

| Rule | Severity | Matches |
|---|---|---|
| `SAST-RUST-UNSAFE-BLOCK` | high | `unsafe { … }` and `unsafe fn` |
| `SAST-RUST-UNWRAP-UNCHECKED` | high | `.unwrap_unchecked()` |
| `SAST-RUST-UNWRAP` | medium | `.unwrap()` |
| `SAST-RUST-EXPECT` | medium | `.expect(…)` |
| `SAST-RUST-SLAB-PANIC` | low | `panic!`, `unreachable!`, `todo!`, `unimplemented!` |
| `SAST-RUST-NARROWING-CAST` | low | `as u8/u16/u32/u64` (truncation in fixed-point math) |

Two deliberate choices:

* **Commented-out code and doc prose are skipped** — a `// let v = x.unwrap();`
  in an explanatory comment is not a finding.
* **The idiomatic failure path is not a finding** — `panic_with_error!(env,
  ErrorCode::NoData)` is how this codebase is *supposed* to fail, so the raw
  panic-macro rule does not match it. A gate that flagged the correct pattern
  would be suppressed wholesale within a week.
* **Test files are out of scope** (`*_tests.rs`, `test.rs`, `prop_tests.rs`,
  `fuzz/`): an `unwrap()` in a test asserts a condition, it does not ship.

## 3. The baseline

`config/security-baseline.json` records the accepted number of findings per
`rule:file` pair, plus a digest that approves them:

```json
{
  "approved_by": "…", "approved_on": "2026-01-01",
  "approved_sha256": "…",
  "counts": { "SAST-RUST-UNWRAP:price-oracle/src/prices.rs": 2 }
}
```

* **Growth fails.** A file whose count exceeds the baseline is *unsafe-code
  creep*; a new file containing any finding fails too, because the pair is not
  in the baseline at all.
* **Shrinkage also fails.** A baseline entry with more slack than reality has is
  reported as drift, so the baseline cannot be left permanently pessimistic and
  quietly cover future additions.
* **Editing counts without re-approving them fails.** `approved_sha256` is the
  SHA-256 of the canonical serialization of `counts`. Change the counts without
  changing the digest and the gate errors out with the re-baseline command.

So the baseline can only grow through a deliberate, reviewable act.

## 4. The allowlist

`config/security-allowlist.json` holds acknowledged findings that CI will not
fail on. Every entry **must** carry:

| Field | Meaning |
|---|---|
| `id` | the rule id (`SAST-…`) or advisory id (`RUSTSEC-YYYY-NNNN`) |
| `kind` | `advisory`, `unsafe` or `sast` |
| `owner` | a person or team, not a bot — an unowned risk is an abandoned risk |
| `expires` | ISO date, at most **180 days** out |
| `reason` | why this specific finding is accepted |
| `paths` | glob scope, e.g. `["price-oracle/src/zk_verify.rs"]` |

Rules enforced by CI, not by review discipline:

* an entry without an owner, reason, expiry or path scope **fails**;
* an entry with a blanket scope (`**` or `*`) **fails** — a false positive is
  tracked where it lives, not muted repo-wide;
* an **expired** entry **fails**, so the allowlist cannot rot into a permanent
  mute;
* an entry dated more than 180 days out **fails** — allowlists are time-boxed.

An allowlisted `rule:file` pair is removed from the count comparison in *both*
directions, so it is neither reported as unsafe creep nor as baseline drift,
while every other file keeps being scanned.

## 5. Triage process

When the gate fails, work through the report artifact in this order:

1. **`## New findings`** — a real defect. Fix it, or add a path-scoped
   allowlist entry with an owner, a reason and an expiry ≤ 180 days out. New
   `unsafe` or `unwrap_unchecked` in price paths is a security change and needs
   a second reviewer.
2. **`## Dependency advisories`** — if a patched version exists, bump the
   dependency (see `docs/reproducible-builds.md` for the update procedure) and
   re-run. If none exists, add an advisory allowlist entry naming the crate, an
   owner and an expiry; the report flags it as `NO FIX` so the risk stays
   visible.
3. **`## Unsafe-code baseline`** — compare against the committed baseline. If
   the change is an improvement, tighten the baseline (below). If the change is
   new unsafe code, do not re-baseline: fix it.
4. **Allowlist errors** — an expired or ownerless entry is a maintenance task,
   not a reason to delete the entry. Re-review it and re-date it, or remove it
   and fix the finding.

## 6. Re-baseline process

Re-baselining is a code change with a review trail, not a local toggle:

```bash
# 1. make the change that moves the counts (or fix the findings)
# 2. re-approve deliberately, naming who reviewed it:
python -m services.static_analysis.gate --skip-advisories --rebaseline "<your-handle>"
# 3. commit config/security-baseline.json together with the change
```

`rebaseline` refuses an empty reviewer, stamps `approved_by` / `approved_on`,
and recomputes `approved_sha256`. In review, the diff shows both the count
change and who approved it. Reviewers should treat an unexplained count
increase in a *price* or *auth* path as a defect until proven otherwise.

**Do not re-baseline to make a red build green.** The gate failing is the
feature; a baseline bump needs a second pair of eyes and, for `unsafe` rules, a
security review.

## 7. Report artifact

Every run writes `security-artifacts/static-analysis.md` (counts by rule, the
full baseline, new findings, allowlisted findings, advisories and notes) and CI
uploads it alongside the raw SAST JSON, so a failure can be triaged from the run
page without re-running locally.

## 8. Out of scope

Runtime security monitoring and any automatic remediation: this gate reports,
it does not patch, and it never changes on-chain state.
