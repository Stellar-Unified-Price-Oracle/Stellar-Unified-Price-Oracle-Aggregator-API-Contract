import {
  Account,
  Address,
  Contract,
  Keypair,
  nativeToScVal,
  Networks,
  scValToNative,
  SorobanRpc,
  TransactionBuilder,
} from "@stellar/stellar-sdk";
import { Invoker, OracleBindings } from "./generated/bindings";

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

export interface OracleErrorDetail {
  code: number;
  name: string;
  meaning: string;
  remediation: string;
}

export class OracleError extends Error {
  constructor(public readonly detail: OracleErrorDetail) {
    super(`[${detail.name} (#${detail.code})]: ${detail.meaning}`);
    this.name = "OracleError";
  }
}

/** Error-code to typed error mapping (#873). */
export function mapErrorCode(code: number): OracleError {
  const errorMap: Record<number, { name: string; meaning: string; remediation: string }> = {
    0: { name: "NotAuthorized", meaning: "Caller not authorized.", remediation: "Check admin/source signature." },
    1: { name: "AlreadyInitialized", meaning: "Contract already initialized.", remediation: "Do not re-initialize." },
    2: { name: "AssetNotRegistered", meaning: "Asset is not registered.", remediation: "Register asset first." },
    3: { name: "AssetAlreadyRegistered", meaning: "Asset already registered.", remediation: "Treat as success." },
    4: { name: "SourceAlreadyExists", meaning: "Source already exists.", remediation: "Do not duplicate." },
    5: { name: "SourceNotFound", meaning: "Source address not found.", remediation: "Register source first." },
    6: { name: "SourceNotActive", meaning: "Source is inactive.", remediation: "Reactivate source." },
    7: { name: "InvalidPrice", meaning: "Price must be strictly positive.", remediation: "Submit positive price." },
    8: { name: "InvalidTimestamp", meaning: "Timestamp outside acceptable threshold.", remediation: "Use current unix time." },
    9: { name: "InsufficientSources", meaning: "Fewer contributing sources than quorum.", remediation: "Ensure min sources submit." },
    58: { name: "ContractPaused", meaning: "Contract paused.", remediation: "Wait for unpause." },
    168: { name: "SimulationRequired", meaning: "Simulation required.", remediation: "Attach simulation." },
    169: { name: "ProposalHarmful", meaning: "Proposal flagged harmful.", remediation: "Revise proposal." },
  };

  const info = errorMap[code] ?? {
    name: "UnknownError",
    meaning: `Contract error code ${code}`,
    remediation: "Refer to docs/errors/registry.json",
  };

  return new OracleError({ code, ...info });
}

/** Converts raw price integer into a floating-point number using decimal precision (#873). */
export function toDecimal(price: bigint, decimals: number): number {
  return Number(price) / 10 ** decimals;
}

/** Type-safe client for the Stellar Unified Price Oracle contract. */
export class OracleClient {
  private readonly server: SorobanRpc.Server;
  private readonly contract: Contract;
  private readonly networkPassphrase: string;
  /** Generated raw contract bindings (#871). */
  public readonly bindings: OracleBindings;

  constructor(private readonly config: OracleClientConfig) {
    this.server = new SorobanRpc.Server(config.rpcUrl);
    this.contract = new Contract(config.contractId);
    this.networkPassphrase = config.networkPassphrase ?? Networks.TESTNET;

    const invoker: Invoker = {
      invoke: (method: string, args: unknown[]) => this.view(method, args),
    };
    this.bindings = new OracleBindings(invoker);
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

  /** Staleness-guarded price query (#873). */
  async getFreshPrice(asset: string, maxAgeSec: bigint): Promise<PriceEntry | null> {
    const entry = await this.getPrice(asset, maxAgeSec);
    if (!entry) return null;
    const now = BigInt(Math.floor(Date.now() / 1000));
    if (now > entry.timestamp && now - entry.timestamp > maxAgeSec) {
      return null;
    }
    return entry;
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
