import { test } from "node:test";
import assert from "node:assert/strict";
import { StrKey, xdr, nativeToScVal } from "@stellar/stellar-sdk";
import { buildCall, otherAsset, lastprice } from "../src/lastprice.js";

const CONTRACT = StrKey.encodeContract(Buffer.alloc(32, 1));

test("builds a SEP-40 lastprice invocation offline", () => {
  const tx = buildCall(CONTRACT, "lastprice", [otherAsset("BTC")]);
  const op = tx.operations[0];
  assert.equal(op.type, "invokeHostFunction");
  assert.equal(op.func.invokeContract().functionName().toString(), "lastprice");
});

test("scales price by decimals and handles None end to end", async () => {
  const retvals = { lastprice: xdr.ScVal.scvMap([
      new xdr.ScMapEntry({ key: nativeToScVal("price", { type: "symbol" }), val: nativeToScVal(12345000000n, { type: "i128" }) }),
      new xdr.ScMapEntry({ key: nativeToScVal("timestamp", { type: "symbol" }), val: nativeToScVal(1700000000n, { type: "u64" }) })]),
    decimals: nativeToScVal(7, { type: "u32" }) };
  const fake = (rv) => ({ simulateTransaction: async (tx) => ({ result: { retval: rv[tx.operations[0].func.invokeContract().functionName().toString()] } }) });
  assert.deepEqual(await lastprice(fake(retvals), CONTRACT, "BTC"), { price: 1234.5, timestamp: 1700000000 });
  assert.equal(await lastprice(fake({ ...retvals, lastprice: xdr.ScVal.scvVoid() }), CONTRACT, "BTC"), null);
});
