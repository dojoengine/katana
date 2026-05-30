# Cross-Chain Dice

A small game that demonstrates **two-way Katana messaging** between a settlement
layer ("L1") and an appchain ("L2"). The game has three explicit phases:

1. **Buy games (L1 → L2).** Click *Buy a game* (as many as you like). Each
   purchase sends a message from the settlement layer's piltover core that Katana
   relays into the appchain's `mint_game` handler, adding a game to the playable
   pool.
2. **Play a game (L2).** The UI shows how many games are **available to play**.
   Click *🎲 Roll* — the appchain rolls a score on-chain and instantly finishes
   the game (one available game is consumed per play).
3. **Publish the score (L2 → L1, automatic).** Finishing a game emits the score to
   L1 in the same transaction — no separate step. **saya** proves and settles the
   block onto the piltover core, then the settlement `score_registry` consumes the
   message and records the score.

Both roles are Katana instances: a settlement Katana acting as the Starknet
settlement layer, and an appchain Katana running as a **rollup** (`--tee mock`)
that settles to a piltover core via a **saya-tee** sidecar.

```
                         buy → mint  (L1 → L2)
   ┌───────────────────────────┐ ─────────────────────► ┌──────────────────────────┐
   │  Settlement Katana :5050   │  relayed as L1-handler  │   Appchain Katana :5051   │
   │   (SN_SEPOLIA, the "L1")   │                         │  (rollup, --tee mock, L2) │
   │  • piltover core           │ ◄───────────────────── │  • game                   │
   │  • score_registry          │  score (L2 → L1) + saya │    (mint / play / publish)│
   └───────────────────────────┘                         └──────────────────────────┘
           ▲   ▲                                                   │   │
           │   │ buy / claim (settlement acct)        play/roll (appchain acct)
           │   └─────────────────── React app :3001 ◄─────────────┘
   saya-tee --mock-prove ── proves each appchain block, submits update_state ──┘
```

## Prerequisites

- The `katana` binary (`cargo build --release`), or `katana` on `PATH`.
- [`scarb`](https://docs.swmansion.com/scarb/) — builds the Cairo contracts.
- [`bun`](https://bun.sh/) — deploy scripts + frontend.
- [`saya-tee`](https://github.com/cartridge-gg/saya) and `saya-ops` **v0.4.0** on
  `PATH`, **with the patch in [`saya-patch/`](./saya-patch/README.md) applied**
  (saya 0.4.0 hashes L1→L2 messages with the Ethereum keccak formula; a
  Starknet-settled appchain needs the Poseidon formula, or settlement of blocks
  that consume an L1→L2 message — i.e. every purchase — stalls).

## Run it

```bash
cd examples/cross-chain-game
./up.sh
```

`up.sh` builds the contracts, starts the settlement Katana, deploys a mock TEE
registry (`saya-ops`) + the piltover core (`katana init rollup --tee`), starts the
appchain rollup + the `saya-tee` sidecar, deploys the contracts, and serves the UI.
Open **http://localhost:3001**. Ctrl-C (or `./down.sh`) tears everything down.

Each node serves Katana's block explorer (`--explorer`) at `/explorer`; every tx
hash in the UI deep-links to the right node's explorer.

## What's where

| Path | Role |
| --- | --- |
| `cairo/src/game.cairo` | Appchain: `mint_game` (purchase), `play_game` (roll + publish), views |
| `cairo/src/score_registry.cairo` | Settlement: consumes the published score via `consume_message_from_appchain` |
| `scripts/deploy.ts` | Deploys score_registry (L1) then game (L2) |
| `app/` | React + Vite + TS + [shadcn/ui](https://ui.shadcn.com) frontend |
| `saya-patch/` | The required saya-tee fix + rationale |
| `up.sh` / `down.sh` | Start / stop the whole stack |

## How the messaging works

**Buy → mint (L1 → L2, instant):** `piltover.send_message_to_appchain(game, mint_game, [game_id])`
emits `MessageSent`; the appchain (`--messaging.enabled`) relays it as an `L1HandlerTx`
that runs `mint_game`, incrementing the available pool.

**Play → publish (L2 → L1, settled by saya):** `game.play_game()` rolls a score, then
calls `send_message_to_l1(score_registry, [player, score])` in the same tx. saya-tee
proves the appchain block and submits `update_state` to the piltover core, which
registers the message. The frontend then auto-calls `score_registry.claim_score(...)`,
which consumes it via `consume_message_from_appchain` and stores the score.

## Notes

- The settlement node runs `--dev.no-fee`, the appchain runs `--dev --dev.no-fee`
  (fees off, mirroring the saya-tee test harness). Dev keys are throwaway local
  keys — never reuse with real funds.
- The on-chain roll is a Poseidon-based pseudo-random in `1..=100` — fine for a demo,
  not secure randomness.
- `--http.cors_origins '*'` lets the browser read the RPCs. Scope it down outside local dev.
- `app/src/deployments.json` and `.run/chain-config/` are regenerated every `up.sh` run.
- This is `--mock-prove` (no real SP1/TEE). It exercises the messaging + settlement
  plumbing, not proof soundness.
