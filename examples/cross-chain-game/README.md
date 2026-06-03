# Cross-Chain Dice

A small game that demonstrates **two-way Katana messaging** between a settlement
layer ("L1") and an appchain ("L2"), built on the **Dojo framework** and indexed
by **Torii**. The whole game lives in three Dojo worlds (`store` + `score` on L1,
`game` on L2):

1. **Buy games (L1 → L2).** *Insert coin* calls `buy_game` on the L1 `store`
   contract, which runs the store's rules and then messages the appchain via the
   piltover core. Katana relays it into the game world's `mint_game` `#[l1_handler]`,
   adding a credit to the playable pool.
2. **Play a game (L2).** *Roll* calls the game system's `play_game` — the appchain
   rolls a score on-chain, consumes one credit, and finishes the game.
3. **Bank the score (L2 → L1).** `play_game` emits the score to L1 via
   `send_message_to_l1` in the same tx. **saya** proves and settles the appchain
   block onto the piltover core; the settlement score world's `claim_score` then
   consumes the message (`consume_message_from_appchain`) and records the run.

Both roles are Katana instances: a settlement Katana acting as the Starknet
settlement layer, and an appchain Katana running as a **rollup** (`--tee mock`)
that settles to a piltover core via a **saya-tee** sidecar. Game/score state lives
in Dojo models; the frontend reads it from a Torii indexer per chain (it never
calls contract views), and rebuilds its feeds from Dojo events.

> **Building your own appchain app?** This demo doubles as a worked example for a
> generalized guide — architecture, why each service exists, the contracts, and
> how the client queries state: see **[docs/](./docs/README.md)**.

```
                         buy → mint  (L1 → L2)
   ┌───────────────────────────┐ ─────────────────────► ┌──────────────────────────┐
   │  Settlement Katana :5050   │  relayed as L1-handler  │   Appchain Katana :5051   │
   │   (SN_SEPOLIA, the "L1")   │                         │  (rollup, --tee mock, L2) │
   │  • piltover core           │ ◄───────────────────── │  • game world             │
   │  • store + score worlds    │  score (L2 → L1) + saya │    (mint / play / publish)│
   └───────────────────────────┘                         └──────────────────────────┘
        │ torii :8081                                          │ torii :8082
        └──────────────────── React app :3001 ◄────────────────┘
                          (reads models + events via Torii SQL)
   saya-tee --mock-prove ── proves each appchain block, submits update_state ──┘
```

## Prerequisites

- The `katana` binary (`cargo build --release`), or `katana` on `PATH`.
- [`bun`](https://bun.sh/) — deploy scripts + frontend.
- The Dojo toolchain, pinned in [`.tool-versions`](./.tool-versions): **sozo 1.8.7**
  (migrates the worlds; bundles its own scarb 2.13.1) and **torii 1.8.16** (indexer).
  Install with [`asdf`](https://asdf-vm.com/): `asdf install` in this directory.
- A sibling checkout of the [`dojo`](https://github.com/dojoengine/dojo) repo
  (`../../../../dojo`, i.e. alongside `katana`) at the `sozo/v1.8.7` line — the
  Cairo packages depend on it by path so the world class hash matches `sozo`.
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

`up.sh` is the one-click entry point. It first **preflights the toolchain** —
auto-installing the Dojo tools (`sozo`/`torii`/`scarb` via `asdf install`) and the
JS deps, and failing fast with the exact command if a heavy prerequisite is
missing (the `katana` binary, the patched `saya`, or the sibling `dojo` checkout).
It then starts the settlement Katana, deploys a mock TEE registry (`saya-ops`) +
the piltover core (`katana init rollup --tee`), starts the appchain rollup + the
`saya-tee` sidecar, **migrates the Dojo worlds with `sozo`** (the settlement-side
`store` and `score` worlds, then the appchain `game` world — wired with each
other's addresses), starts a **Torii indexer per chain**, and serves the UI. Open
**http://localhost:3001**. Ctrl-C (or `./down.sh`) tears everything down.

Each node serves Katana's block explorer (`--explorer`) at `/explorer`; every tx
hash in the UI deep-links to the right node's explorer.

## Using Controller (optional)

By default the demo signs everything with a prefunded **dev account** — no wallet,
no login, fully offline. The header **Login** button also lets you sign with a
[Cartridge Controller](https://github.com/cartridge-gg/controller) instead. The
*same* Controller identity signs on **both chains** — buy + bank on L1 **and** the
roll on the appchain — at the **same address** (a Controller is deployed
deterministically from its username, so its address is chain-independent). That
closes the attribution gap: `play_game`'s caller, the L2→L1 score message, and the
leaderboard all key on your Controller, not a shared dev key.

Controller is a hosted-keychain wallet, so it isn't offline. To enable it:

```bash
CONTROLLER=1 ./up.sh
```

This starts **both** nodes Controller-capable (`--paymaster --cartridge.paymaster
--cartridge.controllers`; katana fetches the `paymaster-service` sidecar if needed —
see [`../../docs/cartridge.md`](../../docs/cartridge.md)). It also declares the
Controller account class on the appchain at boot (working around katana #584; see
[`docs/client.md`](./docs/client.md)), so the same Controller can deploy and sign
there too — no manual step.
It also **serves the app over trusted HTTPS** at `https://localhost:3001` (via
`mkcert`). Then click **Login → Connect Controller** and sign in. Caveats:

- Needs internet + a Controller account (the keychain + Cartridge API own the identity).
- **HTTPS is required for passkey login.** Controller's WebAuthn flow needs a secure
  context with a *trusted* cert — plain `http://localhost` or a self-signed cert fails
  with *"WebAuthn is not supported on sites with TLS certificate errors"*. The first
  `CONTROLLER=1` run installs a local `mkcert` CA (a one-time OS prompt); after that
  `https://localhost:3001` is trusted.
- The hosted keychain reaching `localhost` may still need Chrome's Private Network
  Access flag — enable `chrome://flags/#local-network-access-check` if connect stalls.
- Session policies cover `store.buy_game`, `score_registry.claim_score`, and
  `game.play_game`, so buy/roll/bank are gasless session calls (no per-tx popup).
  The roll switches the Controller to the appchain (chain id `GAMECHAIN`) for the
  call, then switches back.

Without `CONTROLLER=1`, choosing *Connect Controller* simply can't complete; the
dev account keeps working.

## What's where

| Path | Role |
| --- | --- |
| `cairo/game/src/lib.cairo` | Appchain game world (Dojo): `mint_game` l1_handler, `play_game`, `Stats`/`GameConfig` models, `GameMinted`/`GamePlayed` events |
| `cairo/store/src/lib.cairo` | Settlement store world (Dojo): `buy_game` runs the store's rules then `send_message_to_appchain` (the L1 storefront) |
| `cairo/score/src/lib.cairo` | Settlement score world (Dojo): `claim_score` (consumes the settled message), `Leaderboard`/`PlayerScore` models, `ScoreClaimed` event |
| `scripts/deploy.ts` | `sozo migrate` the three worlds (score → game → store) and record addresses in `deployments.json` |
| `app/` | React + Vite + TS + [shadcn/ui](https://ui.shadcn.com) frontend; reads via Torii SQL (`app/src/chain.ts`); wallet picker (dev account / Controller) in `app/src/wallet.tsx` |
| `saya-patch/` | The required saya-tee fix + rationale |
| `up.sh` / `down.sh` | Start / stop the whole stack (2 Katanas + saya-tee + 2 Torii + frontend) |

## How the messaging works

**Buy → mint (L1 → L2, instant):** the client calls `store.buy_game(game_id)`, which
calls `piltover.send_message_to_appchain(game, mint_game, [game_id])` and emits `MessageSent`;
the appchain (`--messaging.enabled`) relays it as an `L1HandlerTx` that runs the game
world's `mint_game` l1_handler, incrementing the `Stats.available` model.

**Play → publish (L2 → L1, settled by saya):** the game system's `play_game()` rolls a
score, then calls `send_message_to_l1(score_system, [player, score])` in the same tx.
saya-tee proves the appchain block and submits `update_state` to the piltover core,
which registers the message. The frontend then calls the score system's `claim_score(...)`,
which consumes it via `consume_message_from_appchain` and writes the `Leaderboard` model.

**Reads:** every Dojo write updates a model (indexed by Torii) and/or emits a Dojo event
(stored in per-event tables). The frontend queries each chain's Torii over its SQL
endpoint (`/sql`) — current state from model rows, per-mint/play/bank feeds from event
tables. The one exception is the L1-side purchase log (piltover `MessageSent`), read
straight from the settlement RPC so the "pending mint" state shows before the relay.

## Notes

- The settlement node runs `--dev.no-fee`, the appchain runs `--dev --dev.no-fee`
  (fees off, mirroring the saya-tee test harness). Dev keys are throwaway local
  keys — never reuse with real funds.
- Dojo worlds are deterministic in their seed (`ccg_game` / `ccg_score` / `ccg_store`), so a
  re-migration onto a fresh chain lands at the same world address.
- The on-chain roll is a Poseidon-based pseudo-random in `1..=100` — fine for a demo,
  not secure randomness.
- `--http.cors_origins '*'` on both Katanas and both Torii lets the browser read
  them. Scope it down outside local dev.
- Torii relay ports are set per instance (9181-9183 / 9184-9186) to avoid clashing
  with each other and with other local Dojo projects' defaults (8080 / 9090).
- `app/src/deployments.json`, the generated `cairo/*/dojo_dev.toml` + `manifest_dev.json`,
  and `.run/` are regenerated every `up.sh` run.
- This is `--mock-prove` (no real SP1/TEE). It exercises the messaging + settlement
  plumbing, not proof soundness.
