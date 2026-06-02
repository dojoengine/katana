// Deploy the dungeon onto the running stack, then record addresses.
//
// Reads the base deployments.json written by up.sh (Sepolia + appchain rpc urls,
// operator + appchain accounts, piltover core, USDC), then:
//
//   1. GAME token  (entry credit, plain ERC20)           on Sepolia
//   2. GOLD token  (winnings, plain ERC20)               on Sepolia
//   3. bank world  (init: piltover, GOLD, reward_per_gold) on Sepolia [sozo]
//   4. game world  (init: bank system)                   on appchain  [sozo]
//   5. TokenSale (USDC -> GAME)                           on Sepolia
//   6. Entry (charge GAME + send mint_run to game system) on Sepolia
//   7. grant GAME minter -> TokenSale; GOLD minter -> the bank system
//
// Order matters: the game world withdraws to the bank system, so bank must exist
// first; Entry addresses the appchain game system, so the game world must exist
// before Entry; the sale (GAME) and the bank (GOLD) must each be granted as minters.

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

  // 1. GAME (entry credit) — owner is the operator (it controls minter grants).
  console.log("[deploy] declaring + deploying GAME (entry credit) on Sepolia...");
  const gameToken = await declareAndDeploy(operator, "token", "game_token", {
    owner: d.settlement.account.address,
  });
  d.settlement.gameToken = gameToken;
  saveDeployments(d);
  console.log("  gameToken:", gameToken);

  // 2. GOLD (winnings) — minted on L1 when a player banks; owner is the operator.
  console.log("[deploy] declaring + deploying GOLD (winnings) on Sepolia...");
  const goldToken = await declareAndDeploy(operator, "token", "gold_token", {
    owner: d.settlement.account.address,
  });
  d.settlement.goldToken = goldToken;
  saveDeployments(d);
  console.log("  goldToken:", goldToken);

  // 3. bank world on Sepolia (consumes the settled withdrawal, mints GOLD).
  console.log("[deploy] migrating bank world on Sepolia (piltover:", d.settlement.piltover, ")");
  const bank = migrateWorld({
    pkg: "score",
    // Derive the seed from the piltover address so each fresh chain gets a fresh
    // bank world. The bank world lives on persistent settlement (Sepolia/mainnet);
    // a fixed seed would reuse the same world across FRESH redeploys, and sozo
    // `migrate` upgrades it in place WITHOUT re-running `dojo_init` — leaving the
    // stored piltover/gold_token pointing at the previous deploy. A piltover-keyed
    // seed guarantees a new world (fresh `dojo_init`) whenever the chain changes.
    seed: `ccd_bank_${d.settlement.piltover.slice(2, 12)}`,
    namespace: "bank",
    systemTag: "bank-bank",
    rpcUrl: d.settlement.rpcUrl,
    account: d.settlement.account,
    initArgs: [d.settlement.piltover, goldToken, ...u256Args(config.rewardPerGold)],
  });
  d.settlement.bankWorld = bank.world;
  d.settlement.bankSystem = bank.system;
  saveDeployments(d);
  console.log("  bankWorld:", bank.world, "bankSystem:", bank.system);

  // 4. game world on the appchain (withdraws the vault to the bank system).
  console.log("[deploy] migrating game world on appchain (registry:", bank.system, ")");
  const game = migrateWorld({
    pkg: "game",
    seed: "ccd_game3",
    namespace: "game",
    systemTag: "game-game",
    rpcUrl: d.appchain.rpcUrl,
    account: d.appchain.account,
    initArgs: [bank.system],
  });
  d.appchain.gameWorld = game.world;
  d.appchain.gameSystem = game.system;
  saveDeployments(d);
  console.log("  gameWorld:", game.world, "gameSystem:", game.system);

  // 5. TokenSale — buy GAME with USDC at the fixed rate; USDC accrues to operator.
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

  // 6. Entry — charge GAME, then send mint_run to the appchain game system.
  console.log("[deploy] deploying Entry on Sepolia (game system:", game.system, ")");
  const entry = await declareAndDeploy(operator, "token", "entry", {
    game_token: gameToken,
    entry_fee: cairo.uint256(config.entryFee),
    piltover: d.settlement.piltover,
    appchain_game: game.system,
  });
  d.settlement.entry = entry;
  saveDeployments(d);
  console.log("  entry:", entry);

  // 7. Grant minter rights: the sale mints GAME on purchase; the bank world mints
  //    GOLD on settlement. Two tokens, two distinct minters.
  console.log("[deploy] granting GAME minter -> TokenSale, GOLD minter -> bank system...");
  await invoke(operator, gameToken, "set_minter", [tokenSale, "0x1"]);
  await invoke(operator, goldToken, "set_minter", [bank.system, "0x1"]);

  console.log("[deploy] done.");
}

main().catch((err) => {
  console.error("[deploy] failed:", err);
  process.exit(1);
});
