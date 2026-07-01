# Relayer Integration

## Overview

This document describes the off-chain relayer network integration for the Stellar Unified Price Oracle. The design is inspired by IBC relayer protocols (Hermes, Egypt) where independent agents relay messages between two parties without being trusted with the underlying data's integrity.

In this context, a **relayer** is an approved off-chain agent that:
1. Collects price observations from oracle sources (off-chain).
2. Bundles source-signed authorization entries into Stellar transactions.
3. Submits those transactions to the contract on behalf of the source.

The key property is that **relayers cannot forge prices**. The oracle source cryptographically signs the exact price data (via Soroban's authorization mechanism) before handing it to the relayer. The contract verifies both signatures before accepting any submission.

---

## Architecture

```
Oracle Source (off-chain)
  │  Signs AuthorizationEntry for submit_price_relayed(relayer, source, asset, price, ts)
  │
  ▼
Relayer Agent (off-chain)
  │  Bundles source's AuthEntry + relayer's own signature into a Stellar transaction
  │
  ▼
Stellar Network
  │  Sends transaction to contract
  │
  ▼
PriceOracleContract.submit_price_relayed()
  │  1. relayer.require_auth()   → verifies relayer signed the tx
  │  2. source.require_auth()    → verifies source's pre-signed AuthEntry
  │  3. is_relayer(relayer)      → verifies admin approved this relayer
  │  4. check_source(source)     → verifies source is registered
  │  5. Validates price & timestamp
  │  6. Stores PriceEntry
  │  7. Emits PriceRelayedEvent
  │  8. Runs aggregation (do_aggregate)
  │  9. Updates relayer fee balance & submission count
```

---

## Relayer Authorization

### Adding a Relayer

Only the contract admin can approve relayers. There is no self-registration.

```
admin → add_relayer(relayer_address, name)
       → stores RelayerInfo { name, approved_at_ledger }
       → emits RelayerAddedEvent
```

**Storage key**: `DataKey::ApprovedRelayer(Address)` → `RelayerInfo`

### Removing a Relayer

```
admin → remove_relayer(relayer_address)
       → removes DataKey::ApprovedRelayer entry
       → emits RelayerRemovedEvent
```

Removal takes effect immediately. Any in-flight transactions from the removed relayer will fail on-chain.

### Querying Relayer Status

| Function | Description |
|----------|-------------|
| `is_relayer(relayer)` | Returns `true` if the address is currently approved |
| `get_relayer_info(relayer)` | Returns `Option<RelayerInfo>` with name and approval ledger |

---

## Price Submission via Relayer

### Function Signature

```rust
submit_price_relayed(
    relayer: Address,   // approved relayer (signs the transaction)
    source:  Address,   // registered oracle source (pre-signs AuthEntry off-chain)
    asset:   Address,   // registered asset contract address
    price:   i128,      // raw price scaled by 10^decimals
    timestamp: u64,     // unix seconds of the price observation
)
```

### Authorization Model

Soroban's host-level authorization system is used to verify both parties:

- **Relayer authorization**: The relayer submits the transaction and signs it with its Stellar keypair. `relayer.require_auth()` verifies this at the host level.
- **Source authorization**: The source creates a Soroban `AuthorizationEntry` off-chain that pre-authorizes the exact invocation `submit_price_relayed(relayer, source, asset, price, timestamp)` on the specific contract. The relayer includes this entry in the transaction alongside its own signature. `source.require_auth()` verifies the pre-signed entry.

This means **the source never needs to be online** when the transaction is submitted. It only needs to be online to generate and sign the authorization entry, which can be done asynchronously.

### Source Signing Flow (Off-Chain)

```
1. Source fetches current price from its data feed.
2. Source constructs the Soroban AuthorizationEntry:
   - contract: oracle_contract_address
   - function: "submit_price_relayed"
   - args: [relayer_address, source_address, asset_address, price, timestamp]
   - valid_until_ledger: current_ledger + N  (expiry window)
3. Source signs the entry with its Stellar keypair.
4. Source sends the signed entry to the relayer (via API, queue, or P2P).
5. Relayer assembles a Stellar transaction containing:
   - The contract invocation
   - The source's AuthorizationEntry
   - The relayer's own signature (as the transaction source account)
6. Relayer submits the transaction to the Stellar network.
```

### Validation Checks

The contract enforces the same validation as direct `submit_price` calls:

| Check | Error |
|-------|-------|
| Contract not paused | `ContractPaused` |
| Relayer is admin-approved | `RelayerNotAuthorized` |
| Source is registered | `SourceNotFound` |
| Asset is registered | `AssetNotRegistered` |
| Source is not suspended | `NotAuthorized` |
| `price > 0` | `InvalidPrice` |
| `price >= asset.min_price` | `PriceBelowMinimum` |
| `timestamp <= ledger_time + threshold` | `InvalidTimestamp` |

### Storage

The relayed price entry is stored under the **same key** as a direct submission:

```
DataKey::Submission(asset, source)  →  PriceEntry { price, timestamp, source, decimals, last_updated }
```

This means relayed and direct submissions are indistinguishable to the aggregation logic. A source can mix direct and relayed submissions freely.

---

## Events

### RelayerAddedEvent

Emitted when the admin approves a new relayer.

| Field | Type | Description |
|-------|------|-------------|
| `relayer` (topic) | `Address` | Approved relayer address |
| `admin` (topic) | `Address` | Admin who approved |
| `name` | `String` | Display name of the relayer |

### RelayerRemovedEvent

Emitted when the admin revokes a relayer's approval.

| Field | Type | Description |
|-------|------|-------------|
| `relayer` (topic) | `Address` | Revoked relayer address |
| `admin` (topic) | `Address` | Admin who revoked |

### PriceRelayedEvent

Emitted for every successful relayed price submission.

| Field | Type | Description |
|-------|------|-------------|
| `asset` (topic) | `Address` | Asset being priced |
| `source` (topic) | `Address` | Oracle source whose data was relayed |
| `relayer` (topic) | `Address` | Relayer that submitted the transaction |
| `price` | `i128` | Raw price value |
| `timestamp` | `u64` | Unix timestamp of the observation |

In addition, the standard `PriceAggregatedEvent` or `SourcesInsufficientEvent` is emitted from the shared aggregation path.

### Relayer Fee Changed Event

Emitted via manual publish (symbol `rfee`) when the admin changes the fee rate.

---

## Relayer Incentives and Fee Structure

### Design Rationale

Relayers incur Stellar transaction fees (XLM) for every submission they make on behalf of sources. Without compensation, relayers have no economic incentive to operate. The fee system provides on-chain accounting that can be used to settle payments off-chain or through a future token-contract integration.

### Fee Configuration

The admin sets a **fee per submission** denominated in stroops (1 stroop = 10⁻⁷ XLM):

```
admin → set_relayer_fee_per_submission(fee: i128)
       → stored at DataKey::RelayerFeePerSubmission
       → emits rfee event
```

Default value: `0` (no fee accrual).

### Fee Accrual

On every successful `submit_price_relayed` call, the fee is credited to the relayer's balance:

```
DataKey::RelayerFeeBalance(relayer) += fee_per_submission
DataKey::RelayerSubmissionCount(relayer) += 1
```

### Reading Fee State

| Function | Returns | Description |
|----------|---------|-------------|
| `get_relayer_fee_per_submission()` | `i128` | Current fee rate in stroops |
| `get_relayer_fee_balance(relayer)` | `i128` | Accumulated fees owed to this relayer |
| `get_relayer_submission_count(relayer)` | `u64` | Total successful submissions by this relayer |

### Settlement

Fee settlement is currently tracked on-chain but **not automatically disbursed**. Options for settlement:

1. **Off-chain settlement**: The operator reads `get_relayer_fee_balance`, computes the owed amount, and sends XLM directly to the relayer via a separate Stellar transaction.
2. **Token contract integration** (future): A separate settlement contract can read the fee balances and distribute a configured token (XLM or custom) to relayers in batch.

This design keeps the oracle contract lightweight and free of cross-contract calls during the critical price submission path.

---

## Error Codes

| Code | Name | Description |
|------|------|-------------|
| `16` | `RelayerNotAuthorized` | The caller is not an approved relayer, or a `remove_relayer` target is not found |
| `17` | `RelayerAlreadyExists` | `add_relayer` called for an address already approved |

---

## Storage Keys

| Key | Type | Description |
|-----|------|-------------|
| `ApprovedRelayer(Address)` | `RelayerInfo` | Metadata for each approved relayer |
| `RelayerFeePerSubmission` | `i128` | Current fee per relayed submission |
| `RelayerFeeBalance(Address)` | `i128` | Accumulated fee balance per relayer |
| `RelayerSubmissionCount(Address)` | `u64` | Submission count per relayer |

All keys use persistent storage with standard TTL extension (threshold: 1000 ledgers, bump: 4000 ledgers).

---

## Security Considerations

### Signature Forgery

A relayer **cannot** forge a source's price submission. The Soroban host verifies the source's `AuthorizationEntry` signature cryptographically before `source.require_auth()` returns. If the source did not sign this exact invocation with these exact arguments, the transaction will be rejected at the host level.

### Replay Protection

Soroban `AuthorizationEntry` entries include a `valid_until_ledger` field. Sources should set a tight expiry (e.g., current ledger + 100) to limit the window during which a relayer can replay the submission. The contract's timestamp threshold (`set_timestamp_threshold`) provides additional protection: stale price data is rejected even if the authorization is still technically valid.

### Relayer Key Compromise

If a relayer's key is compromised, the attacker can submit transactions signed as the relayer — but cannot generate valid source authorization entries. They can only relay existing (already signed) price data. The admin can immediately revoke the relayer via `remove_relayer` to stop further submissions.

### Admin Centralization

Relayer approval is fully admin-controlled. This is intentional for the current design: the admin is responsible for vetting relayer operators before granting them approval. A future upgrade could introduce a decentralized relayer registry governed by the timelock mechanism.

### Fee Manipulation

The fee rate is admin-only and cannot be changed by relayers. Fee balances only increase via successful price submissions, so there is no griefing vector for inflating fee balances.

---

## Integration Guide

### Deploying the Contract

No additional initialization is required for the relayer system. After `initialize()`, the admin can immediately call `add_relayer`.

### Setting Up a Relayer

```bash
# 1. Admin approves the relayer
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source <ADMIN_KEY> \
  -- add_relayer \
  --relayer <RELAYER_ADDRESS> \
  --name "my-hermes-relayer"

# 2. (Optional) Set a fee per submission
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source <ADMIN_KEY> \
  -- set_relayer_fee_per_submission \
  --fee 1000
```

### Running the Relayer Agent

The relayer should implement the following loop:

```
loop:
  1. For each configured (source, asset) pair:
     a. Collect latest price from the source's data feed.
     b. Request a signed AuthorizationEntry from the source.
     c. Build a Stellar transaction invoking submit_price_relayed.
     d. Attach the source's AuthEntry to the transaction.
     e. Sign the transaction with the relayer's keypair.
     f. Submit to Stellar Horizon.
  2. Sleep until the next submission interval.
```

### Querying Fee Balance

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source <ANY_KEY> \
  -- get_relayer_fee_balance \
  --relayer <RELAYER_ADDRESS>
```
