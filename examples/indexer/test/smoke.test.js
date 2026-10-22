import { test } from "node:test";
import assert from "node:assert/strict";
import { nativeToScVal } from "@stellar/stellar-sdk";
import { poll } from "../src/indexer.js";

test("pages through events and decodes them", async () => {
  const ev = { ledger: 10, id: "1", topic: [nativeToScVal("price_updated", { type: "symbol" })], value: nativeToScVal(5n, { type: "i128" }) };
  const pages = [{ events: [ev], cursor: "a" }, { events: [], cursor: "b" }];
  const rows = [];
  const cursor = await poll({ getEvents: async () => pages.shift() }, "C", 1, (r) => rows.push(r));
  assert.equal(cursor, "b");
  assert.deepEqual(rows, [{ ledger: 10, id: "1", topics: ["price_updated"], value: 5n }]);
});
