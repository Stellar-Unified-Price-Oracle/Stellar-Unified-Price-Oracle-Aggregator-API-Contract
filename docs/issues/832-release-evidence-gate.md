# #832 Release evidence gate — remaining work

## Built
- `scripts/check-release-evidence.sh` — parses a Markdown checklist, fails on
  unchecked required items (`- [ ]`) and on checked items without an evidence
  link (Markdown link or bare URL). `- [~]` marks optional items. Exits 2 when
  the checklist file is missing, 1 on gate failure, 0 on pass.

## Remaining
1. `docs/mainnet-release-checklist.md` — the checklist the workflow passes in.
   Sections: build reproducibility, audit sign-off, governance proposal,
   canary results, rollback plan, monitoring/alerts, comms. Every required item
   must carry an evidence link; mark genuinely optional items `[~]`.
2. Tests: `scripts/test_check_release_evidence.py` (pytest, matching
   `scripts/test_docs_freshness.py` style) with fixtures
   `scripts/fixtures/release-evidence/{passing,failing-unchecked,failing-no-evidence,missing}.md`,
   asserting exit codes 0/1/1/2 and the `::error` annotations.
3. Wire the test into CI (the `release-evidence` job in
   `.github/workflows/release.yml` or the existing pytest job) so the gate's
   own behaviour is covered.

## Done when
- Tag pushes (`v*`) run the gate against a real checklist and fail on content.
- `pytest scripts/test_check_release_evidence.py` passes.
