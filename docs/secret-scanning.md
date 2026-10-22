# Secret Scanning and Leak Prevention

**Issue:** #502 — prevent secrets from ever entering the repository, with
pre-commit scanning, CI enforcement, a history scan, and a documented rotation
playbook.

A single committed key can compromise deployment or bridge integrations, and
git history makes removal painful. Prevention at commit time is far cheaper than
remediation.

| Claim | Enforced by |
|---|---|
| A deliberately committed fake secret fails CI | `services/secret_scan/scanner.py::main`, asserted in `test_cli_fails_on_a_committed_fake_secret` |
| Pre-commit catches it before it is written | `.husky/pre-commit` → `scanner --staged`, asserted in `test_staged_scan_catches_a_secret_before_it_is_committed` |
| Test fixtures are allowlistable without weakening scanning | path-scoped, owned, expiring entries; asserted in `test_fixture_secret_is_allowlistable` and `test_allowlisting_a_fixture_does_not_weaken_scanning_elsewhere` |
| History scan runs and reports findings | `scanner.scan_history`, asserted in `test_history_scan_reports_a_secret_that_was_already_removed` and `test_history_scan_is_clean_on_this_repository` |
| Scanning covers source, config and CI files | `scanner.scan_tree`, asserted in `test_scanning_covers_source_config_and_ci_files` |
| A rotation playbook exists and is actionable | §5 below |

```bash
make secret-scan                      # whole working tree
python -m services.secret_scan.scanner --staged   # what pre-commit runs
python -m services.secret_scan.scanner --history --report secret-artifacts/secret-scan.md
```

---

## 1. What is scanned

| Surface | How |
|---|---|
| Working tree (source, config, CI definitions, fixtures, docs) | `scan_tree` on every text file, skipping only `target/`, `node_modules/`, `vendor/`, `.git/` and binaries |
| Staged files (pre-commit) | `scan_staged` — `git diff --cached --name-only --diff-filter=ACM` |
| Full history | `scan_history` — every blob ever committed on any ref, reported with the commit that introduced it |

Binary suffixes (`.wasm`, images, archives) are skipped; a credential cannot be
recovered from them by this scanner and pretending otherwise would be a false
assurance.

## 2. Rules

| Rule | Matches |
|---|---|
| `SECRET-AWS-ACCESS-KEY-ID` | `AKIA…` (16 chars) |
| `SECRET-GITHUB-TOKEN` | `ghp_` / `gho_` / `ghu_` / `ghs_` / `ghr_` tokens |
| `SECRET-GITHUB-FINE-GRAINED-PAT` | `github_pat_…` |
| `SECRET-SLACK-TOKEN` | `xox[baprs]-…` |
| `SECRET-PRIVATE-KEY-BLOCK` | `-----BEGIN … PRIVATE KEY` |
| `SECRET-SOROBAN-SECRET-SEED` | Stellar secret seeds (`S` + 55 base32 chars) |
| `SECRET-GENERIC-ASSIGNMENT` | `api_key` / `secret` / `password` / `token` / `private_key` assigned a literal ≥ 12 chars |

**Findings are redacted.** The report and the CI log show at most the first and
last four characters; the value itself never reaches an artifact, a log line or
a test failure message.

## 3. False positives without disabling scanning

Two independent mechanisms, so legitimate material does not need a suppression:

1. **Placeholders are not findings at all.** Values containing `example`,
   `changeme`, `your…`, `dummy`, `redacted`, `xxxx`, `<…>`, `${VAR}`, `$(…)`,
   `{{…}}`, `os.environ[…]`, `process.env…`, `vault:`, `arn:aws`, … are ignored
   outright. The canonical AWS documentation key (`AKIAIOSFODNN7EXAMPLE`) is
   therefore not a finding — writing documentation does not require an entry.
2. **A path-scoped allowlist for material that must look real.** Fixtures and
   documentation that genuinely need credential-shaped data get an entry in
   `config/secret-scan-allowlist.json` with `id`, `kind: "secret"`, **`owner`**,
   **`expires`** (≤ 180 days out), `reason` and a `paths` glob.

CI rejects an entry that has no owner, no reason, no expiry, an expired date, a
date more than 180 days out, or a blanket `**` scope. A blanket scope is the
failure mode this design exists to prevent: it would silence the scanner for the
whole repository in exchange for one fixture. Bypassing with
`git commit --no-verify` is not a supported path — add the entry.

## 4. CI enforcement

The `secret-scan` job runs the tree scan **and** the history scan, and uploads
`secret-artifacts/secret-scan.md` to the run. Any new detection, any invalid
allowlist entry, or any historical finding fails the job.

A historical finding is a failure, not a warning, on purpose: material already
in the log is a live risk until it is rotated, and the remediation for that is
§5 — not a rebase.

## 5. Rotation playbook (a leaked credential)

Work top to bottom. Steps 1–3 are minutes; do them before anything else.

1. **Revoke first, investigate second.** Assume the credential is compromised
   the moment it is found — anyone with repository read access had it. Revoke
   or rotate at the provider (GitHub: *Settings → Developer settings → Personal
   access tokens*; AWS: deactivate the key; a Stellar account: rotate the
   account key on-chain and re-issue the deploy role). Revocation before
   forensics also limits the blast radius of a slow response.
2. **Record what leaked.** Capture the scanner report: rule, path, line,
   redacted value, and — for a history finding — the introducing commit. Attach
   it to the incident (`docs/incident-management/`). The redacted form is
   sufficient; do not paste the live value into a ticket, a chat message or a
   commit.
3. **Assess the blast radius.**
   * *Deploy / CI credentials* — follow `docs/secretless-ci.md` §5: federation
     means the correct response is usually to remove the long-lived secret
     entirely, not to rotate it.
   * *Stellar account / deploy key* — check on-chain for unexpected
     transactions and contract deployments; treat every contract deployed with
     the key as suspect (`docs/deployment.md`).
   * *Third-party API key* — check the provider's audit log for use you cannot
     account for, and rotate the dependent configuration at the same time.
4. **Purge the working tree.** Remove the value in a follow-up commit (or amend
   the introducing commit if it is on an unshared branch). **This is
   hygiene, not remediation**: the value stays in history and in every clone and
   fork. Never treat step 4 as the fix.
5. **Add a regression guard.** Add the credential's *shape* as a test fixture
   through the allowlist (§3) only if the value is genuinely fake; otherwise
   assert on the scanner rule that now covers it. The pre-commit hook and the CI
   job prevent the same value returning.
6. **Close the incident** with: what leaked, when it was revoked, what the
   blast-radius check found, and which guard now prevents a repeat.

### History rewriting is not part of the playbook

Rewriting history (`git filter-repo`, force-push) does not un-leak anything: the
value remains in every existing clone, in every fork, and in provider-side
caches. It is only worth doing to keep *new* clones from carrying dead material,
and it must never be done to other contributors' commits. Rotation (step 1) is
the control that actually reduces risk; history rewriting is cosmetic.

## 6. Out of scope

Runtime secret management (vaults, dynamic credential issuance, on-chain key
custody). This document covers repository-time prevention and the response to a
leak that got through.
