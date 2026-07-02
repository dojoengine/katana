// PR #623 gate. Fire ONE `arcade.play_all` on the settlement chain (which sends
// an L1->L2 message to every machine) and assert that EVERY machine's coin count
// goes up on the appchain.
//
// This is the exact scenario the fix enables: the messages target distinct
// contracts, so their (global, monotonic) L1->L2 nonces are non-contiguous per
// target. Before PR #623 only the first machine's message was mined and this
// script would fail on machines #1..N (they'd never receive their coin).

import {
  account,
  APPCHAIN_RPC,
  call,
  feltToName,
  loadDeployments,
  provider,
  SETTLEMENT_RPC,
  sleep,
} from "./lib.ts";

const TIMEOUT_MS = 45_000;
const POLL_MS = 1_000;

async function coinsOf(p: ReturnType<typeof provider>, address: string): Promise<number> {
  const res = await call(p, address, "coins");
  return Number(BigInt(res[0]));
}

async function main() {
  const d = loadDeployments();
  const machines = d.appchain.machines ?? [];
  const arcade = d.settlement.arcade;
  if (!arcade || machines.length === 0) throw new Error("contracts not deployed — run deploy-game");

  const l1 = provider(SETTLEMENT_RPC);
  const l2 = provider(APPCHAIN_RPC);
  const player = d.appchain.account.address;

  console.log(`[verify] ${machines.length} machines; reading baselines...`);
  const before = await Promise.all(machines.map((m) => coinsOf(l2, m.address)));

  console.log("[verify] sending ONE play_all on L1 (fan-out to all machines)...");
  const l1Account = account(l1, d.settlement.account);
  const { transaction_hash } = await l1Account.execute({
    contractAddress: arcade,
    entrypoint: "play_all",
    calldata: [player],
  });
  await l1Account.waitForTransaction(transaction_hash);
  console.log(`[verify] L1 tx ${transaction_hash} accepted; waiting for relays...`);

  // Poll until every machine advanced by exactly 1, or time out.
  const deadline = Date.now() + TIMEOUT_MS;
  const done = new Array(machines.length).fill(false);
  for (;;) {
    const now = await Promise.all(machines.map((m) => coinsOf(l2, m.address)));
    for (let i = 0; i < machines.length; i++) {
      if (!done[i] && now[i] >= before[i] + 1) {
        done[i] = true;
        console.log(`  ✓ ${machines[i].name} received its coin (${before[i]} -> ${now[i]})`);
      }
    }
    if (done.every(Boolean)) break;
    if (Date.now() > deadline) {
      console.error("\n[verify] TIMED OUT waiting for relays. Per-machine status:");
      for (let i = 0; i < machines.length; i++) {
        const state = done[i] ? "PASS" : "STALLED (never relayed — the PR #623 bug)";
        console.error(`  ${done[i] ? "✓" : "✗"} ${machines[i].name}: ${now[i]} coins — ${state}`);
      }
      process.exit(1);
    }
    await sleep(POLL_MS);
  }

  // Sanity: last_player is the player we credited.
  const lastPlayers = await Promise.all(
    machines.map((m) => call(l2, m.address, "last_player").then((r) => r[0])),
  );
  const okPlayer = lastPlayers.every((lp) => BigInt(lp) === BigInt(player));

  console.log("\n[verify] ✅ PASS — all machines received their coin from a single L1 tx.");
  console.log(`         (fan-out to ${machines.length} distinct target contracts;`);
  console.log("          each carried a non-contiguous global L1->L2 nonce.)");
  if (!okPlayer) {
    console.error("[verify] WARNING: last_player mismatch on some machine:", lastPlayers);
  }
  // Show the names round-trip (proves per-machine identity / distinct contracts).
  const names = await Promise.all(
    machines.map((m) => call(l2, m.address, "name").then((r) => feltToName(r[0]))),
  );
  console.log("[verify] machines:", names.join(", "));
}

main().catch((err) => {
  console.error("[verify] failed:", err);
  process.exit(1);
});
