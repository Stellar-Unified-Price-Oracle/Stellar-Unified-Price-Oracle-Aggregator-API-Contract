# STRIDE Threat Model — Price Oracle Aggregator (#503)

A maintained map of **what is defended, what is not, and why** for the contract,
its callers, its operators, and its cross-contract edges. Every threat below is
mapped to a concrete mitigation with a test or a code reference, or to an
explicit accepted-risk note. Nothing here is a reassurance without a reference.

Companion documents (referenced throughout, not duplicated):

| Document | Covers |
|---|---|
| [`endpoint-authority-matrix.md`](endpoint-authority-matrix.md) | Per-endpoint authority; kept honest by `rbac_escalation_tests::matrix_covers_every_endpoint` |
| [`invariants.md`](invariants.md) | Safety/liveness invariants + hostile-sequence harness |
| [`admin-compromise-drill.md`](admin-compromise-drill.md) | Admin-key blast radius and recovery |
| [`timelock-bypass-audit.md`](timelock-bypass-audit.md) | Timelock queue semantics |
| [`reputation-gaming.md`](reputation-gaming.md) | Reputation scoring economics |
| [`signed-submission-binding.md`](signed-submission-binding.md) | Ed25519 submission binding |
| [`cross-chain-replay-audit.md`](cross-chain-replay-audit.md) | Axelar/LayerZero/Wormhole relays |
| [`decoder-forgery-audit.md`](decoder-forgery-audit.md) | Untrusted-byte decoders |
| [`aggregation-semantics.md`](../aggregation-semantics.md) | Median/mean/trimmed/weighted/VWAP specification |

## 1. Assets

Anything whose loss, forgery, or unavailability changes a published price or the
authority to publish one.

| Id | Asset | Where it lives | Why it matters |
|---|---|---|---|
| A1 | Published aggregate price + history | `DataKey::Price*`, `DataHistory` | The product. Consumers (SEP-40) read it to move funds. |
| A2 | Source registry & admission | `SrcActive`, `OracleSources` | Determines *who* can influence A1. |
| A3 | Asset registry | `AssetRegistry`, `AssetMetadata` | Determines *what* can be priced. |
| A4 | Admin authority | `Admin` | Can admit sources, rewrite policy, and replace code. |
| A5 | Guardian quorum | `RecoveryGuardians` | The revocation path for a compromised A4. |
| A6 | Staked reputation | staking/treasury entries | Slashing lever; a financial asset in its own right. |
| A7 | Submission authorization | registered Ed25519 keys | Lets a submission act without Soroban auth. |
| A8 | Cross-contract trust config | `AxelarTrustedSource`, `LzTrustedRemote`, foreign asset mappings | The edge into other chains — spoofable if writable by the wrong party. |
| A9 | Relayer bonds | `RelayerBond*` | Financial asset; bonded relayers carry authority. |
| A10 | Subscription auto-renewal allowance | pre-approved token allowance | Spendable without a fresh signature. |

## 2. Actors

| Actor | Capability assumed |
|---|---|
| **Anonymous user** | Can call any permissionless endpoint and read all public state. Pays fees. |
| **Registered source** | Holds its own key; can submit prices for admitted assets. |
| **Malicious source** | A registered source that submits chosen values, withholds, or times out. Controls only its own submissions. |
| **Consumer** | A contract or account calling the SEP-40 read interface and the consumer-auth endpoints. |
| **Relayer** | Bonded; can relay a signed submission on a source's behalf. |
| **Challenger** | Can open a challenge against a published price. |
| **Admin** | Full control of all `admin`-gated endpoints, including `upgrade`. |
| **Compromised admin** | As above, under adversarial control. Explicitly in scope. |
| **Governance / external governor** | Acts through delegated role grants, scoped by an epoch. |
| **Remote chain** | An Axelar gateway, LayerZero endpoint, or Wormhole emitter. Semi-trusted: messages are authenticated but the *remote* state is not under our control. |
| **Host / network** | Correct, non-Byzantine, but with finite resource limits (the `Budget, ExceededLimit` class of failure). |


## 3. Attacker capabilities and assumptions

Stated explicitly, because most of the model depends on them.

**Assumed.**

- **A1.** Fees are non-zero, so on-chain spam is economic, not free.
- **A2.** The admin key is held by a competent operator in normal operation. The
  *compromised admin* case is analysed separately and is **not** assumed away.
- **A3.** A registered source is a distinct key from the admin. A source cannot
  call `admin`-gated endpoints (enforced by `require_auth`, checked by the
  authority matrix).
- **A4.** Ed25519 signatures are unforgeable; the host's `verify` is correct.
- **A5.** Hash/signature primitives (SHA-256, HMAC) behave as specified.
- **A6.** Arbitrary `Bytes`/`String` input is reachable from any caller — this is
  the premise of the #506 audit, not a hypothetical.
- **A7.** Persistent and temporary storage entries expire; the host enforces the
  ledger footprint limits rather than the contract.

**Explicitly *not* assumed.**

- That source keys are honest. A malicious source is in the model.
- That a consumer only calls endpoints it is entitled to. Access control is a
  control under test, not a premise.
- That network delivery is atomic or ordered. Cross-chain messages may be
  duplicated, delayed, or reordered.
- That the contract's input is well-formed. Untrusted bytes reach decoders.

## 4. Trust boundaries

| Id | Boundary | Crossing | Control |
|---|---|---|---|
| TB1 | External caller → contract | Any public endpoint | `require_auth`, role checks, validation (#506) |
| TB2 | Source → aggregate | `submit_price` | Registry membership, staleness, quorum, median |
| TB3 | Contract → consumer | SEP-40 `get_price` | `consumer_auth`, access mode, degraded-read policy |
| TB4 | Remote chain → contract | Axelar / LayerZero / Wormhole | Trusted-source registry, nonce/replay guards |
| TB5 | Admin → contract | 183 admin endpoints | Auth, timelock, audit log, guardian recovery |
| TB6 | Relayer → contract | `submit_price_relayed` | Bond, registered key, binding to `(source, asset, …)` |

## 5. STRIDE per component

Threats use STRIDE categories: **S**poofing, **T**ampering, **R**epudiation,
**I**nformation disclosure, **D**oS, **E**levation of privilege. Each row has a
mitigation reference or an explicit accepted-risk note.

### 5.1 Sources and price submission (`sources.rs`, `prices.rs`)

| Id | STRIDE | Threat | Mitigation / disposition |
|---|---|---|---|
| S1 | Spoofing | An unregistered address submits a price and is counted in the aggregate. | `check_source` rejects; invariant **S3** (`INV_REJECT_UNREGISTERED`) asserts it over hostile sequences. |
| S2 | Spoofing | A source submits *for another source* by passing its address as an argument. | Submission auth is `source.require_auth()`, so the caller must hold that key (authority matrix, 49 `caller:` endpoints). |
| T1 | Tampering | One source moves the published price. | Median over N sources; a single outlier cannot move the median unless it is the majority. Pinned by `test_median_frontrun_resistance_adversarial` and `test_median_outlier_injection_resistant`. |
| T2 | Tampering | A stale or future-dated submission is replayed to look fresh. | Timestamps checked on read; aggregate timestamp monotonicity is invariant **S5** (`INV_AGG_MONOTONIC`). |
| T3 | Tampering | `override_price` writes an arbitrary value with no market basis. | Admin-only by design; every override emits an event and an audit-log entry. **Accepted risk**: the admin can always set a price. Blast radius in `admin-compromise-drill.md`. |
| R1 | Repudiation | A source denies submitting a bad price. | Submission events plus an admin audit log with a verifiable chain (`verify_audit_chain`). |
| R2 | Repudiation | An admin denies a configuration change. | `AdminAuditLog` + `AdminActionEvent`; governance actions emit `GovernanceOp*` events. |
| I1 | Info disclosure | Consumer access mode is bypassed to read restricted state. | `consumer_auth::check_consumer_authorized`; `ConsumerAccessMode` / `ConsumerAuthorized` / `ConsumerBlocked` per (consumer, asset). |
| D1 | DoS | A source stops submitting, so the quorum is never met and prices go stale. | **Accepted risk** — inherent to a permissioned quorum. Detected by staleness monitoring; admin can `remove_source` and re-admit. Invariant L1 asserts the oracle does not stall *once quorum is met*. |
| D2 | DoS | A single huge `String`/`Bytes` bloats a ledger entry until writes fail. | **Mitigated** by #506: per-endpoint byte caps and canonical-form rejection (`input_validation.rs`), enforced by `input_validation_tests`. |
| E1 | Elevation | A source grants itself configuration rights. | Config endpoints require `admin.require_auth()`; a source key cannot satisfy it. |

### 5.2 Aggregation (`storage.rs`, `core_pricing.rs`)

| Id | STRIDE | Threat | Mitigation / disposition |
|---|---|---|---|
| T4 | Tampering | Quickselect selection bug yields a wrong median under adversarial input. | **Mitigated** by #505: an independent naive reference plus differential property tests over 1000 randomized cases per property (`reference_diff_tests`). |
| T5 | Tampering | Integer overflow in `mean`/`vwap` product with extreme `i128` values. | `saturating_mul`/`saturating_add` and `checked_add` in the sum path; `mean_no_overflow` (#464); boundary cases in `spec_mean_extremes_within_envelope`. |

### 5.3 Administration and governance (`admin.rs`, timelock, RBAC)

| Id | STRIDE | Threat | Mitigation / disposition |
|---|---|---|---|
| E2 | Elevation | Admin key compromise → total control, including `upgrade`. | **Accepted risk (critical)**, documented in `admin-compromise-drill.md` finding AC-1: replacement code can delete the recovery module, so only off-chain guardian monitoring of `ContractUpgradedEvent` detects it. Gating `upgrade` behind the timelock needs a governance decision. |
| E3 | Elevation | Delegated role holders escalate beyond their grant. | Per-role grants; bumping `GovernorEpoch` revokes every op grant at once; `external_governance.rs` scopes ops by allow-list. |
| E4 | Elevation | Timelock bypass — an operation executes early, or the queue is cancelled by a third party. | **Mitigated** by `timelock-bypass-audit.md`: the delay is snapshotted at proposal and execution requires `max(snapshot, current)`; per-element batch delay; only admin may cancel; re-queue yields a new id; an executed id is removed. |
| E5 | Elevation | The recovery guardians themselves are compromised. | **Accepted risk**: the quorum threshold is a governance parameter; documented in `admin-compromise-drill.md`. |
| R3 | Repudiation | A timelock operation executes that was never proposed. | `OperationProposed` / `OperationExecuted` / `OperationCancelled` events; ids are single-use. |
| D4 | DoS | A queued operation can never execute (stuck queue). | Admin may cancel and re-propose; cancellation emits an event. |

### 5.4 Cross-contract and cross-chain edges (`axelar_gmp.rs`, `layerzero.rs`, cross-chain modules)

| Id | STRIDE | Threat | Mitigation / disposition |
|---|---|---|---|
| S3 | Spoofing | A forged remote-chain message injects a price. | Messages are accepted only from a registered trusted source/remote (`AxelarTrustedSource`, `LzTrustedRemote`); the gateway address is admin-configured. |
| R4 | Repudiation / replay | The same remote message is delivered twice, double-counting a price. | **Mitigated** per `cross-chain-replay-audit.md`: inbound nonces are tracked (`get_lz_inbound_nonce`) and a consumed id cannot be replayed. |
| T8 | Tampering | A remote chain reports a price for an asset it has no standing to price. | Foreign asset mappings are admin-registered; unmapped assets are rejected. |
| T9 | Tampering | An oversized validator set, proof list, or batch is supplied to exhaust the ledger. | **Mitigated** by #506 count caps (`LIST_PARAMS`, 19 endpoints), enforced by `validate_list_len` and completeness-checked by `every_fixed_element_list_parameter_is_count_capped`. |
| E6 | Elevation | Anyone can register a trusted remote and become a price source. | `set_axelar_*` / `set_lz_*` are `admin`-gated; the authority matrix covers each. |

### 5.5 Relayers, challengers, subscriptions (`relayer.rs`, subscription modules)

| Id | STRIDE | Threat | Mitigation / disposition |
|---|---|---|---|
| S4 | Spoofing | A relayer relays a submission it was not authorised to relay. | `relayer.require_auth()` plus registration and bond; the payload is bound to `(source, asset, price, timestamp)` so it cannot be retargeted (`signed-submission-binding.md`). |
| T10 | Tampering | Auto-renewal drains a subscriber's allowance repeatedly. | **Mitigated** by #289: renewal requires a monotonic nonce and a current-period id; replays rejected (`RenewalAuthorizationReplay`), cancellation is permanent (`AutoRenewalCancelled`), allowance capped (`AutoRenewalAllowanceExceeded`). |
| R5 | Repudiation | A challenge outcome is disputed later. | Challenge records and reward accounting are on-chain and evented. |
| D5 | DoS | A challenger spams challenges to grief honest sources. | Griefing bounded by stake (`challenger_griefing_tests`). |

### 5.6 Resource and host boundary (TB8)


## 6. Accepted risks (consolidated)

Explicit non-goals, so they are not rediscovered as "gaps" each review.

| Id | Accepted risk | Rationale | Where tracked |
|---|---|---|---|
| AR1 | A compromised admin can replace contract code irrecoverably. | Gating `upgrade` behind the timelock breaks the current upgrade flow; needs a governance decision. | `admin-compromise-drill.md` AC-1 |
| AR2 | An honest-but-silent source can stall the quorum. | Inherent to permissioned quorum design; admin removal is the remedy. | S1 / D1 |
| AR3 | Some aggregation endpoints exceed the default budget. | Documented gas envelope; enforcing a limit is a separate change. | D6 |
| AR4 | Reputation farming is bounded, not impossible. | Integer flooring caps the achievable score at 81; farming costs the same as earning. | `reputation-gaming.md` |

**Closed during this work.** T7 (>128-input envelope divergence) was previously
listed here as an accepted risk. It is now **fixed**: `median_core` derives
parity from the window it actually copied, the bound is named
`core_pricing::MEDIAN_WINDOW`, and the behaviour is pinned by
`median_core_window_parity_is_consistent_above_the_envelope`. The residual
difference between the capped `median_core` and the uncapped
`storage::compute_median` remains, and is documented in
`docs/aggregation-semantics.md` rather than silently assumed away.

## 7. Update cadence and triggers

This model is only useful if it is revisited. **Update it when:**

- a new **public endpoint** is added, or an existing one's authority changes
  (the authority-matrix completeness test is the trigger point);
- a new **error code** or module lands that changes a mitigation's code reference;
- a **mitigation is removed or weakened**, or an accepted risk is opened/closed;
- an **incident, audit finding, or fuzz/proptest crash** implicates any row above;
- the **aggregation envelope**, source caps, or trust-config model changes.

**Cadence.** Reviewed at each release that touches the contract surface, and at
least once per quarter otherwise.

**Keeping it honest.** The rows most likely to rot are the endpoint *counts* and
the *code references*, so both are checked mechanically rather than by reading:
`rbac_escalation_tests::matrix_covers_every_endpoint` pins the authority table,
and `input_validation_tests::{every_string_parameter_is_classified,
string_param_table_has_no_stale_rows, every_bytes_parameter_is_bounded,
every_fixed_element_list_parameter_is_count_capped}` pin the validation tables.
A stale row fails CI instead of misleading a reviewer.

## 8. Traceability to issues

| Threat set | Issue |
|---|---|
| D2, T9 (input validation: text, bytes, and list caps) | #506 |
| T4, T5, T6, T7 (aggregation correctness, tie-break, rounding, envelope) | #505 |
| D7 (coverage of decoders and the entry-point surface) | #504 |
| This model | #503 |

## 9. Out of scope

Formal verification of the mitigations. A STRIDE row here is a claim backed by a
reference and a test, not a proof.

| Id | STRIDE | Threat | Mitigation / disposition |
|---|---|---|---|
| D6 | DoS | An unbounded batch or list exhausts the ledger footprint, failing the tx with `Budget, ExceededLimit`. | **Partially mitigated.** #506 caps every `Bytes` payload, every text field, and every fixed-element list (`LIST_PARAMS`, `MAX_DEPENDENCY_COUNT = 32`). **Accepted risk**: several heavy endpoints (full-scan aggregation, max-size batches) still exceed the default test budget — surfaced by `gas_budget_tests`, not yet enforced by a limit. |
| D7 | DoS | Decoder work is superlinear in the input length of an untrusted payload. | **Mitigated** by the input caps plus `decoder-forgery-audit.md`; fuzz coverage tracked under #504. |
| D8 | DoS | An untrusted endpoint traps (arithmetic overflow, index-out-of-bounds) instead of returning a contract error. | **Mitigated** by #504: `fuzz_endpoints` drives the public surface with type-directed fuzz input and treats any *host-level* failure as a finding. The harness's own endpoint sequence is replayed under `cargo test` by `fuzz_coverage_tests::harness_endpoint_sequence_is_well_behaved`, so a broken harness fails CI rather than silently fuzzing nothing. |

| T6 | Tampering | Tie-break/rounding ambiguity lets two parties disagree about the aggregate. | **Mitigated** by #505: semantics written down in `docs/aggregation-semantics.md` and pinned by `spec_*` tests — even counts floor the midpoint, exact-half weighted ties resolve to the next higher price. |
| T7 | Tampering | Above the 128-input envelope, `median_core` and `compute_median` disagree. | **Fixed + characterised** (see §6). `core_pricing::MEDIAN_WINDOW` is the single named bound; parity is now derived from the window length, and `median_core_window_parity_is_consistent_above_the_envelope` pins it. `set_max_sources` keeps production below the cap. |
| D3 | DoS | Aggregation cost grows superlinearly in source count. | Quickselect is O(n) expected; bounded by `set_max_sources`; gas envelope measured by `gas_budget_tests`. |

| TB7 | Signature → authority | Proof-carrying endpoints | Ed25519 verify + nonce/timestamp binding |
| TB8 | Contract → host | Every call | Budget/metering limits; resource-exhaustion class |
