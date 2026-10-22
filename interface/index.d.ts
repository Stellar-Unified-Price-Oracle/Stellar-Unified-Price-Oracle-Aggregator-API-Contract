export interface FunctionSpec { params: string[]; returns: string; deprecated?: { since: string; removal: string; replacement: string; guide: string } }
export interface ErrorEntry { code: number; name: string; class: "config" | "caller" | "transient" | "state"; meaning: string; cause: string; remediation: string }
export const interface: { name: string; version: string; contract: string; functions: Record<string, FunctionSpec> };
export const errors: ErrorEntry[];
export const versions: Record<string, { contract_version: string; wasm_hash: string | null; networks: Record<string, string> }>;
export function errorByCode(code: number): ErrorEntry | undefined;
