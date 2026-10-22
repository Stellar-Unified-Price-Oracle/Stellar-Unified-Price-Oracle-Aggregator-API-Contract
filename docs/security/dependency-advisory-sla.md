# Dependency Advisory Triage and Remediation SLA (#681)

## Overview

This policy establishes severity-based remediation deadlines, explicit component ownership, and automated release-blocking decisions across all dependency ecosystems:
- **Rust**: Workspace contracts, host dependencies, and crates evaluated via `cargo-audit` and `cargo-deny`.
- **JavaScript / TypeScript**: SDKs, CLI tooling, and web explorers audited via `npm audit`.
- **Operational**: Docker base images, CI runner actions, and infrastructure tooling.

---

## Severity & Remediation SLAs

| Severity | Remediation Deadline | Release Blocking? | Target Owner |
|---|---|---|---|
| **CRITICAL** | **24 hours** (1 calendar day) | **YES** — Immediate release halt | Security Lead & Component Tech Lead |
| **HIGH** | **7 calendar days** | **YES** — Blocks subsequent releases | Component Tech Lead |
| **MEDIUM** | **30 calendar days** | **NO** — Tracked and scheduled | Assigned Engineering Maintainer |
| **LOW** | **90 calendar days** | **NO** — Backlog maintenance | Assigned Engineering Maintainer |

---

## Ecosystem Ownership Roster

| Ecosystem | Scope | Default Owner Role |
|---|---|---|
| **Rust** | `contracts/**`, `Cargo.lock`, `deny.toml` | **Core Contracts Tech Lead** |
| **JavaScript / TypeScript** | `sdk/**`, `explorer/**`, `package.json` | **SDK & Frontend Tech Lead** |
| **Operational** | `docker/**`, `.github/workflows/**`, `scripts/**` | **DevOps & Infrastructure Lead** |

---

## Triage & Response Workflow

```
[Advisory Ingest] ────► [Severity Classification] ────► [Ticket Generation & SLA Due Date]
                                                                    │
                    ┌───────────────────────────────────────────────┴───────────────────────────────────────────────┐
                    ▼                                                                                               ▼
         [CRITICAL / HIGH]                                                                                   [MEDIUM / LOW]
       * Block Release Gate                                                                               * Create Tracking Ticket
       * Emergency Patch or Pinned Bump                                                                   * Target next scheduled release
       * Validate Reproducible Build                                                                      * Apply patch during sprint
```

1. **Advisory Discovery**:
   - Ingested automatically from CI `static-analysis` (`cargo audit --json`) or Dependabot/OSV.
2. **Ticket Generation**:
   - Each finding is converted into an actionable ticket carrying the advisory identifier, CVE/GHSA reference, affected package version, designated owner, and strict due date (`due_date = discovered_at + SLA_days`).
3. **Release-Blocking Gate**:
   - Any unresolved `CRITICAL` or `HIGH` finding halts the release pipeline until remediated or covered by a formal, time-bounded waiver signed by the Security Lead in `config/`.
4. **Remediation Verification**:
   - Upgrade dependency in workspace manifest (`=x.y.z`).
   - Regenerate lockfile and verify checksum integrity.
   - Run reproducible build gate (`make reproducible-build`).

---

## Automated Gating & Verification

Run the automated triage evaluator:

```bash
# Seeded test advisory evaluation
python -m services.dependency_advisory.triage --seed CRITICAL --ecosystem RUST --report security-artifacts/advisory-report.md
```

Run test suite:

```bash
python -m pytest services/dependency_advisory -q
```
