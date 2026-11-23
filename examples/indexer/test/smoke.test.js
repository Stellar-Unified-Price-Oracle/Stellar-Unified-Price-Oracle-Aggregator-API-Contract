import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { nativeToScVal } from "@stellar/stellar-sdk";
import { poll } from "../src/indexer.js";
import { startTelemetry } from "../src/telemetry.js";

test("pages through events and decodes them", async () => {
  const ev = { ledger: 10, id: "1", topic: [nativeToScVal("price_updated", { type: "symbol" })], value: nativeToScVal(5n, { type: "i128" }) };
  const pages = [{ events: [ev], cursor: "a" }, { events: [], cursor: "b" }];
  const rows = [];
  const cursor = await poll({ getEvents: async () => pages.shift() }, "C", 1, (r) => rows.push(r));
  assert.equal(cursor, "b");
  assert.deepEqual(rows, [{ ledger: 10, id: "1", topics: ["price_updated"], value: 5n }]);
});

test("exports event spans and propagates trace context", async () => {
  let exported = 0;
  const receiver = createServer((req, res) => {
    req.resume();
    req.on("end", () => {
      exported += 1;
      res.writeHead(200).end();
    });
  });
  await new Promise((resolve) => receiver.listen(0, "127.0.0.1", resolve));
  const previousEndpoint = process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
  process.env.OTEL_EXPORTER_OTLP_ENDPOINT =
    `http://127.0.0.1:${receiver.address().port}/v1/traces`;
  const stopTelemetry = await startTelemetry();
  try {
    const ev = {
      ledger: 10,
      id: "1",
      topic: [nativeToScVal("price_updated", { type: "symbol" })],
      value: nativeToScVal(5n, { type: "i128" }),
    };
    const pages = [{ events: [ev], cursor: "a" }, { events: [], cursor: "b" }];
    const rows = [];
    await poll({ getEvents: async () => pages.shift() }, "C", 1, (row) => rows.push(row));
    assert.match(rows[0].traceparent, /^00-[0-9a-f]{32}-[0-9a-f]{16}-01$/);
  } finally {
    await stopTelemetry();
    if (previousEndpoint === undefined) delete process.env.OTEL_EXPORTER_OTLP_ENDPOINT;
    else process.env.OTEL_EXPORTER_OTLP_ENDPOINT = previousEndpoint;
    await new Promise((resolve) => receiver.close(resolve));
  }
  assert.ok(exported > 0);
});
