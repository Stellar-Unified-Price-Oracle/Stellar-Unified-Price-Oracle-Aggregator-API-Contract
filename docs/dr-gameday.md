# Disaster-recovery game days (#532)

`services/dr_gameday/gameday.py` turns docs/disaster-recovery.md into
scheduled, measured drills.

| Scenario | Injection | Recovery steps |
|----------|-----------|----------------|
| `region_loss` | ingest region down | `fail_over_ingest` |
| `key_unavailability` | admin key lost | `multisig_rotate_admin` |
| `migration_failure` | WASM migration fails | `rollback_wasm_to_blue` |
| `index_corruption` | indexer wiped | `restore_from_backup`, `reindex_tail` |

- **Isolation:** drills refuse any sandbox on `mainnet` or whose id is not
  prefixed `gameday-` (`ProductionTargetError`). The workflow holds no
  production secrets.
- **Measurement:** RTO is wall-clock from injection to verified recovery;
  RPO is ledgers lost. Both are compared to docs/SLA.md §10.
- **Feedback:** every gap (failed step, unverified recovery, RTO/RPO over
  target) becomes an issue draft, filed by `.github/workflows/dr-gameday.yml`
  (weekly, Monday 06:00 UTC) with the `disaster-recovery` label.

Run locally: `python -m services.dr_gameday.gameday --out report.json`.
