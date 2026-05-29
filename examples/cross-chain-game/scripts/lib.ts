// Shared helpers for the cross-chain game store demo deploy scripts.
//
// Both Katana instances are started with `--dev --dev.no-fee` using the default
// seed ("0"), so they share the same deterministic predeployed accounts. We use
// account #0 (verified from the Katana startup banner) to sign on both nodes.

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Account, RpcProvider, type CompiledSierra, type CompiledSierraCasm } from "starknet";

const __dirname = dirname(fileURLToPath(import.meta.url));

/** Demo root: examples/cross-chain-game */
export const DEMO_ROOT = resolve(__dirname, "..");
/** Repo root: the katana checkout. */
export const REPO_ROOT = resolve(DEMO_ROOT, "..", "..");

export const SETTLEMENT_RPC = "http://localhost:5050";
export const APPCHAIN_RPC = "http://localhost:5051";

// Deterministic predeployed account #0 (seed "0"), shared by both dev nodes.
export const DEV_ACCOUNT_ADDRESS =
  "0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec";
export const DEV_ACCOUNT_PRIVATE_KEY =
  "0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912";

// Prebuilt piltover messaging mock — acts as the settlement-layer messaging
// contract that emits `MessageSent` events.
export const MOCK_SIERRA_PATH = resolve(
  REPO_ROOT,
  "crates/contracts/build/piltover_messaging_mock.contract_class.json",
);
export const MOCK_CASM_PATH = resolve(
  REPO_ROOT,
  "crates/contracts/build/piltover_messaging_mock.compiled_contract_class.json",
);

// The appchain game_minter contract, compiled by `scarb build` in ./cairo.
export const GAME_SIERRA_PATH = resolve(
  DEMO_ROOT,
  "cairo/target/dev/cross_chain_game_game_minter.contract_class.json",
);
export const GAME_CASM_PATH = resolve(
  DEMO_ROOT,
  "cairo/target/dev/cross_chain_game_game_minter.compiled_contract_class.json",
);

// The frontend imports this file directly via Vite.
export const DEPLOYMENTS_PATH = resolve(DEMO_ROOT, "app/src/deployments.json");

export type Deployments = {
  account: { address: string; privateKey: string };
  settlement: { rpcUrl: string; messagingContract?: string };
  appchain: { rpcUrl: string; gameContract?: string };
};

export function loadJson<T>(path: string): T {
  return JSON.parse(readFileSync(path, "utf-8")) as T;
}

export function loadDeployments(): Deployments {
  if (existsSync(DEPLOYMENTS_PATH)) {
    return loadJson<Deployments>(DEPLOYMENTS_PATH);
  }
  return {
    account: { address: DEV_ACCOUNT_ADDRESS, privateKey: DEV_ACCOUNT_PRIVATE_KEY },
    settlement: { rpcUrl: SETTLEMENT_RPC },
    appchain: { rpcUrl: APPCHAIN_RPC },
  };
}

export function saveDeployments(d: Deployments): void {
  writeFileSync(DEPLOYMENTS_PATH, JSON.stringify(d, null, 2) + "\n");
}

export function account(rpcUrl: string): Account {
  const provider = new RpcProvider({ nodeUrl: rpcUrl });
  // starknet.js v8 takes a single options object; v3 transactions are the default.
  return new Account({
    provider,
    address: DEV_ACCOUNT_ADDRESS,
    signer: DEV_ACCOUNT_PRIVATE_KEY,
    cairoVersion: "1",
  });
}

/** Declare (if needed) and deploy a contract, returning the deployed address. */
export async function declareAndDeploy(
  acct: Account,
  sierraPath: string,
  casmPath: string,
  constructorCalldata: string[] = [],
): Promise<string> {
  const contract = loadJson<CompiledSierra>(sierraPath);
  const casm = loadJson<CompiledSierraCasm>(casmPath);

  const { deploy } = await acct.declareAndDeploy({ contract, casm, constructorCalldata });
  return deploy.contract_address;
}

/** Poll an RPC endpoint until it responds (node is up). */
export async function waitForRpc(rpcUrl: string, timeoutMs = 30_000): Promise<void> {
  const provider = new RpcProvider({ nodeUrl: rpcUrl });
  const start = Date.now();
  for (;;) {
    try {
      await provider.getChainId();
      return;
    } catch {
      if (Date.now() - start > timeoutMs) {
        throw new Error(`RPC at ${rpcUrl} did not come up within ${timeoutMs}ms`);
      }
      await new Promise((r) => setTimeout(r, 500));
    }
  }
}
