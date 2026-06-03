// Declare the Cartridge Controller account class on the appchain.
//
// Why this exists (katana #584 genesis gap): `katana init rollup` round-trips
// genesis.json on disk, and that round-trip shifts the embedded controller class
// hash. #584 declares the preloaded genesis classes via a real declare tx, but it
// declares the *round-tripped* artifact, which hashes to a shifted value — not the
// canonical hash the hosted keychain actually deploys. So after a fresh boot the
// canonical class isn't on the appchain, and the Controller can't auto-deploy there.
//
// Declaring the on-disk `controller.latest` artifact here lands the canonical hash,
// so a Controller logging in via x.cartridge.gg can deploy + sign on the appchain.
// Idempotent (declareIfNot). Remove once #584 declares the canonical class in genesis.

import { resolve } from "node:path";
import { Account, RpcProvider, json } from "starknet";
import { DEMO_ROOT, loadDeployments, loadJson, waitForRpc } from "./lib.ts";

// The controller account artifacts embedded in katana (katana-slot-controller).
const CLASSES_DIR = resolve(
  DEMO_ROOT,
  "../../crates/contracts/contracts/controller/account_sdk/artifacts/classes",
);
// The canonical class hash the hosted keychain deploys; assert we land it.
const CANONICAL_HASH = "0x743c83c41ce99ad470aa308823f417b2141e02e04571f5c0004e743556e7faf";

async function main() {
  const { appchain } = loadDeployments();
  await waitForRpc(appchain.rpcUrl);

  const provider = new RpcProvider({ nodeUrl: appchain.rpcUrl });
  const account = new Account({
    provider,
    address: appchain.account.address,
    signer: appchain.account.privateKey,
    cairoVersion: "1",
  });

  const contract = loadJson(resolve(CLASSES_DIR, "controller.latest.contract_class.json"));
  const casm = loadJson(resolve(CLASSES_DIR, "controller.latest.compiled_contract_class.json"));

  const res = await account.declareIfNot({ contract, casm });
  if (res.transaction_hash) await provider.waitForTransaction(res.transaction_hash);

  console.log(`[controller-class] declared on appchain: ${res.class_hash}`);
  if (res.class_hash !== CANONICAL_HASH) {
    console.warn(
      `[controller-class] WARNING: expected canonical ${CANONICAL_HASH}; the keychain` +
        ` deploys that hash, so a mismatch means the Controller deploy will fail.`,
    );
  }
}

main().catch((e) => {
  console.error("[controller-class] declare failed:", e?.message ?? e);
  process.exit(1);
});
