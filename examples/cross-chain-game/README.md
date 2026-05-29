# Cross-Chain Game Store

An end-to-end demo of **two-way Katana messaging** between a settlement layer
("L1") and an appchain ("L2"), with a React UI that reacts to both directions:

- **L1 → L2 — buy a game.** Clicking *Purchase* calls `send_message_to_appchain`
  on the settlement layer's piltover core; Katana relays it into the appchain's
  `mint_game` L1-handler.
- **L2 → L1 — sync your score.** Clicking *Sync* emits `send_message_to_l1` on the
  appchain; **saya** proves and settles the block onto the piltover core, after
  which the settlement `score_registry` consumes the message.

Both roles are Katana instances: a settlement Katana acting as the Starknet
settlement layer, and an appchain Katana running as a **rollup** (`--tee mock`)
that settles to a piltover core contract via a **saya-tee** sidecar.

```
                      send_message_to_appchain  (L1 → L2)
   ┌───────────────────────────┐ ───────────────────────► ┌──────────────────────────┐
   │  Settlement Katana :5050   │   relayed as L1-handler   │   Appchain Katana :5051   │
   │   (SN_SEPOLIA, the "L1")   │                           │  (rollup, --tee mock, L2) │
   │  • piltover core           │ ◄─────────────────────── │  • game_minter            │
   │  • score_registry          │   send_message_to_l1      │  • achievements           │
   └───────────────────────────┘   + saya settles (L2 → L1) └──────────────────────────┘
           ▲   ▲                                                      │   │
           │   │ purchase / claim (settlement acct)   sync (appchain acct)  │
           │   └──────────────────── React app :3001 ◄───────────────┘
           │                                                          
   saya-tee --mock-prove ── proves each appchain block, submits update_state ──┘
```

## Prerequisites

- The `katana` binary (`cargo build --release`), or `katana` on `PATH`.
- [`scarb`](https://docs.swmansion.com/scarb/) — builds the appchain/settlement contracts.
- [`bun`](https://bun.sh/) — deploy scripts + frontend.
- [`saya-tee`](https://github.com/cartridge-gg/saya) and `saya-ops` **v0.4.0**, on `PATH`,
  **with the patch in [`saya-patch/`](./saya-patch/README.md) applied** (saya 0.4.0
  hashes L1→L2 messages with the Ethereum keccak formula; a Starknet-settled
  appchain needs the Poseidon formula, or L1→L2 settlement stalls).

## Run it

```bash
cd examples/cross-chain-game
./up.sh
```

`up.sh` builds the contracts, starts the settlement Katana, deploys a mock TEE
registry (`saya-ops`) + the piltover core (`katana init rollup --tee`), starts the
appchain rollup + the `saya-tee` sidecar, deploys the demo contracts, and serves the
UI. Open **http://localhost:3001**. Ctrl-C (or `./down.sh`) tears everything down.

Each node also serves Katana's block explorer (`--explorer`) at `/explorer`, and
every tx hash in the UI deep-links to the right node's explorer.

## What's where

| Path | Role |
| --- | --- |
| `cairo/src/game_minter.cairo` | Appchain: `mint_game` L1-handler + views (L1→L2) |
| `cairo/src/achievements.cairo` | Appchain: `sync_score` → `send_message_to_l1` (L2→L1) |
| `cairo/src/score_registry.cairo` | Settlement: consumes the settled message via `consume_message_from_appchain` |
| `scripts/deploy.ts` | Deploys the three contracts onto the running stack |
| `app/` | React + Vite + TS + [shadcn/ui](https://ui.shadcn.com) frontend |
| `saya-patch/` | The required saya-tee fix + rationale |
| `up.sh` / `down.sh` | Start / stop the whole stack |

## How each direction works

**L1 → L2 (instant):** `piltover.send_message_to_appchain(game_minter, mint_game, [game_id])`
emits `MessageSent`; the appchain (`--messaging.enabled`) relays it as an `L1HandlerTx`
that runs `mint_game(from_address, game_id)`. The UI polls `total_minted`.

**L2 → L1 (settled by saya):** `achievements.sync_score(score)` calls
`send_message_to_l1(score_registry, [player, score])`. saya-tee proves the appchain
block and submits `update_state` to the piltover core, which registers the message.
Once settled, `score_registry.claim_score(achievements, player, score)` consumes it via
`consume_message_from_appchain` and stores the score. The UI shows the round trip:
*emitted on L2 → saya settling block N → claimed on L1*.

## Notes

- The settlement node runs `--dev.no-fee` and the appchain runs `--dev --dev.no-fee`
  (fees off, mirroring the saya-tee test harness' `fee:false` rollup config). The dev
  keys are throwaway local keys — never reuse with real funds.
- `--http.cors_origins '*'` lets the browser app read the RPCs. Scope it down outside
  local dev.
- `app/src/deployments.json` and `.run/chain-config/` are regenerated every `up.sh` run.
- This is `--mock-prove` (no real SP1/TEE). It proves the messaging + settlement
  plumbing, not proof soundness.
