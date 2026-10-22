# Web quickstart (SEP-40 consumer)

Reads `lastprice` and `decimals` through RPC simulation — **no secret key is required or should ever be shipped to a browser**.

```bash
npm ci && npm test
CONTRACT_ID=C... ASSET=BTC npm start
```

Key handling: this template is read-only. If your web app must write, sign with a wallet (Freighter, WalletKit) in the user's browser — never embed a secret in frontend code or env vars bundled into the client.

Errors: simulation failures carry `Error(Contract, #N)`; look up `N` in [`docs/errors/REGISTRY.md`](../../docs/errors/REGISTRY.md).
