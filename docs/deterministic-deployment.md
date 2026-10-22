# Deterministic Deployment

Deterministic deployment lets you calculate a contract's address **before** it is deployed to the network. The address is derived from three inputs: the deployer's address, the uploaded WASM hash, and a 32-byte salt you choose. Given the same three inputs, the address is always identical — on any network, at any time.

## What Is Deterministic Deployment?

In Soroban, every contract address is computed as a hash of:

```
contract_address = hash(deployer_address || wasm_hash || salt)
```

This is equivalent to Ethereum's `CREATE2` opcode. The `stellar contract deploy` command accepts a `--salt` flag that feeds this formula, and `stellar contract id wasm` computes the result without actually deploying anything.

Because the formula is deterministic, you can:

- Calculate the address offline or on a different machine.
- Hard-code the address in consumer contracts before the oracle is live.
- Re-deploy to the exact same address after a factory reset (same deployer, same salt, same WASM).
- Publish the address in governance proposals or audits days before deployment.

## Why It Matters

| Benefit | Detail |
|---|---|
| Pre-configure consumers | Consumer contracts can be coded with a hard-coded oracle address and deployed before the oracle itself. |
| Auditability | A governance vote can approve a specific `(wasm_hash, salt)` pair, then anyone can verify that the deployed address matches. |
| Reproducibility | Staging and production environments can share the same address if the same deployer key and salt are used on each network. |
| Disaster recovery | After a catastrophic state loss, the same contract address can be restored by redeploying with the same inputs. |

## Prerequisites

- **`stellar` CLI** installed and configured (`stellar --version` should print a version string).
- A funded **deployer identity** on the target network (`stellar keys ls` to list identities).
- A compiled **WASM file** at `target/wasm32v1-none/release/price_oracle.wasm` (run `make build` first).

## How to Generate a Salt

A salt is a 64-character hex string representing 32 random bytes. Generate one with any of these:

```bash
# Using openssl (most systems)
openssl rand -hex 32

# Using /dev/urandom directly
xxd -l 32 -p /dev/urandom | tr -d '\n'

# Using Python
python3 -c "import secrets; print(secrets.token_hex(32))"
```

Example output:
```
a3f1e2d4b5c6789012345678abcdef0123456789abcdef0123456789abcdef01
```

Store the salt alongside the deployment record — it is required to reproduce or verify the address later.

## How to Pre-compute the Address Without Deploying

Pass `--dry-run` to the script. It uploads the WASM, computes the expected address, prints it, and exits without deploying:

```bash
./scripts/deterministic-deploy.sh \
  --deployer my-admin \
  --network testnet \
  --salt a3f1e2d4b5c6789012345678abcdef0123456789abcdef0123456789abcdef01 \
  --wasm target/wasm32v1-none/release/price_oracle.wasm \
  --admin GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF \
  --dry-run
```

Expected output (address will differ based on your inputs):

```
=====================================================
 Stellar Deterministic Oracle Deployment
=====================================================
 Deployer : my-admin
 Network  : testnet
 WASM     : target/wasm32v1-none/release/price_oracle.wasm
 Salt     : a3f1e2d4b5c6789012345678abcdef0123456789abcdef0123456789abcdef01
 Admin    : GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF
 Dry run  : true
=====================================================

[1/4] Uploading WASM to get hash...
      WASM hash: abc123...

[2/4] Pre-computing contract address...
      Expected address: CXXX...

[DRY RUN] Would deploy to: CXXX...
[DRY RUN] No deployment performed.
```

You can also call the Stellar CLI directly to compute the address, if you already have the WASM hash:

```bash
stellar contract id wasm \
  --wasm-hash <wasm-hash> \
  --salt <64-char-hex> \
  --source <deployer-identity> \
  --network testnet
```

## How to Deploy

Run the script without `--dry-run`:

```bash
./scripts/deterministic-deploy.sh \
  --deployer my-admin \
  --network testnet \
  --salt a3f1e2d4b5c6789012345678abcdef0123456789abcdef0123456789abcdef01 \
  --wasm target/wasm32v1-none/release/price_oracle.wasm \
  --admin GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF \
  --decimals 18 \
  --description "My Production Oracle"
```

The script performs four steps:

1. **Upload WASM** — `stellar contract upload` submits the binary and returns its ledger hash. If the WASM was already uploaded (idempotent), the same hash is returned.
2. **Pre-compute address** — `stellar contract id wasm` calculates the deterministic address from `(deployer, wasm_hash, salt)`.
3. **Deploy** — `stellar contract deploy --wasm-hash ... --salt ...` creates the contract on-chain.
4. **Verify** — the script compares the pre-computed address to the address returned by the deploy command. If they differ, the script exits with an error.

After a successful deployment, the script prints the initialization command you should run next.

## How to Verify After Deployment

Anyone can independently verify a deployed contract's address by running:

```bash
stellar contract id wasm \
  --wasm-hash <wasm-hash> \
  --salt <salt> \
  --source <original-deployer-address> \
  --network testnet
```

If the output matches the contract ID visible in Stellar Explorer, the deployment was deterministic and used the claimed inputs.

To retrieve the WASM hash of a live contract:

```bash
stellar contract info --id <contract-address> --network testnet
```

## Full Example

```bash
# 1. Build the contract
make build

# 2. Generate a salt and save it
SALT=$(openssl rand -hex 32)
echo "Salt: $SALT"

# 3. Dry-run to see the address before spending fees
./scripts/deterministic-deploy.sh \
  --deployer alice \
  --network testnet \
  --salt "$SALT" \
  --wasm target/wasm32v1-none/release/price_oracle.wasm \
  --admin GBBB...XXXX \
  --dry-run

# 4. Record the expected address, hard-code it in consumer contracts, etc.

# 5. Deploy for real
./scripts/deterministic-deploy.sh \
  --deployer alice \
  --network testnet \
  --salt "$SALT" \
  --wasm target/wasm32v1-none/release/price_oracle.wasm \
  --admin GBBB...XXXX \
  --decimals 18 \
  --description "Testnet Oracle v1"

# 6. Initialize (command printed at the end of the deploy script output)
stellar contract invoke \
  --id CXXX...deployed-address \
  --source alice \
  --network testnet \
  -- initialize \
  --admin GBBB...XXXX \
  --min_sources_required 1 \
  --max_history_length 100 \
  --decimals 18 \
  --description "Testnet Oracle v1"
```

## Regular Deploy vs Deterministic Deploy

| Aspect | Regular deploy | Deterministic deploy |
|---|---|---|
| Command | `stellar contract deploy --wasm ...` | `stellar contract deploy --wasm-hash ... --salt ...` |
| Address known before deploy | No | Yes |
| Address reproducible | No | Yes (same deployer + wasm + salt) |
| Consumer contracts can be pre-configured | No | Yes |
| Auditable before deployment | No | Yes (wasm_hash + salt published in advance) |
| Re-deployable to same address | No | Yes |
| Extra steps | Upload WASM is implicit | Upload WASM separately to get hash |

## Troubleshooting

### Address mismatch between dry-run and actual deploy

The three inputs must be identical for both calls:

| Input | What can go wrong |
|---|---|
| Deployer address | Using a different identity key, or a key alias that resolves to a different address on a different machine. Always use the same named identity or the same raw address. |
| WASM hash | Rebuilding the contract between dry-run and deploy changes the binary and therefore the hash. Run dry-run and deploy in the same session without rebuilding. |
| Salt | A typo, leading/trailing whitespace, or upper/lowercase difference in the hex string. The script enforces exactly 64 hex characters; double-check the value you pass. |

### "WASM file not found"

Run `make build` first. The output path is `target/wasm32v1-none/release/price_oracle.wasm`.

### "stellar: command not found"

Install the Stellar CLI. See the [official installation guide](https://developers.stellar.org/docs/tools/developer-tools/cli/install-cli).

### Upload succeeds but deploy fails with "already exists"

The contract at that address was already deployed (same deployer, same salt, same WASM). Use a different salt if you need a new instance, or call `initialize` on the existing address if this is a re-run of the same deployment.

### Deployment on mainnet

The script works identically on mainnet. Pass `--network mainnet`. Ensure the deployer account holds enough XLM to cover transaction fees and the contract storage rent reserve.
