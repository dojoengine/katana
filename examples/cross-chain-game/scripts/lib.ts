// Shared helpers for the cross-chain game store deploy step.
//
// The unified demo runs a settlement Katana ("L1", SN_SEPOLIA) and a rollup
// appchain Katana ("L2") that settles to a piltover core contract via saya-tee.
// `up.sh` deploys the piltover core (via `katana init rollup`) and writes the
// base `deployments.json` (rpc urls, accounts, piltover). This deploy step then
// declares + deploys the demo contracts and fills in their addresses.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Account, RpcProvider, type CompiledSierra, type CompiledSierraCasm } from "starknet";

const __dirname = dirname(fileURLToPath(import.meta.url));

export const DEMO_ROOT = resolve(__dirname, "..");
const CAIRO_DIR = resolve(DEMO_ROOT, "cairo/target/dev");

export const DEPLOYMENTS_PATH = resolve(DEMO_ROOT, "app/src/deployments.json");

export type Keypair = { address: string; privateKey: string };
export type Deployments = {
  settlement: {
    rpcUrl: string;
    explorer: string;
    account: Keypair;
    piltover: string;
    scoreRegistry?: string;
  };
  appchain: {
    rpcUrl: string;
    explorer: string;
    account: Keypair;
    game?: string;
  };
};

/** Artifact base names (under cairo/target/dev). */
export const ARTIFACT = {
  game: "cross_chain_game_game",
  scoreRegistry: "cross_chain_game_score_registry",
} as const;

export function loadJson<T>(path: string): T {
  return JSON.parse(readFileSync(path, "utf-8")) as T;
}

export function loadDeployments(): Deployments {
  return loadJson<Deployments>(DEPLOYMENTS_PATH);
}

export function saveDeployments(d: Deployments): void {
  writeFileSync(DEPLOYMENTS_PATH, JSON.stringify(d, null, 2) + "\n");
}

export function account(rpcUrl: string, kp: Keypair): Account {
  const provider = new RpcProvider({ nodeUrl: rpcUrl });
  return new Account({ provider, address: kp.address, signer: kp.privateKey, cairoVersion: "1" });
}

/** Declare (if needed) and deploy a contract by artifact name; returns its address. */
export async function declareAndDeploy(
  acct: Account,
  artifactName: string,
  constructorCalldata: string[] = [],
): Promise<string> {
  const contract = loadJson<CompiledSierra>(resolve(CAIRO_DIR, `${artifactName}.contract_class.json`));
  const casm = loadJson<CompiledSierraCasm>(
    resolve(CAIRO_DIR, `${artifactName}.compiled_contract_class.json`),
  );
  const { deploy } = await acct.declareAndDeploy({ contract, casm, constructorCalldata });
  return deploy.contract_address;
}

export async function waitForRpc(rpcUrl: string, timeoutMs = 30_000): Promise<void> {
  const provider = new RpcProvider({ nodeUrl: rpcUrl });
  const start = Date.now();
  for (;;) {
    try {
      await provider.getChainId();
      return;
    } catch {
      if (Date.now() - start > timeoutMs) throw new Error(`RPC ${rpcUrl} not up in ${timeoutMs}ms`);
      await new Promise((r) => setTimeout(r, 500));
    }
  }
}
