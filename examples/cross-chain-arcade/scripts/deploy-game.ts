// Deploy the game contracts, once both nodes are up and the piltover core exists
// (deployed by `katana init rollup` in up.sh):
//   1. N `machine` contracts on the appchain (same class, distinct salts -> the
//      distinct target addresses whose fan-out this demo is about).
//   2. the `arcade` dispenser on the settlement chain, wired with the piltover
//      core + every machine address.

import { resolve } from "node:path";
import {
  account,
  APPCHAIN_RPC,
  CAIRO_TARGET,
  declareAndDeploy,
  loadDeployments,
  MACHINE_NAMES,
  nameToFelt,
  provider,
  saveDeployments,
  scarbBuild,
  SETTLEMENT_RPC,
  waitForRpc,
  type MachineInfo,
} from "./lib.ts";

const MACHINE_SIERRA = resolve(CAIRO_TARGET, "arcade_machine.contract_class.json");
const MACHINE_CASM = resolve(CAIRO_TARGET, "arcade_machine.compiled_contract_class.json");
const ARCADE_SIERRA = resolve(CAIRO_TARGET, "arcade_arcade.contract_class.json");
const ARCADE_CASM = resolve(CAIRO_TARGET, "arcade_arcade.compiled_contract_class.json");

async function main() {
  console.log("[deploy] building cairo contracts...");
  scarbBuild();

  console.log("[deploy] waiting for both nodes...");
  await Promise.all([waitForRpc(SETTLEMENT_RPC), waitForRpc(APPCHAIN_RPC)]);

  const d = loadDeployments();
  if (!d.settlement.piltover) throw new Error("piltover core missing — run `katana init rollup`");

  // 1. Machines on the appchain — one deploy per name, distinct salt => distinct
  //    address. These distinct addresses are exactly what makes the fan-out a
  //    multi-target message batch (the PR #623 repro).
  const appchain = account(provider(APPCHAIN_RPC), d.appchain.account);
  const machines: MachineInfo[] = [];
  for (let i = 0; i < MACHINE_NAMES.length; i++) {
    const name = MACHINE_NAMES[i];
    console.log(`[deploy] machine ${name} (#${i}) on appchain...`);
    const { address } = await declareAndDeploy(
      appchain,
      MACHINE_SIERRA,
      MACHINE_CASM,
      [nameToFelt(name)],
      "0x" + (i + 1).toString(16),
    );
    machines.push({ name, address });
    console.log(`  ${name} = ${address}`);
  }
  d.appchain.machines = machines;
  saveDeployments(d);

  // 2. Arcade dispenser on the settlement chain, wired to the core + machines.
  console.log("[deploy] arcade dispenser on settlement...");
  const settlement = account(provider(SETTLEMENT_RPC), d.settlement.account);
  const { address: arcade } = await declareAndDeploy(
    settlement,
    ARCADE_SIERRA,
    ARCADE_CASM,
    [d.settlement.piltover, machines.map((m) => m.address)],
    "0x1",
  );
  d.settlement.arcade = arcade;
  saveDeployments(d);
  console.log("[deploy] arcade =", arcade);
  console.log("[deploy] done.");
}

main().catch((err) => {
  console.error("[deploy] failed:", err);
  process.exit(1);
});
