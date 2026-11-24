// Polls contract events via RPC `getEvents` and decodes them into plain JSON rows.
import { rpc, scValToNative } from "@stellar/stellar-sdk";
import { context, propagation, SpanStatusCode, trace } from "@opentelemetry/api";

export const decode = (ev) => ({
  ledger: ev.ledger,
  id: ev.id,
  topics: ev.topic.map((t) => scValToNative(t)),
  value: scValToNative(ev.value),
});

export async function poll(server, contractId, startLedger, onRow) {
  const tracer = trace.getTracer("oracle-event-indexer");
  let cursor;
  for (;;) {
    const req = cursor ? { cursor, filters: [{ type: "contract", contractIds: [contractId] }], limit: 100 }
      : { startLedger, filters: [{ type: "contract", contractIds: [contractId] }], limit: 100 };
    const res = await tracer.startActiveSpan("stellar.rpc.getEvents", {
      attributes: { "oracle.contract.id": contractId, "oracle.cursor": cursor ?? String(startLedger) },
    }, async (span) => {
      try {
        return await server.getEvents(req);
      } catch (error) {
        span.recordException(error);
        span.setStatus({ code: SpanStatusCode.ERROR });
        throw error;
      } finally {
        span.end();
      }
    });
    for (const event of res.events) {
      const row = await tracer.startActiveSpan("oracle.indexer.event", {
        attributes: {
          "oracle.contract.id": contractId,
          "oracle.round": event.ledger,
        },
      }, async (span) => {
        try {
          const decoded = decode(event);
          const value = decoded.value;
          if (value && typeof value === "object" && value.asset !== undefined) {
            span.setAttribute("oracle.asset", String(value.asset));
          }
          const carrier = {};
          propagation.inject(context.active(), carrier);
          return carrier.traceparent ? { ...decoded, traceparent: carrier.traceparent } : decoded;
        } catch (error) {
          span.recordException(error);
          span.setStatus({ code: SpanStatusCode.ERROR });
          throw error;
        } finally {
          span.end();
        }
      });
      onRow(row);
    }
    if (!res.events.length) return res.cursor;
    cursor = res.cursor;
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const { RPC_URL = "https://soroban-testnet.stellar.org", CONTRACT_ID, START_LEDGER } = process.env;
  if (!CONTRACT_ID) throw new Error("set CONTRACT_ID");
  const { startTelemetry } = await import("./telemetry.js");
  const shutdownTelemetry = await startTelemetry();
  const server = new rpc.Server(RPC_URL);
  const start = Number(START_LEDGER ?? (await server.getLatestLedger()).sequence - 1000);
  try {
    await poll(server, CONTRACT_ID, start, (row) => console.log(JSON.stringify(row, (_, v) => (typeof v === "bigint" ? v.toString() : v))));
  } finally {
    await shutdownTelemetry();
  }
}
