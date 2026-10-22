# Python bot quickstart (price submitter)

```bash
pip install -r requirements.txt && pytest
ORACLE_SOURCE_SECRET=S... CONTRACT_ID=C... ASSET_ADDRESS=C... PRICE=1234.5 python bot.py
```

Key handling:
- Load `ORACLE_SOURCE_SECRET` from a secret manager (Vault, AWS/GCP secrets) or an env var injected at runtime — never commit it or bake it into images.
- Use a dedicated source key per bot with no other funds/privileges; rotate via `add_source`/removal governance.
- Tests use `Keypair.random()` only.

Errors: see [`docs/errors/REGISTRY.md`](../../docs/errors/REGISTRY.md) (e.g. `5 SourceNotFound` → the signer is not registered).
