# Timing Side-Channel Review of Auth & Crypto Paths (#507)

Scope: every comparison site in `contracts/price-oracle/src/` that an auth or
cryptographic decision depends on. Harness:
`contracts/price-oracle/src/timing_shape_tests.rs` (runs in CI as part of
`cargo test -p price-oracle --lib`).

## Threat model — what "timing" means here

A Soroban caller cannot read a wall clock. What a caller *can* observe is the
**metered instruction count** of the transaction it submitted: directly from the
fee it was charged, from a `simulateTransaction` it runs itself, or from a
cross-call whose own budget is consumed by the callee. A comparison that returns
at the first difference therefore leaks *where* the first difference was, and
that is enough to recover a secret one byte at a time — a classic
"guess the value, watch the meter" attack needs no clock at all.

So the property under review is **constant shape**: the number of instructions a
comparison consumes must be a function of *declared, public* input sizes only,
never of *where* a secret value first diverges.

This review deliberately does **not** cover fault injection or physical side
channels (emissions, power, glitching), which are out of scope for the issue and
not addressable in contract code.

## Classification

Every site is classified **public** (compared values are readable on chain by
anyone, so their comparison shape leaks nothing) or **secret** (at least one
operand is not derivable from public state by the caller).

| # | Site | Operand | Class | Shape | Justification |
|---|------|---------|-------|-------|---------------|
| 1 | `rbac::has_role` — `caller == &admin` | admin address | public | value-compare | Both addresses are on-chain storage; there is no secret to recover. |
| 2 | `rbac::has_role` — role mask `!= 0` | delegated role flag | public | value-compare | The flag is written by an admin-only endpoint and readable by `has_role`, which is itself public. |
| 3 | `storage::check_source` / `check_registered_asset` — registry membership | source / asset address | public | value-compare | Registry contents are public queries. |
| 4 | `multisig::approve_operation` — `approvals.contains(&governor)` | governor address | public | value-compare | The signer set is readable via `ms_get_governors`. |
| 5 | `multisig::execute_ms_operation` — quorum / timelock comparisons | counters, ledger | public | value-compare | Pending operations are readable via `ms_get_operation`. |
| 6 | `signed_submission` — `env.crypto().ed25519_verify` | Ed25519 signature | host | host-provided | Verification is delegated to the host's `ed25519_verify`, a fixed-shape primitive outside contract control. The contract adds no comparison of its own on the signature bytes. |
| 7 | `signed_submission` — nonce `nonce <= last_nonce` | replay nonce | public | value-compare | The caller supplies `nonce` and therefore already knows it; the stored value is the caller's own last submission. |
| 8 | `zk_verify` — Fiat-Shamir tag `proof.fs_check == expected` | prover's fs tag | **secret** | **constant** | The tag commits to the prover's witness and is not derivable from public state. Previously compared with a length check plus a full-width XOR fold; the length check was a data-dependent early exit. |
| 9 | `zk_verify` — BN254 field helpers (`fp_add`, `u256_ge`, …) | curve arithmetic | **secret** | **constant** | Operands are derived from the witness. No early exit remains in any of them; each is a fixed sequence of limb operations. |
| 10 | `vdf_sampler::verify_vdf_proof` — 16-byte consistency loop | proof bytes | **secret** | **constant** | A forged proof's bytes are attacker-chosen and unknown to the contract; an early `return false` on the first mismatching byte revealed how much of a forgery was correct. |
| 11 | `provenance::verify_record` — commitment / id digests | record hashes | public | **constant** | Records are readable on chain. Compared in constant shape anyway so that no digest comparison in the contract is the one site with a different shape. |
| 12 | `provenance::verify_link` — chain predecessor equality | record hashes | public | **constant** | As above. |
| 13 | `cross_chain_verify::verify_cross_chain_price` — deviation vs threshold | published prices | public | value-compare | Both prices are stored by admin-set cross-chain observations and readable; the comparison is a magnitude test, not a secret comparison. |
| 14 | `audit_log::verify_audit_chain` — `previous_hash` / `current_hash` | audit hashes | public | value-compare | The audit log is readable via `get_admin_audit_log`. |
| 15 | `rbac::delegate_role` / `revoke_role` — admin `require_auth` | admin key | secret (host) | host-provided | Authorization is enforced by the host's signature check, not by a contract-side comparison. |
| 16 | `dead_man::heartbeat` — operator membership | operator address | public | value-compare | The operator set is readable via `dead_man_get_config`. |

## Changes made

* New `contracts/price-oracle/src/constant_time.rs` providing `bytes_eq`,
  `bytes_eq_counted`, `digest_eq` and `sig_eq`: every position of the longer
  input is inspected and per-byte differences are folded into an accumulator
  with `|=`, so the scan length never depends on the contents. Length is folded
  into the same accumulator rather than tested, so a length mismatch cannot skip
  the scan either.
* **Site 8**: `zk_verify` now compares the Fiat-Shamir tag through
  `constant_time::bytes_eq_counted`, removing the length early-exit.
* **Site 10**: `vdf_sampler::verify_vdf_proof` folds all 16 byte differences
  into an accumulator instead of returning at the first mismatch.
* **Sites 11–12**: `provenance::verify_record` uses `constant_time::digest_eq`
  for both digest comparisons.

## Accepted residual exposure

These are **accepted with reasoning**, not fixed, because fixing them would
change observable behaviour without changing the security property:

* **Host-provided primitives (sites 6, 15).** `ed25519_verify` and `require_auth`
  run in the host, outside contract control. The contract cannot add or remove
  work inside them, and a host that varied them would break far more than this
  contract. Documented rather than "fixed" because there is nothing to fix here.
* **Loop bounds over declared lengths (all sites).** Every loop still has a
  bound. That bound is a function of a *declared, public* length in every call
  site (fixed-width `BytesN<32>` / `BytesN<64>`, or a caller-supplied buffer
  length that the caller already knows), never of a secret. This is the same
  assumption every constant-time primitive in the ecosystem makes.
* **Storage-read patterns on secret-derived state.** A storage miss costs
  differently from a hit. No such site was found in the auth path: reads are
  keyed by public identifiers (source, asset, operation id), never by a secret.
* **`gas_metering` instrumentation.** `submit_price` records its own CPU cost
  into `LastGasRecord`. This is a *measurement* of metering, not a comparison
  whose shape depends on a secret, and it is admin-readable only.
* **Field arithmetic in `zk_verify`.** Multiplication and inversion are
  fixed-shape limb loops, but the *number of iterations* of `fp_pow` depends on
  the exponent, which is derived from the verifying key (admin-set, therefore
  public). This was checked rather than changed.

## Test coverage

`timing_shape_tests.rs` asserts, for the comparison helpers:

* a mismatch in the **first** byte, in the **last** byte, and in **every** byte
  position all inspect the same number of byte positions;
* equal buffers inspect the same number of positions as unequal ones;
* a length mismatch inspects `max(len_a, len_b)` positions — the scan is not
  skipped;
* `digest_eq` and `sig_eq` always scan their full fixed width;
* `verify_vdf_proof` returns the same verdict for proofs differing in different
  numbers of leading bytes, confirming the accumulator fold did not reintroduce
  an early exit;
* the classification table above is checked against the source: no reviewed
  module contains a `return false` inside a byte-comparison loop, so a future
  edit that reintroduces an early exit fails the build's test run.
