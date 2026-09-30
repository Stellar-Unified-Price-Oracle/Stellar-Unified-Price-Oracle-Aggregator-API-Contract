// SEP-40 consumer: read `lastprice` + `decimals` via simulation (read-only, no signing key needed).
import { Contract, rpc, TransactionBuilder, Networks, Account, nativeToScVal, xdr, scValToNative } from "@stellar/stellar-sdk";

// SEP-40 Asset::Other(Symbol) encoded as a Soroban enum (vec [Symbol("Other"), Symbol(sym)]).
export const otherAsset = (sym) => xdr.ScVal.scvVec([nativeToScVal("Other", { type: "symbol" }), nativeToScVal(sym, { type: "symbol" })]);

// Simulation needs a source account but no signature; a zero-sequence placeholder is fine.
const READ_ONLY_SOURCE = "GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7";

export function buildCall(contractId, method, args = [], networkPassphrase = Networks.TESTNET) {
  return new TransactionBuilder(new Account(READ_ONLY_SOURCE, "0"), { fee: "100", networkPassphrase })
    .addOperation(new Contract(contractId).call(method, ...args))
    .setTimeout(30)
    .build();
}

export async function lastprice(server, contractId, symbol, networkPassphrase) {
  const read = async (method, args) => {
    const sim = await server.simulateTransaction(buildCall(contractId, method, args, networkPassphrase));
    if (rpc.Api.isSimulationError(sim)) throw new Error(sim.error); // map codes via docs/errors/registry.json
    return scValToNative(sim.result.retval);
  };
  const [data, decimals] = [await read("lastprice", [otherAsset(symbol)]), await read("decimals", [])];
  if (!data) return null; // SEP-40: None when no price exists
  return { price: Number(data.price) / 10 ** decimals, timestamp: Number(data.timestamp) };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const { RPC_URL = "https://soroban-testnet.stellar.org", CONTRACT_ID, ASSET = "BTC" } = process.env;
  if (!CONTRACT_ID) throw new Error("set CONTRACT_ID");
  console.log(await lastprice(new rpc.Server(RPC_URL), CONTRACT_ID, ASSET, Networks.TESTNET));
}
