// Deploy the demo contracts onto the running stack, then record their addresses.
//
// Reads the base `deployments.json` written by `up.sh` (rpc urls, accounts, the
// piltover core address) and deploys:
//   - score_registry  on the settlement layer (ctor: piltover address)
//   - game_minter     on the appchain
//   - achievements    on the appchain (ctor: score_registry address)
//
// Order matters: achievements addresses its L2 -> L1 messages to score_registry,
// so the registry must exist first.

import {
  ARTIFACT,
  account,
  declareAndDeploy,
  loadDeployments,
  saveDeployments,
  waitForRpc,
} from "./lib.ts";

async function main() {
  const d = loadDeployments();

  console.log("[deploy] waiting for both nodes...");
  await Promise.all([waitForRpc(d.settlement.rpcUrl), waitForRpc(d.appchain.rpcUrl)]);

  const settlement = account(d.settlement.rpcUrl, d.settlement.account);
  const appchain = account(d.appchain.rpcUrl, d.appchain.account);

  console.log("[deploy] score_registry on settlement (piltover:", d.settlement.piltover, ")");
  d.settlement.scoreRegistry = await declareAndDeploy(settlement, ARTIFACT.scoreRegistry, [
    d.settlement.piltover,
  ]);
  saveDeployments(d);
  console.log("  scoreRegistry:", d.settlement.scoreRegistry);

  console.log("[deploy] game_minter on appchain");
  d.appchain.gameMinter = await declareAndDeploy(appchain, ARTIFACT.gameMinter, []);
  saveDeployments(d);
  console.log("  gameMinter:", d.appchain.gameMinter);

  console.log("[deploy] achievements on appchain (registry:", d.settlement.scoreRegistry, ")");
  d.appchain.achievements = await declareAndDeploy(appchain, ARTIFACT.achievements, [
    d.settlement.scoreRegistry,
  ]);
  saveDeployments(d);
  console.log("  achievements:", d.appchain.achievements);

  console.log("[deploy] done.");
}

main().catch((err) => {
  console.error("[deploy] failed:", err);
  process.exit(1);
});
