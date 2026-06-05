// Declare the Cartridge Controller account classes on the appchain.
//
// Why this exists (katana #584 genesis gap): `katana init rollup` round-trips
// genesis.json on disk, shifting the embedded controller class hash, so the canonical
// class the keychain deploys isn't reliably present after boot. Without it the
// Controller can't auto-deploy on the appchain and its play actions fail.
//
// Why ALL versions: a Controller account is pinned to the class *version* it was
// created with (e.g. v1.0.8), and the keychain deploys it at that version on first
// execute on a new chain. We don't know which version a given user's account uses, so
// declare every bundled version — any account can then auto-deploy on the appchain.
// Idempotent (declareIfNot). Remove once #584 declares these in genesis.

import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import { account, DEMO_ROOT, loadDeployments, loadJson, provider, waitForRpc } from "./lib.ts";

// The controller account artifacts embedded in katana (katana-slot-controller).
const CLASSES_DIR = resolve(
  DEMO_ROOT,
  "../../crates/contracts/contracts/controller/account_sdk/artifacts/classes",
);

async function main() {
  const { appchain } = loadDeployments();
  await waitForRpc(appchain.rpcUrl);

  const acc = account(appchain.rpcUrl, appchain.account);
  const prov = provider(appchain.rpcUrl);

  const versions = readdirSync(CLASSES_DIR)
    .filter((f) => f.startsWith("controller.") && f.endsWith(".contract_class.json"))
    .map((f) => f.replace(".contract_class.json", ""));

  for (const v of versions) {
    const contract = loadJson(resolve(CLASSES_DIR, `${v}.contract_class.json`));
    const casm = loadJson(resolve(CLASSES_DIR, `${v}.compiled_contract_class.json`));
    const res = await acc.declareIfNot({ contract, casm });
    if (res.transaction_hash) await prov.waitForTransaction(res.transaction_hash);
    console.log(`[controller-class] ${v} -> ${res.class_hash}`);
  }
}

main().catch((e) => {
  console.error("[controller-class] declare failed:", e?.message ?? e);
  process.exit(1);
});
