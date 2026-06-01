// Deploy the dungeon onto the running stack, then record addresses.
//
// Reads the base deployments.json written by up.sh (Sepolia + appchain rpc urls,
// operator + appchain accounts, piltover core, USDC), then:
//
//   1. GAME_TOKEN (plain ERC20)                          on Sepolia
//   2. score world  (init: piltover, GAME_TOKEN, reward) on Sepolia   [sozo]
//   3. game world   (init: score system)                 on appchain  [sozo]
//   4. TokenSale (USDC -> GAME)                           on Sepolia
//   5. Entry (charge GAME + send mint_run to game system) on Sepolia
//   6. grant GAME_TOKEN minter rights to TokenSale + the score system
//
// Order matters: the game world publishes extracted runs to the score system, so
// score must exist first; Entry addresses the appchain game system, so the game
// world must exist before Entry; the sale + score world must be granted minters.

import { cairo } from "starknet";
import { config } from "./config.ts";
import {
  account,
  buildPackage,
  declareAndDeploy,
  invoke,
  loadDeployments,
  migrateWorld,
  saveDeployments,
  waitForRpc,
} from "./lib.ts";

/** Serialize a u256 as [low, high] felt-hex strings for sozo init_call_args. */
function u256Args(v: bigint): string[] {
  const mask = (1n << 128n) - 1n;
  return ["0x" + (v & mask).toString(16), "0x" + (v >> 128n).toString(16)];
}

async function main() {
  const d = loadDeployments();

  console.log("[deploy] waiting for Sepolia + appchain rpc...");
  await Promise.all([waitForRpc(d.settlement.rpcUrl), waitForRpc(d.appchain.rpcUrl)]);

  const operator = account(d.settlement.rpcUrl, d.settlement.account);

  // Build the plain token package so its sierra/casm artifacts exist.
  console.log("[deploy] building token package (scarb)...");
  buildPackage("token");

  // 1. GAME_TOKEN — owner is the operator (it controls minter grants).
  console.log("[deploy] declaring + deploying GAME_TOKEN on Sepolia...");
  const gameToken = await declareAndDeploy(operator, "token", "game_token", {
    owner: d.settlement.account.address,
  });
  d.settlement.gameToken = gameToken;
  saveDeployments(d);
  console.log("  gameToken:", gameToken);

  // 2. score world on Sepolia (consumes settled runs, mints the reward).
  console.log("[deploy] migrating score world on Sepolia (piltover:", d.settlement.piltover, ")");
  const score = migrateWorld({
    pkg: "score",
    seed: "ccd_score2",
    namespace: "score",
    systemTag: "score-score",
    rpcUrl: d.settlement.rpcUrl,
    account: d.settlement.account,
    initArgs: [d.settlement.piltover, gameToken, ...u256Args(config.rewardPerPoint)],
  });
  d.settlement.scoreWorld = score.world;
  d.settlement.scoreSystem = score.system;
  saveDeployments(d);
  console.log("  scoreWorld:", score.world, "scoreSystem:", score.system);

  // 3. game world on the appchain (publishes extracted runs to the score system).
  console.log("[deploy] migrating game world on appchain (registry:", score.system, ")");
  const game = migrateWorld({
    pkg: "game",
    seed: "ccd_game2",
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

  // 4. TokenSale — buy GAME with USDC at the fixed rate; USDC accrues to operator.
  console.log("[deploy] deploying TokenSale on Sepolia (usdc:", d.settlement.usdc, ")");
  const tokenSale = await declareAndDeploy(operator, "token", "token_sale", {
    usdc: d.settlement.usdc,
    game_token: gameToken,
    treasury: d.settlement.account.address,
    rate: cairo.uint256(config.rate),
  });
  d.settlement.tokenSale = tokenSale;
  saveDeployments(d);
  console.log("  tokenSale:", tokenSale);

  // 5. Entry — charge GAME, then send mint_run to the appchain game system.
  console.log("[deploy] deploying Entry on Sepolia (game system:", game.system, ")");
  const entry = await declareAndDeploy(operator, "token", "entry", {
    game_token: gameToken,
    entry_fee: cairo.uint256(config.entryFee),
    sink: d.settlement.account.address,
    piltover: d.settlement.piltover,
    appchain_game: game.system,
  });
  d.settlement.entry = entry;
  saveDeployments(d);
  console.log("  entry:", entry);

  // 6. Grant minter rights: the sale (mint-on-purchase) and the score world
  //    (mint-on-bank) must both be able to mint GAME.
  console.log("[deploy] granting GAME_TOKEN minter rights to TokenSale + score system...");
  await invoke(operator, gameToken, "set_minter", [tokenSale, "0x1"]);
  await invoke(operator, gameToken, "set_minter", [score.system, "0x1"]);

  console.log("[deploy] done.");
}

main().catch((err) => {
  console.error("[deploy] failed:", err);
  process.exit(1);
});
