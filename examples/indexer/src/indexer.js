// Polls contract events via RPC `getEvents` and decodes them into plain JSON rows.
import { rpc, scValToNative } from "@stellar/stellar-sdk";

export const decode = (ev) => ({
  ledger: ev.ledger,
  id: ev.id,
  topics: ev.topic.map((t) => scValToNative(t)),
  value: scValToNative(ev.value),
});

export async function poll(server, contractId, startLedger, onRow) {
  let cursor;
  for (;;) {
    const req = cursor ? { cursor, filters: [{ type: "contract", contractIds: [contractId] }], limit: 100 }
      : { startLedger, filters: [{ type: "contract", contractIds: [contractId] }], limit: 100 };
    const res = await server.getEvents(req);
    res.events.forEach((e) => onRow(decode(e)));
    if (!res.events.length) return res.cursor;
    cursor = res.cursor;
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const { RPC_URL = "https://soroban-testnet.stellar.org", CONTRACT_ID, START_LEDGER } = process.env;
  if (!CONTRACT_ID) throw new Error("set CONTRACT_ID");
  const server = new rpc.Server(RPC_URL);
  const start = Number(START_LEDGER ?? (await server.getLatestLedger()).sequence - 1000);
  await poll(server, CONTRACT_ID, start, (row) => console.log(JSON.stringify(row, (_, v) => (typeof v === "bigint" ? v.toString() : v))));
}
