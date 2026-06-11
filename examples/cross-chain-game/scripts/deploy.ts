// Migrate the two Dojo worlds onto the running stack, then record addresses.
//
// Reads the base `deployments.json` written by `up.sh` (rpc urls, accounts,
// piltover core, torii urls) and migrates two worlds with `sozo`:
//   - score world  on the settlement layer (init: piltover address)
//   - game world   on the appchain        (init: score system address)
//
// Order matters: the game world publishes its L2 -> L1 scores to the score
// system, so the score world must exist first to supply its address.

import { loadDeployments, migrateWorld, saveDeployments, waitForRpc } from "./lib.ts";

async function main() {
  const d = loadDeployments();

  console.log("[deploy] waiting for both nodes...");
  await Promise.all([waitForRpc(d.settlement.rpcUrl), waitForRpc(d.appchain.rpcUrl)]);

  console.log("[deploy] migrating score world on settlement (piltover:", d.settlement.piltover, ")");
  const score = migrateWorld({
    pkg: "score",
    seed: "ccg_score",
    namespace: "score",
    systemTag: "score-score_registry",
    rpcUrl: d.settlement.rpcUrl,
    account: d.settlement.account,
    initArgs: [d.settlement.piltover],
  });
  d.settlement.scoreWorld = score.world;
  d.settlement.scoreSystem = score.system;
  saveDeployments(d);
  console.log("  scoreWorld:", score.world, "scoreSystem:", score.system);

  console.log("[deploy] migrating game world on appchain (registry:", score.system, ")");
  const game = migrateWorld({
    pkg: "game",
    seed: "ccg_game",
    namespace: "game",
    systemTag: "game-game",
    rpcUrl: d.appchain.rpcUrl,
    account: d.appchain.account,
    initArgs: [score.system],
  });
  d.appchain.gameWorld = game.world;
  d.appchain.gameSystem = game.system;
  saveDeployments(d);
  console.log("  gameWorld:", game.world, "gameSystem:", game.system);

  // Store world: the L1 storefront whose buy_game sends the L1->L2 mint message.
  // It needs the piltover core (to send) and the game system (to address). The
  // game system only exists after the migration above, so store goes last.
  console.log("[deploy] migrating store world on settlement (game:", game.system, ")");
  const store = migrateWorld({
    pkg: "store",
    seed: "ccg_store",
    namespace: "store",
    systemTag: "store-store",
    rpcUrl: d.settlement.rpcUrl,
    account: d.settlement.account,
    initArgs: [d.settlement.piltover, game.system],
  });
  d.settlement.storeWorld = store.world;
  d.settlement.storeSystem = store.system;
  saveDeployments(d);
  console.log("  storeWorld:", store.world, "storeSystem:", store.system);

  console.log("[deploy] done.");
}

main().catch((err) => {
  console.error("[deploy] failed:", err);
  process.exit(1);
});
