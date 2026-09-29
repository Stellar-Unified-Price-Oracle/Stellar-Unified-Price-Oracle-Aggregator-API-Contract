import { Contract, SorobanRpc, TransactionBuilder, Networks, Keypair, Address, Account, nativeToScVal, scValToNative } from "@stellar/stellar-sdk";

/** Sequence number is irrelevant for a simulated (unsubmitted) transaction. */
const SIMULATION_SOURCE = new Account(Keypair.random().publicKey(), "0");

export interface OracleClientConfig {
  contractId: string;
  rpcUrl: string;
  networkPassphrase?: string;
}

export interface PriceEntry {
  price: bigint;
  timestamp: bigint;
}

/** Type-safe client for the Stellar Unified Price Oracle contract. */
export class OracleClient {
  private readonly server: SorobanRpc.Server;
  private readonly contract: Contract;
  private readonly networkPassphrase: string;

  constructor(private readonly config: OracleClientConfig) {
    this.server = new SorobanRpc.Server(config.rpcUrl);
    this.contract = new Contract(config.contractId);
    this.networkPassphrase = config.networkPassphrase ?? Networks.TESTNET;
  }

  /** Sign and submit a transaction invoking `method` with `args`, using `signer` for auth. */
  private async invoke(method: string, args: unknown[], signer: Keypair) {
    const account = await this.server.getAccount(signer.publicKey());
    const scArgs = args.map((a) => nativeToScVal(a, {}));
    const tx = new TransactionBuilder(account, {
      fee: "100000",
      networkPassphrase: this.networkPassphrase,
    })
      .addOperation(this.contract.call(method, ...scArgs))
      .setTimeout(30)
      .build();

    const prepared = await this.server.prepareTransaction(tx);
    prepared.sign(signer);
    const sendResult = await this.server.sendTransaction(prepared);

    let status = sendResult.status;
    let hash = sendResult.hash;
    for (let i = 0; i < 10 && status === "PENDING"; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      const res = await this.server.getTransaction(hash);
      status = res.status as typeof status;
      if (status === "SUCCESS") return res.returnValue ? scValToNative(res.returnValue) : undefined;
    }
    if (status !== "SUCCESS") throw new Error(`Transaction ${hash} failed with status ${status}`);
  }

  /** Read-only simulation call — no signature or fee required. */
  private async view(method: string, args: unknown[]) {
    const scArgs = args.map((a) => nativeToScVal(a, {}));
    const tx = new TransactionBuilder(SIMULATION_SOURCE, {
      fee: "100",
      networkPassphrase: this.networkPassphrase,
    })
      .addOperation(this.contract.call(method, ...scArgs))
      .setTimeout(30)
      .build();
    const sim = await this.server.simulateTransaction(tx);
    if (SorobanRpc.Api.isSimulationError(sim)) throw new Error(sim.error);
    return sim.result?.retval ? scValToNative(sim.result.retval) : undefined;
  }

  // ── Price queries ────────────────────────────────────────────────────
  async getPrice(asset: string, maxAge: bigint): Promise<PriceEntry | null> {
    return this.view("get_price", [Address.fromString(asset), maxAge]);
  }

  async getSourcePrice(asset: string, source: string): Promise<PriceEntry> {
    return this.view("get_source_price", [Address.fromString(asset), Address.fromString(source)]);
  }

  async getAllPrices(asset: string): Promise<PriceEntry[]> {
    return this.view("get_all_prices", [Address.fromString(asset)]);
  }

  // ── Submission ───────────────────────────────────────────────────────
  async submitPrice(source: string, asset: string, price: bigint, timestamp: bigint, signer: Keypair) {
    return this.invoke("submit_price", [Address.fromString(source), Address.fromString(asset), price, timestamp], signer);
  }

  // ── Subscription management ─────────────────────────────────────────
  async subscribe(consumer: string, duration: number, signer: Keypair) {
    return this.invoke("subscribe", [Address.fromString(consumer), duration], signer);
  }

  async renewSubscription(consumer: string, signer: Keypair) {
    return this.invoke("renew_subscription", [Address.fromString(consumer)], signer);
  }

  async getSubscriptionExpiry(consumer: string): Promise<bigint> {
    return this.view("get_subscription_expiry", [Address.fromString(consumer)]);
  }

  // ── Source / asset registry ─────────────────────────────────────────
  async isSource(source: string): Promise<boolean> {
    return this.view("is_source", [Address.fromString(source)]);
  }

  async isAssetRegistered(asset: string): Promise<boolean> {
    return this.view("is_asset_registered", [Address.fromString(asset)]);
  }
}
