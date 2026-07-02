// Shared helpers for the cross-chain arcade deploy + verify steps.
//
// Everything here talks to the two Katana nodes over plain starknet.js — no
// sozo, no Dojo, no Torii. Two chains:
//   - settlement ("L1"): a plain `--dev` Katana hosting the piltover core
//     contract (deployed by `katana init rollup`) + the arcade contract.
//   - appchain   ("L2"): a Katana booted from the generated rollup chain config,
//     hosting the N machine contracts (each an insert_coin l1_handler).
// `up.sh` deploys the piltover core (via `katana init rollup`) and writes the
// base `deployments.json` (rpc urls, accounts, piltover). These scripts fill in
// the machine + arcade addresses.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  Account,
  CallData,
  RpcProvider,
  shortString,
  type BigNumberish,
  type CairoAssembly,
  type CompiledSierra,
  type RawArgs,
} from "starknet";

const __dirname = dirname(fileURLToPath(import.meta.url));

export const DEMO_ROOT = resolve(__dirname, "..");
export const REPO_ROOT = resolve(DEMO_ROOT, "../..");
export const CAIRO_TARGET = resolve(DEMO_ROOT, "cairo/target/dev");
export const DEPLOYMENTS_PATH = resolve(DEMO_ROOT, "app/src/deployments.json");

export const SETTLEMENT_RPC = "http://localhost:5050";
export const APPCHAIN_RPC = "http://localhost:5051";

// The machines deployed on the appchain. Names are short-string felts.
export const MACHINE_NAMES = ["SLOTS", "PINBALL", "CLAW", "RACER"] as const;

export type Keypair = { address: string; privateKey: string };
export type MachineInfo = { name: string; address: string };
export type Deployments = {
  settlement: {
    rpcUrl: string;
    explorer: string;
    account: Keypair; // plain --dev account 0
    piltover?: string;
    arcade?: string;
  };
  appchain: {
    rpcUrl: string;
    explorer: string;
    account: Keypair; // generated rollup genesis account
    machines?: MachineInfo[];
  };
};

export function provider(url: string): RpcProvider {
  return new RpcProvider({ nodeUrl: url });
}

export function account(p: RpcProvider, kp: Keypair): Account {
  return new Account({ provider: p, address: kp.address, signer: kp.privateKey });
}

export function nameToFelt(name: string): string {
  return shortString.encodeShortString(name);
}

export function feltToName(felt: BigNumberish): string {
  return shortString.decodeShortString(BigInt(felt).toString());
}

export function loadDeployments(): Deployments {
  if (!existsSync(DEPLOYMENTS_PATH)) {
    throw new Error(`deployments.json not found at ${DEPLOYMENTS_PATH} — run up.sh (it writes it)`);
  }
  return JSON.parse(readFileSync(DEPLOYMENTS_PATH, "utf-8")) as Deployments;
}

export function saveDeployments(d: Deployments): void {
  writeFileSync(DEPLOYMENTS_PATH, JSON.stringify(d, null, 2) + "\n");
}

function readArtifact(sierraPath: string, casmPath: string): {
  sierra: CompiledSierra;
  casm: CairoAssembly;
} {
  const sierra = JSON.parse(readFileSync(sierraPath, "utf-8")) as CompiledSierra;
  const casm = JSON.parse(readFileSync(casmPath, "utf-8")) as CairoAssembly;
  return { sierra, casm };
}

/** Declare a class if needed and deploy one instance via the UDC. */
export async function declareAndDeploy(
  acct: Account,
  sierraPath: string,
  casmPath: string,
  constructorCalldata: RawArgs,
  salt: string,
): Promise<{ classHash: string; address: string }> {
  const { sierra, casm } = readArtifact(sierraPath, casmPath);

  const declare = await acct.declareIfNot({ contract: sierra, casm });
  if (declare.transaction_hash) {
    await acct.waitForTransaction(declare.transaction_hash);
  }
  const classHash = declare.class_hash;

  const deploy = await acct.deployContract({
    classHash,
    constructorCalldata: CallData.compile(constructorCalldata),
    salt,
    unique: false,
  });
  await acct.waitForTransaction(deploy.transaction_hash);

  return { classHash, address: deploy.contract_address };
}

/** Call a view entrypoint and return the raw felt result array. */
export async function call(
  p: RpcProvider,
  contractAddress: string,
  entrypoint: string,
  calldata: BigNumberish[] = [],
): Promise<string[]> {
  return p.callContract({
    contractAddress,
    entrypoint,
    calldata: CallData.compile(calldata),
  });
}

export async function waitForRpc(url: string, timeoutMs = 60_000): Promise<void> {
  const p = provider(url);
  const start = Date.now();
  for (;;) {
    try {
      await p.getChainId();
      return;
    } catch {
      // not up yet
    }
    if (Date.now() - start > timeoutMs) throw new Error(`RPC ${url} not up in ${timeoutMs}ms`);
    await sleep(500);
  }
}

export const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function scarbBuild(): void {
  execFileSync("scarb", ["build"], {
    cwd: resolve(DEMO_ROOT, "cairo"),
    stdio: ["ignore", "inherit", "inherit"],
  });
}
