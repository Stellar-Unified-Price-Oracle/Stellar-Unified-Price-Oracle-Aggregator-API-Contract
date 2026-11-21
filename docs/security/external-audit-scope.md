# Focused External Audit Scope — Core Oracle Contract (#816)

Scope definition and audit pack for a focused external security audit of the
core `price-oracle` contract. This document is the single source of truth for
what the auditors review, what they do not, and what evidence they receive.

Companion documents (referenced, not duplicated):

| Document | Role in this engagement |
|---|---|
| [`../security-audit-checklist.md`](../security-audit-checklist.md) | Per-domain review checklist the auditors work through |
| [`stride-threat-model.md`](stride-threat-model.md) | Actors, assets, attacker capabilities, accepted risks |
| [`invariants.md`](invariants.md) | Safety/liveness invariants the audit must try to break |
| [`audit-finding-registry.md`](audit-finding-registry.md) | Where every finding is tracked (`config/audit-findings.json`) |
| [`../aggregation-semantics.md`](../aggregation-semantics.md) | Normative median/mean/trimmed/weighted/VWAP specification |
| [`../storage-tiers.md`](../storage-tiers.md) | Persistent vs temporary storage and TTL policy |
| [`../blue-green-upgrade.md`](../blue-green-upgrade.md) | Upgrade and rollback procedure |

---

## 1. Objective

Answer one question with evidence: **can an attacker who is not the admin make
the contract publish a price that is wrong, stale, or unavailable, or take
control of the contract's authority?**

The engagement is *focused*: contract source, storage layout, upgrade path and
aggregation math. Off-chain services are out of scope (see §4).

## 2. In scope

### 2.1 Core contract

`contracts/price-oracle/src/` — every module compiled into the crate:

| Area | Modules | What the auditors must attack |
|---|---|---|
| Entry points & auth | `lib.rs`, `admin.rs`, `sources.rs`, `assets.rs`, `pause.rs` | `require_auth` placement, re-initialization, role separation, pause bypass |
| Aggregation math | `prices.rs`, `aggregation*.rs` | Median/mean/trimmed/weighted/VWAP correctness, i128 overflow, rounding, empty/one-source sets, `min_sources_required` enforcement |
| Storage layout | `storage.rs`, `types.rs`, `history.rs` | `DataKey` uniqueness and aliasing, TTL extension on every load-bearing read, temporary vs persistent placement, history index consistency |
| Upgrade path | `admin.rs`, `timelock.rs` | `upgrade` auth and event ordering, timelock queue semantics, operation-id reuse, storage compatibility across WASM replacement |
| Events | `events.rs` | Completeness on every state change, topic collisions, accuracy of reported values |
| Cross-contract edges | `cross_chain*.rs`, `signed*.rs` (as compiled) | Replay, decoder forgery, signature binding |

### 2.2 Build artifacts

- The release WASM hash under audit, recorded in the audit pack (§5).
- `Cargo.lock` and the dependency tree (`cargo deny check`, `cargo audit` output).

### 2.3 Explicitly in scope as *design* review

- Single-admin risk and the timelock defaults (`docs/security-audit-checklist.md` §7).
- The `is_source_suspended` / `record_invalid_submission` stubs: the auditors
  must state whether the gap is acceptable for the intended deployment.

## 3. Out of scope

- `contracts/minimal-oracle/` (separate, minimal surface; audited separately if needed).
- Off-chain services: `services/rpc_guard/`, `services/indexer/`,
  `services/tx_review/`, `services/evidence_redaction/`, `scripts/`.
  These have their own gates (`make rpc-gate`, `make tx-review-gate`,
  `make redaction-gate`) and are covered by §8 of the checklist, not by this engagement.
- Frontend, SDKs, examples, deployment tooling.
- Economic/game-theoretic review of source incentives beyond what is needed to
  judge aggregation manipulation resistance.

## 4. Assumptions given to the auditors

Taken from [`stride-threat-model.md`](stride-threat-model.md) §3 and not to be
re-litigated: fees are non-zero; the admin key is a competent operator in normal
operation but the *compromised admin* case is in scope; a source key is distinct
from the admin key; Ed25519 is unforgeable; the host is correct but
resource-bounded.

## 5. Audit pack

Delivered to each firm as one archive plus a read-only repo link at the audited commit:

1. This scope document and the pre-audit checklist status.
2. `docs/security-audit-checklist.md` with every item marked reviewed / open.
3. `docs/security/stride-threat-model.md`, `invariants.md`, `aggregation-semantics.md`.
4. Contract source at the audited commit, plus the release WASM and its SHA-256.
5. Test inventory: `cargo test -p price-oracle --lib` output, coverage summary,
   and the property/fuzz corpora (`fuzz/`, `proptest-regressions/`).
6. Known limitations and accepted risks, each with a link to the code or doc
   that records the decision.
7. Finding intake: `config/audit-findings.json` schema and the lifecycle in
   [`audit-finding-registry.md`](audit-finding-registry.md).

## 6. Engagement plan

| Step | Owner | Done when |
|---|---|---|
| 1. Freeze scope | Security lead | This document merged; audited commit tagged |
| 2. Build audit pack | Security lead | §5 items assembled and hashed |
| 3. Request quotes from ≥2 Soroban-experienced auditors | Security lead | Both quotes recorded in §7 |
| 4. Select firm and schedule | Security lead + Release Manager | Start and end dates recorded in §7 |
| 5. Findings triaged | Security lead | One `OPEN` entry per finding in `config/audit-findings.json` |
| 6. Fixes and re-test | Owners | Findings `FIXED` with `fix_ref` |
| 7. Close-out | Reviewer ≠ owner | Findings `CLOSED`; report published per `docs/security/disclosure-process.md` |

## 7. Quote and schedule log

| Firm | Soroban experience (reference) | Quote received | Start | End | Status |
|---|---|---|---|---|---|
| _TBD_ | | | | | requested |
| _TBD_ | | | | | requested |

Update this table in the PR that records each quote or schedule change; do not
paste contract details or secrets here.

## 8. Done when

- This scope document is merged and linked from the pre-audit checklist.
- At least two quotes are recorded in §7 and one engagement is scheduled with
  dates.
- The audit pack of §5 is assembled for the tagged commit.
