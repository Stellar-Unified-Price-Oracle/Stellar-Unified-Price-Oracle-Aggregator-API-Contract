# Stellar Unified Price Oracle

[![CI](https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/actions/workflows/ci.yml/badge.svg)](https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![SEP-40](https://img.shields.io/badge/interface-SEP--40-informational)](docs/sep40-conformance.md)
[![Status](https://img.shields.io/badge/status-testnet%20only-orange)](docs/deployment.md)

**One trustworthy price for Stellar DeFi.** A Soroban contract that collects
prices from multiple permissioned sources, aggregates them by **median**, and
serves current and historical prices to other contracts through the standard
**SEP-40** oracle interface — so one bad or compromised feed cannot move the
price.

> **Security notice (November 2026):** two pull requests merged on 2026-10-22
> contained a third-party malware payload (the "PolinRider" campaign). It has
> been removed and CI now blocks it. If you cloned this repo between
> 2026-10-22 and the cleanup, read
> [docs/security/malware-incident-2026-10.md](docs/security/malware-incident-2026-10.md).

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the system diagram
(contracts, off-chain services, data flow for a price update and for a
governance change), module structure, storage layout and event catalogue.

## Features

- **Median aggregation** across N permissioned sources, with a configurable
  minimum quorum (`min_sources_required`).
- **SEP-40 consumer interface**: `lastprice`, `price`, `prices`, `assets`,
  `decimals`, `resolution`, `base`.
- **Price history** per asset with ranged and cursor-paginated reads and
  optional gap interpolation.
- **Source enrollment with evidence**: sources submit identity, endpoint
  ownership, contact and reliability evidence before an admin activates them.
- **Asset lifecycle**: register, temporarily disable, re-enable, delist.
- **Admin-gated upgrades and configuration**, with events for every change.
- **Client SDKs** (TypeScript, Python, Rust, Go) and runnable consumer
  [examples](examples/).
- **Security gates in CI**: unit, property, model-based and Kani-bounded tests,
  attack-regression corpus, fuzzing, secret scanning and a supply-chain
  malware scan.

## How it works

```
 source A ─┐
 source B ─┼─ submit_price ─▶ [ Price Oracle contract ] ─▶ median ─▶ AggregatePrice + history
 source C ─┘   (source auth)        │                                     │
                                    └── admin: sources, assets, config    └─▶ SEP-40 reads by
                                                                               consumer contracts
```

| Network | Contract ID | Status |
|---|---|---|
| Mainnet | — | not yet deployed |
| Testnet | `TBD` — see [docs/deployment.md](docs/deployment.md) | pending |
| Futurenet | `TBD` — see [docs/deployment.md](docs/deployment.md) | pending |

Contract IDs are recorded per release in [docs/deployment.md](docs/deployment.md)
and [docs/interface-versioning.md](docs/interface-versioning.md).

## I want to…

### …consume prices (DeFi protocols, dApps)

Read the latest price in ten lines with the TypeScript SDK — no signing key
needed, reads are simulated:

```ts
import { Contract, rpc, TransactionBuilder, Networks, Account, nativeToScVal, scValToNative } from "@stellar/stellar-sdk";

const server = new rpc.Server("https://soroban-testnet.stellar.org");
const asset = nativeToScVal("BTC", { type: "symbol" }); // SEP-40 Asset::Other("BTC")
const tx = new TransactionBuilder(new Account("GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7", "0"), { fee: "100", networkPassphrase: Networks.TESTNET })
  .addOperation(new Contract(CONTRACT_ID).call("lastprice", asset)).setTimeout(30).build();
const sim = await server.simulateTransaction(tx);
const data = scValToNative(sim.result.retval); // null when no price exists
console.log(Number(data.price) / 10 ** 14, data.timestamp);
```

- Runnable example: [examples/web/src/lastprice.js](examples/web/src/lastprice.js)
- SDKs: [sdk/](sdk/) (TypeScript, Python, Rust, Go) · [examples/](examples/)
- API reference: [docs/abi.md](docs/abi.md) · SEP-40: [docs/sep40-conformance.md](docs/sep40-conformance.md)
- Errors: [docs/error-codes.md](docs/error-codes.md) · scaling: [docs/price-scaling.md](docs/price-scaling.md)
- Semantics: [docs/aggregation-semantics.md](docs/aggregation-semantics.md) · [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

### …operate a price source

1. Read [docs/source-onboarding.md](docs/source-onboarding.md) and the
   [source lifecycle](docs/source-lifecycle.md).
2. Enroll: `./scripts/oracle-cli.sh add-source --address G... --name "Source A" --admin-key admin`,
   then submit evidence with `submit_source_evidence` (public references only —
   never credentials or private keys).
3. Submit prices with `submit_price`; see the
   [submission bot](docs/price-submission-bot.md) and
   [signed adapters](docs/signed-price-adapters.md).

- Operations: [runbook](docs/runbook.md) · [operators](docs/operators.md) · [key rotation](docs/operations/source-key-rotation.md)
- Reliability: [docs/source-reliability-score.md](docs/source-reliability-score.md) · [docs/source-scorecards.md](docs/source-scorecards.md)
- Deployment record: [docs/deployment.md](docs/deployment.md) · SLA: [docs/SLA.md](docs/SLA.md)

### …contribute

```bash
make build    # cargo build -p price-oracle --target wasm32v1-none --release
make test     # cargo test -p price-oracle --lib
make lint     # clippy -D warnings
make check    # rustfmt --check
```

- Start here: [docs/ONBOARDING.md](docs/ONBOARDING.md) · [CONTRIBUTING.md](CONTRIBUTING.md) · [AGENTS.md](AGENTS.md)
- Find work: [open issues](https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/issues) · [good first issues](https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/labels/good%20first%20issue)
- Layout: `contracts/price-oracle/` (contract + tests), `sdk/`, `examples/`,
  `scripts/`, `services/`, `fuzz/`, `docs/`
- Every PR must pass `make build test lint check` and `make malware-scan`;
  changes to `.github/`, `.husky/`, `.devcontainer/`, `scripts/` and any
  `package.json` need maintainer review.
- Security: [SECURITY.md](SECURITY.md) · [audit checklist](docs/security-audit-checklist.md)

## License

[MIT](LICENSE)

