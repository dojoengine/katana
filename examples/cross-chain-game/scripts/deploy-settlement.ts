// Deploy the messaging contract on the settlement ("L1") Katana node.
//
// This is the piltover messaging mock. Its `send_message_to_appchain` entrypoint
// emits the `MessageSent` event that the appchain Katana consumes. The appchain
// node must be started with `--settlement.core-contract <this address>`.

import {
  SETTLEMENT_RPC,
  MOCK_SIERRA_PATH,
  MOCK_CASM_PATH,
  account,
  declareAndDeploy,
  loadDeployments,
  saveDeployments,
  waitForRpc,
} from "./lib.ts";

async function main() {
  console.log(`[settlement] waiting for node at ${SETTLEMENT_RPC} ...`);
  await waitForRpc(SETTLEMENT_RPC);

  const acct = account(SETTLEMENT_RPC);

  console.log("[settlement] declaring + deploying messaging mock ...");
  // Constructor: cancellation_delay_secs (u64). 0 is fine for the demo.
  const address = await declareAndDeploy(acct, MOCK_SIERRA_PATH, MOCK_CASM_PATH, ["0"]);

  const deployments = loadDeployments();
  deployments.settlement = { rpcUrl: SETTLEMENT_RPC, messagingContract: address };
  saveDeployments(deployments);

  console.log(`[settlement] messaging contract deployed: ${address}`);
}

main().catch((err) => {
  console.error("[settlement] deploy failed:", err);
  process.exit(1);
});
