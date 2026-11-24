import {
  Account,
  Contract,
  Networks,
  rpc,
  scValToNative,
  TransactionBuilder,
  xdr,
} from "@stellar/stellar-sdk";

export interface FormattedPrice {
  price: number;
  rawPrice: bigint;
  decimals: number;
  timestamp: number;
}

const READ_ONLY_SOURCE = "GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7";

export function createSep40Asset(address: string): xdr.ScVal {
  return xdr.ScVal.scvVec([
    xdr.ScVal.scvSymbol("Stellar"),
    xdr.ScVal.scvAddress(xdr.Address.account(xdr.PublicKey.publicKeyTypeEd25519(Buffer.alloc(32)))),
  ]);
}

export function formatPrice(rawPrice: bigint, decimals: number, timestamp: number): FormattedPrice {
  return {
    price: Number(rawPrice) / 10 ** decimals,
    rawPrice,
    decimals,
    timestamp,
  };
}

export async function fetchPrice(
  server: rpc.Server,
  contractId: string,
  assetAddress: string,
  networkPassphrase = Networks.TESTNET
): Promise<FormattedPrice | null> {
  const account = new Account(READ_ONLY_SOURCE, "0");
  const contract = new Contract(contractId);

  // Read decimals
  const decimalsTx = new TransactionBuilder(account, { fee: "100", networkPassphrase })
    .addOperation(contract.call("decimals"))
    .setTimeout(30)
    .build();

  const decimalsSim = await server.simulateTransaction(decimalsTx);
  if (rpc.Api.isSimulationError(decimalsSim)) {
    throw new Error(`Simulation failed: ${decimalsSim.error}`);
  }
  const decimals = Number(scValToNative(decimalsSim.result.retval));

  // Read lastprice
  const priceTx = new TransactionBuilder(account, { fee: "100", networkPassphrase })
    .addOperation(contract.call("lastprice", createSep40Asset(assetAddress)))
    .setTimeout(30)
    .build();

  const priceSim = await server.simulateTransaction(priceTx);
  if (rpc.Api.isSimulationError(priceSim)) {
    throw new Error(`Simulation failed: ${priceSim.error}`);
  }
  const data = scValToNative(priceSim.result.retval);
  if (!data) return null;

  return formatPrice(BigInt(data.price), decimals, Number(data.timestamp));
}
