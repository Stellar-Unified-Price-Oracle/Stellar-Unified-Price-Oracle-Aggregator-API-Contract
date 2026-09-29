# Stellar Unified Price Oracle — SDKs

Type-safe client libraries for interacting with the oracle contract, covering price
queries, price submission, subscription management, and source/asset registry lookups.
Both SDKs share the same auth model (a Stellar `Keypair`/identity signs write calls;
reads are simulated and unsigned) and JSON-friendly serialization of contract types.

The core surface below is intentionally the high-traffic subset of the contract's
endpoints; the contract itself exposes many more admin/governance functions (see
`contracts/price-oracle/src/lib.rs`) that can be wrapped the same way as needed.

## TypeScript

```ts
import { OracleClient } from "@stellar-oracle/sdk";
import { Keypair } from "@stellar/stellar-sdk";

const client = new OracleClient({
  contractId: "CXXXX...",
  rpcUrl: "https://soroban-testnet.stellar.org",
});

const price = await client.getPrice("CBTC...", 600n);

const signer = Keypair.fromSecret(process.env.SOURCE_SECRET_KEY!);
await client.submitPrice("CSOURCE...", "CBTC...", 6500000000000000n, BigInt(Math.floor(Date.now() / 1000)), signer);
```

Build: `cd sdk/typescript && npm install && npm run build`

## Python

```python
from oracle_sdk import OracleClient, OracleClientConfig
from stellar_sdk import Keypair

client = OracleClient(OracleClientConfig(
    contract_id="CXXXX...",
    rpc_url="https://soroban-testnet.stellar.org",
))

price = client.get_price("CBTC...", max_age=600)

signer = Keypair.from_secret(os.environ["SOURCE_SECRET_KEY"])
client.submit_price("CSOURCE...", "CBTC...", price=6500000000000000, timestamp=int(time.time()), signer=signer)
```

Install: `cd sdk/python && pip install -e .`
