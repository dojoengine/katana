// Deploy the game_minter contract on the appchain ("L2") Katana node.
//
// Its `mint_game` #[l1_handler] is invoked by Katana's messaging service when a
// message relayed from the settlement layer targets this contract.

import {
  APPCHAIN_RPC,
  GAME_SIERRA_PATH,
  GAME_CASM_PATH,
  account,
  declareAndDeploy,
  loadDeployments,
  saveDeployments,
  waitForRpc,
} from "./lib.ts";

async function main() {
  console.log(`[appchain] waiting for node at ${APPCHAIN_RPC} ...`);
  await waitForRpc(APPCHAIN_RPC);

  const acct = account(APPCHAIN_RPC);

  console.log("[appchain] declaring + deploying game_minter ...");
  // No constructor arguments.
  const address = await declareAndDeploy(acct, GAME_SIERRA_PATH, GAME_CASM_PATH, []);

  const deployments = loadDeployments();
  deployments.appchain = { rpcUrl: APPCHAIN_RPC, gameContract: address };
  saveDeployments(deployments);

  console.log(`[appchain] game_minter deployed: ${address}`);
}

main().catch((err) => {
  console.error("[appchain] deploy failed:", err);
  process.exit(1);
});
