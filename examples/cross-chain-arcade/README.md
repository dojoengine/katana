# Cross-Chain Arcade

A minimal example that sends **L1 → L2 messages from one L1 contract to many
distinct L2 contracts** — the scenario fixed by katana
[PR #623](https://github.com/dojoengine/katana/pull/623).

An **arcade** dispenser on the settlement layer ("L1") drops a coin into every
**machine** on the appchain ("L2") in a *single* L1 transaction. Each machine is
a separate deployed contract with its own `insert_coin` `#[l1_handler]`.

Unlike [`cross-chain-game`](../cross-chain-game/) (a full Dojo + Torii + settlement
demo that only ever messages a single L2 target), this example is deliberately
small — **plain Cairo contracts, two Katana nodes, `starknet.js`**, and only the
L1 → L2 direction — so the fan-out to distinct targets is the whole story. (It
still uses `katana init rollup` to deploy a real piltover core contract, but no
Dojo, `sozo`, Torii, TEE registry, or embedded settlement.)

```
          arcade.play_all(player)      ── 1 L1 tx → N messages, distinct targets ──►
   ┌──────────────────────────────┐                                    ┌──────────────────────────────┐
   │  Settlement Katana :5050 (L1) │  each send_message_to_appchain     │   Appchain Katana :5051 (L2)  │
   │  • piltover core contract     │  bumps the GLOBAL nonce  ───────►  │  • SLOTS    insert_coin       │
   │    (the L1↔L2 mailbox)        │  relayed as N L1HandlerTx          │  • PINBALL  insert_coin       │
   │  • arcade dispenser contract  │  (--messaging.enabled)             │  • CLAW     insert_coin       │
   └──────────────────────────────┘                                    │  • RACER    insert_coin       │
        │ RPC                                                           └──────────────────────────────┘
        └──────────────── React app :3001 ◄──── appchain RPC (machine coins) ────────────────────────────┘
```

## What PR #623 fixes

The settlement chain's core contract assigns a **single, global, monotonic**
nonce to every L1 → L2 message, regardless of which contract it targets. Katana's
pool used to gate that nonce as if it were a **per-target account nonce**. So the
moment an L1 sender messaged a *second* contract (nonce `1`, while that contract's
own account nonce is still `0`), the message — and every later one — was rejected
with `InvalidNonce` and never mined.

That is exactly what `arcade.play_all` triggers: one L1 tx sends messages to four
distinct machine contracts, carrying global nonces `n, n+1, n+2, n+3`. With the
fix, all four are relayed and every machine gets its coin. Without it, only the
first machine (`SLOTS`) ever would.

## Prerequisites

- The `katana` binary is **built from this repo** — `up.sh` uses
  `target/debug/katana` and runs `cargo build -p katana` if it's missing. (Don't
  use a released PATH/asdf katana; you want the binary with the fix under test.)
- [`bun`](https://bun.sh/) — deploy/verify scripts + frontend.
- [`scarb`](https://docs.swmansion.com/scarb/) 2.15 (pinned in
  [`.tool-versions`](./.tool-versions); `asdf install` here). Builds the two Cairo
  contracts. **No Dojo, no `sozo`, no `torii`, no sibling `dojo` checkout.**
- `node` — only to read the generated rollup genesis when writing `deployments.json`.

The L1↔L2 mailbox (the piltover core contract) is deployed by `katana init
rollup` against the local settlement node — no `starkli` or TEE registry needed.

## Run it

```bash
cd examples/cross-chain-arcade
./up.sh
```

`up.sh` starts the settlement node, deploys the piltover core (`katana init
rollup`, validity mode), starts the appchain node from the generated rollup config
with `--messaging.enabled`, deploys the four machines + the arcade, runs the
**verify gate**, and serves the UI. Open **http://localhost:3001** and click
**🪙 Insert Coins (Play All)** — all four machine cards light up as their messages
are relayed. Ctrl-C (or `./down.sh`) tears everything down.

> The appchain node builds the full rollup genesis on boot, which takes ~40–60s
> with a **debug** katana build (`up.sh` waits for it). For a snappier demo, build
> a release binary: `cargo build -p katana --release --bin katana` and point
> `up.sh`'s `KATANA` at `target/release/katana`.

Every node also serves Katana's block explorer at `/explorer`; inspect the
appchain to see one `L1_HANDLER` tx per machine, each with a different nonce.

## The verification gate

`scripts/verify.ts` (run automatically by `up.sh`, or `bun run scripts/verify.ts`)
sends **one** `play_all` and asserts that **every** machine's coin count went up:

```
[verify] 4 machines; reading baselines...
[verify] sending ONE play_all on L1 (fan-out to all machines)...
  ✓ SLOTS received its coin (0 -> 1)
  ✓ PINBALL received its coin (0 -> 1)
  ✓ CLAW received its coin (0 -> 1)
  ✓ RACER received its coin (0 -> 1)
[verify] ✅ PASS — all machines received their coin from a single L1 tx.
```

## Seeing the old failure

To watch the bug the PR fixes, revert the fix and rebuild katana:

- In `crates/pool/pool/src/validation/stateful.rs`, remove the short-circuit that
  returns `ValidationOutcome::Valid` for `ExecutableTx::L1Handler` before the
  nonce-dependency gate.
- `cargo build -p katana --bin katana`, then `./up.sh` again.

`verify.ts` now fails: `SLOTS` gets its coin, but `PINBALL`, `CLAW`, and `RACER`
stall forever (`InvalidNonce`, never relayed). Restore the fix → green again.

## What's where

| Path | Role |
| --- | --- |
| `cairo/src/arcade.cairo` | L1 dispenser: `play_all` loops machines calling `send_message_to_appchain` (one L1 tx → N messages) |
| `cairo/src/machine.cairo` | L2 target: `insert_coin` `#[l1_handler]` (deployed once per machine, distinct addresses) |
| `scripts/deploy-game.ts` | Deploy the machines on L2 and the arcade on L1 |
| `scripts/verify.ts` | The PR #623 gate — one `play_all`, assert every machine received its coin |
| `app/` | React + Vite frontend; reads machine state over RPC (no indexer) |
| `up.sh` / `down.sh` | Start / stop the stack (2 Katanas + frontend) |

## Notes

- Both nodes run `--dev --dev.no-fee`; the appchain adds `--block-time 3000` so
  relayed L1-handlers mine on a steady cadence. Dev keys are throwaway local keys.
- The settlement node is a plain `--dev` Katana (standard dev accounts). The
  appchain boots from the rollup config `katana init rollup` generates
  (`--chain .run/chain-config`), so its settlement layer + core contract come from
  there — only `--messaging.enabled` is added to turn on the L1 → L2 relay. The
  appchain's dev account is the first account in the generated genesis, not the
  standard `--dev` account 0.
- `init rollup` runs in **validity-proof** mode with a dummy fact registry
  (`--settlement-facts-registry 0x1`). The fact registry only matters when
  *settling* (L2 → L1 `update_state`), which this demo never does — it just lets
  the appchain's startup validation accept the deployed core contract.
- The L1 sender is a *contract* (the arcade), so each machine's `insert_coin`
  sees the arcade as its message `from_address` — provenance a real machine would
  assert on.
- This example covers the **L1 → L2** direction only (what PR #623 touches). For
  the L2 → L1 settlement round trip, see `examples/cross-chain-game`.
