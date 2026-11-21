# TypeScript dApp Consumer Quickstart (5-Minute Guide)

This quickstart demonstrates how to query the Stellar Unified Price Oracle from a TypeScript/JavaScript frontend or dApp using `@stellar/stellar-sdk`.

## Quick Example

```typescript
import { rpc, Networks } from "@stellar/stellar-sdk";
import { fetchPrice } from "./src/index.js";

const server = new rpc.Server("https://soroban-testnet.stellar.org");
const price = await fetchPrice(server, CONTRACT_ID, ASSET_ADDRESS, Networks.TESTNET);
console.log("Current Price:", price);
```

## Running Tests

```bash
npm install
npm test
```
