# Oracle Source Onboarding Guide

This guide walks new oracle source providers through everything needed to integrate with the Stellar Unified Price Oracle Aggregator contract.

## What Is an Oracle Source?

An oracle source is a permissioned off-chain service (or on-chain account) that submits price data to the aggregator contract. The contract collects submissions from all registered sources, computes the **median**, and exposes the aggregated price to consumers. Multiple independent sources improve robustness against manipulation and single-point failure.

### Responsibilities

- Submit accurate, timely prices for every asset you support
- Maintain high uptime — missed submissions cause `InsufficientSources` errors for consumers
- Secure your signing key; it is the sole authorization mechanism for your submissions
- Follow the submission frequency expected by the contract operator

---

## Technical Requirements

| Requirement | Detail |
|---|---|
| Stellar account | A funded Stellar account (your source address) |
| Admin registration | The contract admin must call `add_source(your_address, "YourName")` |
| Price feed | A reliable upstream data feed (CEX API, aggregated data provider, etc.) |
| Submission tooling | Rust SDK, JS SDK (`@stellar/stellar-sdk`), or soroban-cli |
| Network access | HTTPS to a Stellar RPC endpoint (Testnet or Mainnet) |

---

## Step-by-Step Setup

### 1. Generate or designate a Stellar account

```bash
# Using soroban-cli
soroban keys generate --global my-oracle-source --network testnet
soroban keys address my-oracle-source
```

Fund the account on testnet:

```bash
soroban keys fund my-oracle-source --network testnet
```

### 2. Share your address with the contract admin

Send your Stellar public key to the contract admin. They will register you by calling:

```bash
soroban contract invoke \
  --id <CONTRACT_ID> \
  --source <ADMIN_KEY> \
  --network testnet \
  -- add_source \
  --address <YOUR_ADDRESS> \
  --name "MyOracleName"
```

### 3. Verify registration

```bash
soroban contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- is_source \
  --address <YOUR_ADDRESS>
# returns: true
```

### 4. Submit your first price

`submit_price` signature:

```
submit_price(source: Address, asset: String, price: i128, timestamp: u64)
```

- `source` — your registered address (must sign the transaction)
- `asset` — asset symbol string, e.g. `"BTC"`, `"ETH"`, `"XLM"`
- `price` — integer price scaled by `10^decimals` (check `get_decimals()`)
- `timestamp` — Unix timestamp in seconds

#### soroban-cli

```bash
soroban contract invoke \
  --id <CONTRACT_ID> \
  --source my-oracle-source \
  --network testnet \
  -- submit_price \
  --source <YOUR_ADDRESS> \
  --asset "BTC" \
  --price 6500000000000 \
  --timestamp $(date +%s)
```

#### Rust (soroban-sdk client)

```rust
use stellar_sdk::{Keypair, Network, Server, TransactionBuilder};

// Build and sign the submit_price invocation transaction
// using soroban-rs or stellar-sdk Rust bindings.
// See: https://docs.rs/soroban-sdk
```

#### JavaScript (@stellar/stellar-sdk)

```js
import { Contract, SorobanRpc, TransactionBuilder, Networks, Keypair, nativeToScVal, xdr } from "@stellar/stellar-sdk";

const rpc = new SorobanRpc.Server("https://soroban-testnet.stellar.org");
const sourceKeypair = Keypair.fromSecret("S...");
const contractId = "C...";

const contract = new Contract(contractId);
const account = await rpc.getAccount(sourceKeypair.publicKey());

const decimals = 7; // confirm with get_decimals()
const priceRaw = BigInt(Math.round(65000.0 * 10 ** decimals)); // BTC = $65,000

const tx = new TransactionBuilder(account, {
  fee: "100",
  networkPassphrase: Networks.TESTNET,
})
  .addOperation(
    contract.call(
      "submit_price",
      nativeToScVal(sourceKeypair.publicKey(), { type: "address" }),
      nativeToScVal("BTC",  { type: "string" }),
      nativeToScVal(priceRaw, { type: "i128" }),
      nativeToScVal(BigInt(Math.floor(Date.now() / 1000)), { type: "u64" })
    )
  )
  .setTimeout(30)
  .build();

const prepared = await rpc.prepareTransaction(tx);
prepared.sign(sourceKeypair);
const result = await rpc.sendTransaction(prepared);
console.log("Submitted:", result.hash);
```

#### Python (stellar-sdk)

```python
from stellar_sdk import Keypair, Network, SorobanServer, TransactionBuilder
from stellar_sdk.soroban_rpc import SendTransactionStatus
import time

server = SorobanServer("https://soroban-testnet.stellar.org")
source_keypair = Keypair.from_secret("S...")
contract_id = "C..."

account = server.load_account(source_keypair.public_key)
decimals = 7
price_raw = int(65000.0 * 10**decimals)

tx = (
    TransactionBuilder(account, network_passphrase=Network.TESTNET_NETWORK_PASSPHRASE, base_fee=100)
    .append_invoke_contract_function_op(
        contract_id=contract_id,
        function_name="submit_price",
        parameters=[
            # source, asset, price, timestamp
            source_keypair.public_key,
            "BTC",
            price_raw,
            int(time.time()),
        ],
    )
    .set_timeout(30)
    .build()
)

tx = server.prepare_transaction(tx)
tx.sign(source_keypair)
response = server.send_transaction(tx)
print("Status:", response.status)
```

---

## Best Practices

- **Submit at a consistent interval.** Match or exceed the staleness window expected by consumers. Every 60 seconds is a safe default.
- **Validate your price before submitting.** Reject outliers (>5% deviation from your own recent median) before sending.
- **Use a dedicated signing key.** Never reuse your source key for any other purpose.
- **Monitor your own submissions.** Track `oracle_price_submissions_total` for your source address.
- **Handle RPC errors with exponential backoff.** Transient network failures should not cause a burst of duplicate submissions.
- **Keep timestamp accurate.** Use an NTP-synced clock. Stale timestamps may confuse consumers using `resolution()`.
- **Test on testnet first.** Deploy your submission service on Stellar testnet before switching to mainnet.

---

## Troubleshooting

| Problem | Likely cause | Fix |
|---|---|---|
| `NotAuthorized` (error 0) | Your address is not registered, or you didn't sign | Confirm registration with `is_source()`; ensure your key signs the tx |
| `SourceNotFound` (error 5) | Admin removed your source | Contact the contract admin |
| `InvalidPrice` (error 7) | You submitted price ≤ 0 | Validate price before submission |
| `AssetNotRegistered` (error 2) | The asset string isn't registered | Call `assets()` to list valid assets |
| Transaction fails with `fee too low` | Network congestion | Increase the base fee to 1000+ stroops |
| RPC timeout | Node overloaded | Retry with exponential backoff; switch RPC endpoint |
| Price never shows as aggregated | `InsufficientSources` — not enough sources submitted | Other sources may be offline; confirm `get_min_sources_required()` |

---

## Useful Contract Queries

```bash
# List all registered sources
soroban contract invoke --id <CONTRACT_ID> --network testnet -- get_oracle_sources

# Check decimals (scale factor for prices)
soroban contract invoke --id <CONTRACT_ID> --network testnet -- get_decimals

# Check minimum sources required for aggregation
soroban contract invoke --id <CONTRACT_ID> --network testnet -- get_min_sources_required

# Get the current aggregated price for an asset
soroban contract invoke --id <CONTRACT_ID> --network testnet -- get_price --asset "BTC"

# Get your latest submitted price
soroban contract invoke --id <CONTRACT_ID> --network testnet \
  -- get_source_price --asset "BTC" --source <YOUR_ADDRESS>
```

---

## Further Reading

- [Architecture Overview](./ARCHITECTURE.md)
- [Monitoring Setup Guide](./monitoring-setup.md)
- [Gas Usage Reference](./gas-usage.md)
- [SEP-40 Standard](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0040.md)
