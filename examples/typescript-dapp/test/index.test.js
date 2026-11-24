import test from "node:test";
import assert from "node:assert/strict";
import { formatPrice } from "../src/index.js";

test("formatPrice correctly scales raw price with decimals", () => {
  const result = formatPrice(6500000000000n, 7, 1700000000);
  assert.equal(result.price, 650000);
  assert.equal(result.rawPrice, 6500000000000n);
  assert.equal(result.decimals, 7);
  assert.equal(result.timestamp, 1700000000);
});
