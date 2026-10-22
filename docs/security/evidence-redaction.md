# Production incident evidence redaction (#687)

Incidents are when operators paste raw CLI output, RPC errors and environment
dumps into tickets and artifacts. That output is also where a Stellar secret
seed, an `Authorization` header or an `--admin-key` value is most likely to be
sitting. Once it reaches a log store or an artifact bucket, rotation is the best
remediation left — the value must be treated as compromised.

Code: [`services/evidence_redaction/`](../../services/evidence_redaction/).
Collection entry point: [`scripts/evidence.sh`](../../scripts/evidence.sh).

```bash
./scripts/evidence.sh --out incident-artifacts/evidence.md app.log rpc.log
python3 -m services.evidence_redaction.redact raw.log --out redacted.log --report redaction.md
python3 -m services.evidence_redaction.redact --check incident-artifacts/evidence.md
```

## What is removed, and what is kept

The filter is structural: it keeps the label and replaces the credential, so the
artifact stays diagnosable.

| Rule | Example input | Output |
|---|---|---|
| `stellar-seed` | `S…` (56 base32 chars) | `[REDACTED:stellar-seed]` |
| `pem-private-key` | a `BEGIN … PRIVATE KEY` block | `[REDACTED:pem-private-key]` |
| `auth-header` | `Authorization: Bearer eyJ…` | `Authorization: Bearer [REDACTED:auth-header]` |
| `key-flag` | `--admin-key S…` | `--admin-key [REDACTED:key-flag]` |
| `env-assignment` | `ADMIN_SECRET_KEY=S…`, `ORACLE_SOURCE_KEY=alice` | `ADMIN_SECRET_KEY=[REDACTED:env-assignment]` |
| `json-field` | `"api_key": "S…"` | `"api_key": "[REDACTED:json-field]"` |
| `url-userinfo` | `postgres://oracle:pw@db:5432/x` | `postgres://oracle:[REDACTED:url-userinfo]@db:5432/x` |
| `jwt` | `eyJ….….…` | `[REDACTED:jwt]` |

**Public chain data is deliberately kept.** Account addresses (`G…`), contract
ids (`C…`), ledger numbers and prices are public and are the substance of an
incident timeline. Only secret seeds (`S…`) are credentials. Redacting the
timeline would make the artifact useless without making it safer.

The filter is idempotent: running it twice changes nothing, which is what makes
`--check` meaningful on an artifact that is supposed to be clean already.

## Fail closed, with no pass-through

There is no supported way to emit raw evidence:

* `Redactor(enabled=False)` raises `RedactionDisabled` — it does not return the
  input;
* `redact_text(text, enabled=False)` raises;
* the CLI rejects `--no-redact` with exit code `2` **before writing anything**,
  not even to stdout;
* `scripts/evidence.sh` writes the artifact only through the redactor, so if the
  filter cannot run the script fails and no artifact appears.

A half-redacted artifact is worse than none, because it looks safe.

`--check` exits `1` if any rule still matches, for the case where an artifact was
produced elsewhere and needs vetting before it is attached to an incident.

## Reports do not leak

The report lists rule ids, line numbers and counts — never a matched value. The
redaction report cannot become the leak it is documenting.

```
| Rule | Line | Count |
|---|---|---|
| auth-header | 4 | 1 |
| stellar-seed | 6 | 1 |
```

Line numbers refer to the source document; a multi-line PEM block collapses into
one marker, so later line numbers after such a block shift.

## Fixtures are generated, not committed

The tests need a realistic seed, header and JWT. Committing them as literals
would trip this repository's own secret scanner
([`docs/secret-scanning.md`](../secret-scanning.md)), force an allowlist entry
for something that is not a secret, and teach the scanner to ignore a real
shape. So `services/evidence_redaction/fixtures.py` builds them at runtime from
a fixed seed. They are deterministic, and one test asserts the synthetic seed is
still detected by the repo's own scanner — proving the fixture is adversarial
without ever writing a key-shaped string to disk.

## Wiring

* `scripts/evidence.sh` — captures environment names, contract state and log
  tails, and pipes the bundle through the redactor (`--out` also writes a
  redaction report next to the artifact).
* Operators may pipe anything else through
  `python3 -m services.evidence_redaction.redact` directly.
* [`docs/disaster-recovery.md`](../disaster-recovery.md) points evidence capture
  at this script.

## Verification

```bash
python3 -m services.evidence_redaction.gate
python3 -m pytest services/evidence_redaction -q
```

The gate generates a synthetic incident bundle, asserts every rule fires, that
no secret survives, that structure and public chain data remain, that the report
quotes nothing, that redaction is idempotent, and that both the library and the
CLI refuse to run with redaction disabled while writing no artifact.

Related: the [RPC trust boundary](rpc-trust-boundary.md) keeps untrusted strings
out of log lines in the first place; this filter is the second line.
