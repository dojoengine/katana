# cross-chain-dungeon Implementation Plan

## Overview
A second Katana appchain example (sibling to `examples/cross-chain-game`) that
settles to **real Starknet Sepolia** instead of a local settlement Katana. It
demonstrates an appchain app that **leans on an external settlement-layer contract**
(Circle USDC on Sepolia) via a custom **GAME_TOKEN** economy, and runs **more
gameplay on the appchain**: a dungeon run is one appchain transaction per action
(move / attack / loot / use item), settling a final score + token reward back to
Sepolia. It reuses the proven shape of the first example (worlds-per-chain,
piltover mailbox, saya settlement, two Torii instances, a React/Vite client) and
changes only what the new requirements force.

## Goals
- A runnable example under `examples/cross-chain-dungeon/` that settles a local
  appchain to **real Starknet Sepolia**.
- Demonstrate **depending on an external contract on the settlement layer**: real
  Circle **USDC** gates the economy (you buy GAME_TOKEN with USDC).
- A custom **GAME_TOKEN** (ERC20): bought with USDC at a fixed rate (mint-on-
  purchase), with a **dev-mint** faucet so the demo is playable without acquiring
  real USDC. **Charged per dungeon entry.**
- **More appchain operations**: a dungeon run is many appchain txns (tx-per-action),
  with run state in Dojo models and per-action event feeds.
- A complete economic loop: **spend GAME_TOKEN to enter → play on the appchain →
  bank score to Sepolia → earn a proportional GAME_TOKEN reward**.
- **Cartridge Controller on Sepolia only** for the settlement-side writes (buy,
  approve, enter, bank); appchain actions on the dev key.
- A one-command `up.sh` / `down.sh`, mirroring the first example's docs structure.

## Non-Goals
- Real proof soundness. We use saya `--mock-prove` with a mock TEE registry (the
  same plumbing as the first example), now pointed at Sepolia. Real Atlantic/SP1
  proving is explicitly out of scope (it was offered and declined).
- Controller signing **appchain** actions. The hosted keychain can't switch to a
  local custom appchain (the open issue from `cross-chain-game`); appchain actions
  use the dev key here by design.
- Mainnet. Sepolia only.
- VRF-based randomness on the appchain (note it as a future upgrade; v1 uses
  deterministic pseudo-randomness like the first example).
- An AMM/price oracle for USDC→GAME. The rate is a fixed constant.
- Self-hosting the Cartridge keychain or Cartridge API.

## Assumptions and Constraints
- **Funded Sepolia operator account.** Deploying piltover, the mock TEE registry,
  GAME_TOKEN, TokenSale, Entry, and migrating the settlement `score` world are all
  **real Sepolia transactions** that cost STRK. A funded operator account
  (address + private key) is supplied via env.
- **saya pays ongoing Sepolia gas.** saya submits `update_state` to piltover on
  Sepolia for each settled batch — a recurring real cost. saya needs its own funded
  Sepolia account (or shares the operator account, with nonce contention handled —
  see Risks; the first example hit exactly this with the Controller paymaster).
- **Real USDC is hard to get on Sepolia.** The dev-mint path exists so the demo is
  playable without it; the USDC→GAME purchase path is the "real external
  dependency" showcase and needs the player to hold test USDC.
- **`katana init rollup` can target real Sepolia.** Its prompt/flags already support
  a Sepolia settlement provider (account + private key + settlement-contract deploy).
- **saya Poseidon patch still required.** A Starknet-settled appchain hashes L1→L2
  messages with Poseidon; saya 0.4.0 ships keccak. Reuse `../cross-chain-game/
  saya-patch/` (or its published successor). Without it, every entry (an L1→L2
  message) stalls in settlement.
- **Toolchain pinned** to the same versions as the first example unless a newer
  Dojo/Torii is required: `scarb 2.13.1`, `sozo 1.8.7`, `torii 1.8.16`.
- The appchain stays **local** (`localhost`), fee-less (`--dev.no-fee`), single
  instance. Only the settlement layer moves to Sepolia.
- **Distinct ports from `cross-chain-game`** so both demos can run at once. The
  first example uses appchain `5051` (+ settlement `5050`), Torii `8081`/`8082`
  (gRPC `50081`/`50082`, relay `9181`/`9184`), frontend `3001`. This example has
  **no local settlement node** (Sepolia is remote) and uses a disjoint band:

  | Service | Port |
  |---|---|
  | Appchain Katana RPC (+ `/explorer`) | `5070` |
  | Torii — Sepolia `score` world (HTTP / gRPC / relay) | `8091` / `50091` / `9191` |
  | Torii — appchain `game` world (HTTP / gRPC / relay) | `8092` / `50092` / `9194` |
  | Frontend (Vite) | `3002` |

  Torii relay also claims `+1`/`+2` (webrtc/websocket) off each relay base — so
  `9191–9193` and `9194–9196`, clear of the first example's `9181–9186`.

## Requirements

### Functional
- Buy GAME_TOKEN with USDC: `approve(USDC)` then `TokenSale.buy(usdc_amount)` mints
  `usdc_amount * RATE` GAME_TOKEN to the caller; USDC accrues to a treasury.
- Dev-mint GAME_TOKEN: a faucet entrypoint mints GAME_TOKEN directly to the caller
  (no USDC), for development.
- Enter a dungeon: `approve(GAME_TOKEN)` then `Entry.enter()` charges `ENTRY_FEE`
  GAME_TOKEN and sends an L1→L2 message that starts a run for the player on the
  appchain.
- Play (appchain, tx per action): `move`, `attack`, `loot`, `use_item` each update
  run state, apply a pseudo-random outcome, and emit a feed event; runs end on death
  or when the player chooses to extract.
- Bank: ending a run sends an L2→L1 message with `(player, score, loot)`; once saya
  settles the block, `claim_run` on Sepolia consumes it, writes the leaderboard, and
  **mints a proportional GAME_TOKEN reward** to the player.
- Read: live run state + action feed from the appchain Torii; leaderboard + bank
  feed from the Sepolia Torii; USDC/GAME balances + allowances + settled height from
  RPC.
- Controller (Sepolia only) signs buy/approve/enter/bank; dev key signs appchain
  actions.

### Non-Functional
- One-command bring-up (`./up.sh`) and teardown (`./down.sh`); the appchain side is
  one-click, the Sepolia side requires the operator env to be set.
- Banking latency is bounded by Sepolia inclusion + saya settlement; the UI must
  show a pending→settled transition (reuse the "settled N / tip M" gauge).
- No secrets committed: operator/saya keys come from env or a gitignored `.env`.
- Same-account nonce safety on Sepolia (serialize settlement-account writes, as the
  first example does with `withSettlementLock`).

## Technical Design

### Data Model

**Appchain `game` world (Dojo, local rollup):**
- `Stats` (singleton `id=0`): `total_runs`, `active_runs`, `total_actions`,
  `total_banked`.
- `GameConfig` (singleton): `registry` (the Sepolia `score` system address, set in
  `dojo_init`), plus tunables (`max_hp`, `base_enemy`, etc.).
- `RunState` (key: `player_l1: felt252`): `alive`, `depth`, `hp`, `max_hp`, `gold`,
  `room_kind` (monster/treasure/trap/shrine/empty), `enemy_hp` (0 if no monster),
  `potions`, `seed`, `action_count`, `started_block`. One active run per player.
- Events (keyed by a monotonic sequence so Torii keeps one row each — the append-log
  rule): `RunStarted { #[key] run_no, player_l1, seed }`,
  `ActionTaken { #[key] action_no, player_l1, kind, depth, hp, gold, outcome }`,
  `RunEnded { #[key] end_no, player_l1, score, loot, died }` (emitted on both
  extract and death; only **extract** also sends the L2→L1 message).

**Settlement `score` world (Dojo, Sepolia):**
- `Leaderboard` model (key: `player`): `best_score`, `runs`, `total_reward`.
- `GameConfig` (singleton): `piltover`, `appchain_game_system` (the L2→L1
  `from_address`, supplied at call time, not init), `game_token`, `reward_rate`.
- Event: `RunBanked { #[key] claim_no, player, score, loot, reward }` (append feed).

**Settlement plain Cairo contracts (Sepolia):**
- `GameToken` — OZ ERC20 + controlled mint. Authorized minters: `TokenSale`
  (mint-on-purchase), the `score` claim system (mint-on-bank), and an owner
  dev-mint. Decimals 18.
- `TokenSale` — holds `usdc` address + `treasury` + `RATE`; `buy(usdc_amount)`:
  `IERC20(usdc).transfer_from(caller, treasury, usdc_amount)` then
  `GameToken.mint(caller, usdc_amount * RATE)`. `dev_mint(amount)` faucet.
- `Entry` — holds `game_token`, `entry_fee`, `sink/treasury`, `piltover`,
  `appchain_game_system`, `mint_run_selector`. `enter()`:
  `GameToken.transfer_from(caller, sink, entry_fee)` then
  `piltover.send_message_to_appchain(appchain_game_system, mint_run_selector,
  [caller, seed])`. (Seed derived from caller + L1 block.)

USDC is **external and pre-existing** on Sepolia — referenced by address only
(env/config), never deployed by us.

### API Design (contract entrypoints)

| Chain | Contract | Entrypoint | Purpose |
|------|----------|-----------|---------|
| Sepolia | USDC (external) | `approve`, `transfer_from`, `balance_of` | external dependency |
| Sepolia | `GameToken` | `buy`-minter `mint`, `approve`, `transfer_from`, `balance_of`, `dev_mint` | game currency |
| Sepolia | `TokenSale` | `buy(usdc_amount)`, `dev_mint(amount)` | USDC→GAME |
| Sepolia | `Entry` | `enter()` | charge GAME + L1→L2 start run |
| Appchain | `game` | `#[l1_handler] mint_run(from, player, seed)` | start run |
| Appchain | `game` | `move()`, `attack()`, `loot()`, `use_item()` | tx-per-action play |
| Appchain | `game` | `extract()` → `send_message_to_l1([player, score, loot])` | commit + bank (alive only) |
| Sepolia | `score` | `claim_run(appchain_game_system, player, score, loot)` | consume + leaderboard + mint reward |

Death is **not** an entrypoint — it's a side effect of `attack()`/`move()`/`loot()`
dropping HP to 0. A death finalizes `RunState` and emits `RunEnded { died: true }`
**locally on the appchain only**; it sends no L2→L1 message and pays no reward (see
Game Design). Only `extract()` commits to Sepolia.

**Sacred payload contract:** L2→L1 payload is `[player, score, loot]`; the `score`
consumer must reconstruct it exactly (and pass the appchain game system as
`from_address`) or `consume_message_from_appchain` reverts.

### Game Design (dungeon mechanics)

A **push-your-luck roguelite**. The genre is chosen deliberately: a player's loot
lives on the fast/cheap appchain but isn't *real* until settled to Sepolia, so the
cross-chain "commit" is the core decision, not a footnote. Pay an entry fee in
GAME_TOKEN, descend room by room; deeper = more gold and score but more lethal;
**dying forfeits the haul**, and the only way to keep it is to **extract** (the
L2→L1 settlement).

**Room types** (rolled on entering a room; monster share climbs with depth):
monster ~45% · treasure ~25% · trap ~15% · shrine ~10% · empty/rest ~5%.

**Actions** (each is its own appchain tx — the source of "more ops on the appchain"):
- `move()` — advance one room (`depth += 1`), roll the new room. If a monster is
  present, `move()` is a **flee attempt** (RNG: succeed and advance, or take a
  parting hit and stay).
- `attack()` — combat only. You hit the enemy (RNG dmg); it hits back (RNG dmg).
  Repeat until it dies (drops gold) or your HP hits 0.
- `loot()` — treasure room / post-kill only. Gold + sometimes a potion; small mimic
  (trap) chance.
- `use_item()` — spend a potion to heal; valid whenever `potions > 0`.
- `extract()` — leave **alive**; the bank/commit. Allowed any time you are not
  mid-combat (i.e. `enemy_hp == 0`).

Invalid actions revert (loot an empty room, fight nothing, enter with a run already
active — one active run per `player_l1`).

**Constants (v1 defaults, all tunable):**
- `max_hp = 100`
- `enemy_hp(depth) = 20 + 8·depth`, `enemy_dmg(depth) = 6 + 2·depth` (±RNG)
- player attack = `12–20` (RNG); `trap_dmg(depth) = 8 + 3·depth`
- treasure gold = `10 + 5·depth` (RNG); monster-kill gold = `5 + 4·depth`
- potion heal = `35`; start with `1` potion
- RNG = `poseidon(seed, action_count) % range` — deterministic per run, fresh per
  action (`action_count` is the nonce). Deterministic is fine for a demo; **Cartridge
  VRF** is the upgrade for true unpredictability.

A run lasts ~10–25 actions before the player extracts or dies.

**How a run ends (two outcomes):**
1. **Extract → settles to Sepolia (the only paying path).** `extract()` finalizes
   the run alive, emits `RunEnded { died: false }`, and sends `send_message_to_l1
   ([player_l1, score, loot])`. After saya settles that block, `claim_run` on Sepolia
   writes the leaderboard and **mints `score · REWARD_RATE` GAME_TOKEN** to the player.
2. **Death → local L2 finalization, forfeit.** HP reaching 0 (combat or trap)
   finalizes the run, emits `RunEnded { died: true }` on the appchain only — **no
   L2→L1 message, no reward, no leaderboard row.** The player is still out the
   GAME_TOKEN entry fee. (The death is visible in the appchain feed; it never
   reaches Sepolia. A "deepest dive" death-leaderboard that *does* settle is a
   possible future addition.)

This asymmetry is the lesson: **appchain value is provisional until committed to the
settlement layer.** Extracting *is* that commit.

**Scoring & economy:**
- On extract: `score = DEPTH_WEIGHT·depth + gold`; reward = `score · REWARD_RATE`.
- On death: nothing settles.
- Tuned so shallow extracts roughly refund `ENTRY_FEE`, deep extracts profit, and
  deaths are a net loss — making "how deep do I dare go?" the central decision.

### Architecture

```
        Starknet Sepolia (settlement, REAL)            Local appchain (Katana rollup)
  ┌─────────────────────────────────────────┐        ┌──────────────────────────────┐
  │ USDC(ext) ─approve→ TokenSale ─mint→ GAME│        │  game world (dungeon)        │
  │                         │                │  L1→L2 │   RunState / Stats           │
  │   GAME ─approve→ Entry ─┴─ send_msg ─────┼──────► │   mint_run (l1_handler)      │
  │                                          │ (relay)│   move/attack/loot/use_item  │
  │   score world ◄── consume_msg ◄──────────┼────────┤   extract → send_to_l1       │
  │     leaderboard + mint GAME reward       │  L2→L1 └───────────┬──────────────────┘
  │            ▲          ▲                  │ (settled)          │ update_state
  │            │ index    │ piltover get_state│                   ▼
  │         Torii(Sepolia)│◄── client ──┐    │                 saya --mock-prove
  └────────────────────────────────────┼────┘                 (+ mock TEE registry
            Torii(appchain) ◄───────────┘                        deployed on Sepolia)
```

- **Single Katana** (appchain). Settlement is remote Sepolia.
- **saya** proves appchain blocks and calls `update_state` on the Sepolia piltover.
- **Torii ×2**: one against the Sepolia RPC (`score` world), one against the appchain
  (`game` world). Token balances read via RPC, not indexed.

### Identity mapping (important)
The player identity on Sepolia is the **Controller** address. Appchain actions are
signed by the **dev key**. So the appchain `RunState` is keyed by `player_l1` (the
Sepolia player carried in the entry message), and action systems operate on that
run regardless of the (dev) caller. On bank, the L2→L1 payload carries `player_l1`
so the leaderboard row and the GAME reward land on the real player on Sepolia. This
is a deliberate demo simplification (the dev key can act on any run); document it.

### UX Flow
1. **Fund**: connect Controller (Sepolia). Show USDC + GAME balances. Either
   `Dev-mint GAME` (one click) or `Buy GAME` (approve USDC → buy).
2. **Enter**: `approve GAME` → `enter()`. Show the entry as *pending* (piltover
   `MessageSent`) until the appchain relays `mint_run` and `RunState` appears.
3. **Play**: a dungeon board driven by the appchain Torii — current room/`room_kind`,
   HP/gold/depth/potions from `RunState`, an action feed from `ActionTaken`. Buttons
   for move/attack/loot/use-item (enabled by room/combat state) each send an appchain
   tx (dev key) and poll Torii until indexed. An **Extract** button (enabled only when
   not in combat) is always tempting next to the climbing risk.
4. **Die**: if HP hits 0 the run ends in-place — show the forfeit (haul lost, nothing
   settles). No banking step. The player can enter again (pay another fee).
5. **Extract → Bank**: `extract()` sends the L2→L1 message; show the run *banking*
   with the settled/tip gauge. Once settled, `claim_run` (Controller) consumes,
   the leaderboard updates, and the GAME reward arrives (balance ticks up).
   Refresh-proof, derived from Torii.

---

## Implementation Plan

### Serial Dependencies (Must Complete First)

#### Phase 0: Foundation & on-Sepolia bootstrap
**Prerequisite for:** All subsequent phases

| Task | Description | Output |
|------|-------------|--------|
| 0.1 | Scaffold `examples/cross-chain-dungeon/` mirroring the first example (`cairo/`, `scripts/`, `app/`, `docs/`, `up.sh`, `down.sh`, `.tool-versions`, `.gitignore`). | Directory skeleton |
| 0.2 | Define config/env: `SEPOLIA_RPC_URL`, `OPERATOR_ADDRESS`/`OPERATOR_PRIVKEY`, `SAYA_ADDRESS`/`SAYA_PRIVKEY` (or shared), `USDC_ADDRESS` (real Circle Sepolia — **verify canonical address**, see Open Questions), `RATE`, `ENTRY_FEE`, `REWARD_RATE`. Gitignored `.env.example`. | `config.ts` + `.env.example` |
| 0.3 | Resolve/verify the saya Poseidon patch and the `saya-tee`/`saya-ops` binaries work against a **remote Sepolia** RPC (not just localhost). | Verified prover tooling |
| 0.4 | Deploy the **mock TEE registry** on Sepolia (`saya-ops`, operator account). | `tee_registry` address |
| 0.5 | `katana init rollup --tee` targeting **Sepolia** (settlement provider = Sepolia RPC + operator key, fact registry = mock TEE registry) → deploys **piltover** on Sepolia, writes the appchain chain config. | `piltover` address + chain config |
| 0.6 | Wire the Dojo dependency (path or git tag at the sozo-matching commit) for all worlds. | Buildable `Scarb.toml`s |
| 0.7 | Write base `deployments.json` (Sepolia RPC, appchain RPC, accounts, piltover, USDC, Torii urls). | `deployments.json` seed |

---

### Parallel Workstreams (after Phase 0)

#### Workstream A: Settlement contracts (Sepolia)
**Dependencies:** Phase 0 · **Can parallelize with:** B, C

| Task | Description | Output |
|------|-------------|--------|
| A.1 | `GameToken` ERC20 (OZ) with authorized-minter set + owner `dev_mint`. | `cairo/token` |
| A.2 | `TokenSale`: `buy(usdc_amount)` (USDC `transfer_from` → mint GAME at `RATE`), `dev_mint`. Holds USDC addr + treasury. | `cairo/token` |
| A.3 | `Entry`: `enter()` charges `ENTRY_FEE` GAME, calls piltover `send_message_to_appchain(appchain_game_system, mint_run_selector, [player, seed])`. | `cairo/entry` |
| A.4 | `score` Dojo world: `Leaderboard`/`RunBanked`, `claim_run` consumes the L2→L1 msg, writes leaderboard, mints GAME reward (`score * REWARD_RATE`). | `cairo/score` |
| A.5 | Unit tests: sale math, dev-mint, entry charge + message emission, claim consume (mock) + reward mint, minter authorization. | `sozo test` green |

#### Workstream B: Appchain dungeon world
**Dependencies:** Phase 0 · **Can parallelize with:** A, C

| Task | Description | Output |
|------|-------------|--------|
| B.1 | Models: `Stats`, `GameConfig`, `RunState`; events `RunStarted`/`ActionTaken`/`RunEnded` keyed by sequence. | `cairo/game` |
| B.2 | `#[l1_handler] mint_run(from, player, seed)` — create `RunState`, bump `Stats`, emit `RunStarted`. | run start |
| B.3 | Action systems `move`/`attack`/`loot`/`use_item` — room-kind/combat validity, pseudo-random outcomes from `poseidon(seed, action_count)`, mutate HP/gold/depth/potions/`room_kind`/`enemy_hp`, emit `ActionTaken`. Death (HP→0) finalizes the run locally and emits `RunEnded { died: true }` — **no L2→L1 message**. | tx-per-action play + death |
| B.4 | `extract()` (alive, not in combat) → compute `score = DEPTH_WEIGHT·depth + gold`, `send_message_to_l1(config.registry, [player_l1, score, loot])`, emit `RunEnded { died: false }`, clear run. | bank send (extract only) |
| B.5 | Unit tests: full run lifecycle, combat/flee, trap, death = forfeit + no message, extract = message with correct payload, score formula, invalid-action reverts, one-active-run guard. | `sozo test` green |

#### Workstream C: Client + docs
**Dependencies:** Phase 0 (uses `deployments.json` shape; real addresses at merge)
· **Can parallelize with:** A, B

| Task | Description | Output |
|------|-------------|--------|
| C.1 | `app/src/chain.ts`: data layer — Torii SQL helpers (run state, action feed, leaderboard, bank feed), RPC reads (USDC/GAME balance + allowance, piltover `get_state`, appchain tip, pending entries). | data layer |
| C.2 | Write path: `buyToken`, `devMint`, `approve`, `enter`, appchain action calls, `extract`, `claimRun`; `withSepoliaLock` nonce mutex. | write helpers |
| C.3 | `app/src/wallet.tsx`: Controller **Sepolia-only** (settlement chain id = SN_SEPOLIA, real network) for buy/approve/enter/claim; dev key for appchain actions. Session policies cover USDC approve, GAME approve, buy, enter, claim. | wallet layer |
| C.4 | `app/src/App.tsx`: dungeon UI (board, HP/gold/depth, action buttons, action feed), funding panel (balances, buy/dev-mint), leaderboard, settled/tip gauge; poll loop. | UI |
| C.5 | Docs under `docs/` mirroring the first example (architecture, services, contracts, deployment, client, README), highlighting the Sepolia-settlement + USDC/GAME-token differences. | guide |

#### Workstream D: Orchestration
**Dependencies:** Phase 0; needs contract tags/paths from A & B to finalize migrate order
· **Can parallelize with:** C (and with A/B once their tags are known)

| Task | Description | Output |
|------|-------------|--------|
| D.1 | `scripts/deploy.ts` + `scripts/lib.ts`: deploy `GameToken`→`TokenSale`→`score` world, migrate appchain `game` world (init `registry`=score system), then `Entry` (needs appchain game system + piltover); grant GAME minter to `TokenSale` + `score` claim; record all addresses. | deploy script |
| D.2 | `up.sh`: preflight → mock TEE registry (Sepolia) → `init rollup --tee` (Sepolia) → base `deployments.json` → appchain Katana (`:5070`, `--tee mock --dev --dev.no-fee --messaging.enabled`) → saya-tee (`--mock-prove`, settlement = Sepolia) → migrate (D.1) → Torii ×2 (Sepolia score `:8091`, appchain game `:8092`) → client (`:3002`). Ports per the distinct band above (no local settlement node). | `up.sh` |
| D.3 | `down.sh` + run dir/log hygiene; clear messaging on the appchain not needed (fresh chain each run). | `down.sh` |

---

### Merge Phase

#### Phase N: Integration & cross-wiring
**Dependencies:** A, B, C, D

| Task | Description | Output |
|------|-------------|--------|
| N.1 | Resolve cross-chain init args end-to-end: score system addr → appchain `game` `dojo_init`; appchain game system addr → `Entry` config + `score.claim_run` `from_address` at call time. | wired deploys |
| N.2 | Confirm the L1→L2 selector (`mint_run`) and the L2→L1 payload `[player, score, loot]` match on both ends. | message parity |
| N.3 | Point the client at real `deployments.json`; verify Controller (Sepolia) signs buy/enter/bank and the dev key drives appchain actions. | wired client |
| N.4 | First full live run-through on Sepolia; fix gas/nonce/timing issues. | green E2E |

---

## Testing and Validation
- **Unit (Cairo):** `sozo test` per world/contract (A.5, B.5) — sale math, minter
  auth, entry charge + message, run lifecycle, score formula, payload shape, claim
  consume + reward.
- **Integration (scripted):** a `scripts/` driver that runs dev-mint → enter → N
  actions → extract → wait-settle → claim against the live stack, asserting balances,
  Torii rows, and `piltover.get_state` advancement.
- **Manual E2E:** the client flow on Sepolia with the Controller, end to end.
- **Regression:** USDC purchase path (with real test USDC) at least once; dev-mint
  path as the default playable path.

## Rollout and Migration
- New, isolated directory; no changes to the first example. No production rollout.
- Bring-up is gated on the operator env being set (real Sepolia). Document the
  funding requirements prominently in the README.
- "Rollback" = `./down.sh` (kills local processes); on-Sepolia deploys are
  immutable test artifacts (note their addresses; redeploy on schema change).

## Verification Checklist
- [ ] `examples/cross-chain-dungeon/up.sh` brings up: appchain `:5070`, saya, Torii
      `:8091`/`:8092`, client `:3002`; piltover + mock TEE live on Sepolia. No port
      clash with `cross-chain-game` running concurrently.
- [ ] `deployments.json` has real addresses for piltover, GameToken, TokenSale,
      Entry, score world+system, appchain game world+system, USDC.
- [ ] Dev-mint: GAME `balance_of(player)` increases (RPC).
- [ ] Buy: with test USDC, `approve` + `buy` decreases USDC and increases GAME at
      the fixed `RATE`; USDC treasury balance rises.
- [ ] Enter: GAME balance drops by `ENTRY_FEE`; piltover emits `MessageSent`; the
      appchain relays `mint_run`; `RunState` row appears in the appchain Torii.
- [ ] Play: each action tx mutates `RunState` and adds an `ActionTaken` row;
      `Stats.total_actions` increments; invalid actions revert.
- [ ] Death: HP→0 emits `RunEnded { died: true }`, clears the run, sends **no** L2→L1
      message and mints no reward (haul forfeited).
- [ ] Extract: `RunEnded { died: false }` emitted; L2→L1 message sent.
- [ ] Settle: `piltover.get_state()` (settled height) advances past the run's block.
- [ ] Bank: `claim_run` succeeds; `score-Leaderboard` row updates; GAME reward
      minted (balance ticks up by `score * REWARD_RATE`).
- [ ] Controller signs buy/approve/enter/claim on Sepolia; dev key signs appchain
      actions; reload preserves UI state (Torii-derived).

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| saya `update_state` gas on Sepolia per settled batch (tx-per-action = many blocks) drains the saya account | High | High | Fund saya well; tune appchain block production / saya settlement cadence so it batches; document expected burn; consider batching actions per block. |
| saya/operator **shared account nonce contention** on Sepolia (same failure mode the first example hit) | Med | High | Give saya a dedicated funded Sepolia account; serialize operator-side writes. |
| saya keccak-vs-Poseidon hash bug stalls every entry's settlement | High if unpatched | High | Reuse the saya Poseidon patch; verify on first L1→L2 round-trip. |
| Real USDC unavailable to the player | High | Med | Dev-mint GAME bypasses USDC; document the USDC faucet path as optional. |
| Wrong/!verified USDC address on Sepolia | Med | High | Verify the canonical Circle USDC Sepolia address before wiring; make it env-config. |
| Sepolia latency/instability makes bank slow or flaky | Med | Med | UI pending→settled gauge; retries on claim; poll with backoff. |
| `katana init rollup` against real Sepolia fails (account funding, chain-id, fact registry) | Med | High | Pre-flight balance + chain-id checks in `up.sh`; clear error messages. |
| Controller session policies miss an entrypoint (approve/buy/enter/claim) → per-tx popups or failures | Med | Low | Enumerate all Sepolia entrypoints in policies; test the session. |
| Dev key acting on any player's run (identity simplification) misread as production-safe | Low | Low | Document the simplification explicitly in docs. |
| GAME_TOKEN minter authorization too broad (anyone mints) | Low | High | Restrict mint to TokenSale + score-claim + owner dev-mint; test unauthorized mint reverts. |

## Open Questions
- [ ] **Canonical USDC address on Starknet Sepolia** — confirm the real Circle USDC
      test-token address (and decimals: USDC is 6) before wiring; set in env. (Do not
      hard-code an unverified address.)
- [ ] **saya account model** — dedicated saya Sepolia account vs shared operator
      (recommend dedicated, per the first example's contention bug).
- [ ] **Action→block cadence** — do we let each action be its own block (simple, but
      many `update_state` settlements), or tune block time so saya batches? Affects
      saya gas materially.
- [ ] **Entry seed source** — derive the run seed on L1 (block hash + player) and pass
      it in the message, or derive on L2 at `mint_run`? (Leaning: pass from L1 so the
      client can preview.)
- [ ] **`DEPTH_WEIGHT` / `REWARD_RATE` / `ENTRY_FEE` / `RATE` + room probabilities** —
      pick concrete values so the economy holds (shallow extract ≈ refund, deep
      extract profits, death is a net loss). Gameplay constants in Game Design are
      v1 defaults.
- [ ] **`extract()` availability** — anywhere out of combat (current design) vs only
      in empty/shrine rooms (more tension). Leaning: anywhere out of combat.
- [ ] **One active run per player** (keyed `RunState` by `player_l1`) vs multiple
      concurrent runs (keyed by `run_id`). v1 assumes one; confirm.

## Decision Log

| Decision | Rationale | Alternatives Considered |
|----------|-----------|------------------------|
| Settle to real Sepolia, keep appchain local | The whole point of the example; appchain stays cheap/fast | Two local Katanas (the first example) |
| Custom GAME_TOKEN bought with USDC; charge entry in GAME | User's choice; cleanly demonstrates the external-USDC dependency with a game-owned currency + a dev-mint shortcut | Gate/charge directly in USDC |
| Mint-on-purchase fixed rate + dev-mint | Can't run dry; one-click playable without USDC | Pre-minted treasury (can exhaust) |
| Mock-prove + mock TEE registry on Sepolia | Settles to a real public chain without Atlantic cost/latency | Real Atlantic SP1/STARK proving (declined) |
| Tx-per-action dungeon | Maximizes appchain operations (the stated goal) | Tx-per-room / one-shot resolve |
| Bank = score + proportional GAME reward | Closes a spend-to-enter / earn-on-win loop | Score-only / loot-as-L1-items |
| Push-your-luck: death forfeits + does NOT settle; only extract commits to Sepolia | Makes the L2→L1 commit the core gameplay decision ("appchain value is provisional until settled") | Death also settles a depth-only score (future "deepest dive" board) |
| Controller on Sepolia only | Real network the keychain knows; avoids the unresolved local-appchain keychain limitation | Controller on both (open risk) / dev-only |
| Deterministic pseudo-random (v1) | Matches first example, no extra infra | Cartridge VRF (future) |
| Reuse first example's worlds/services/client shape | Proven, documented, lowers risk | Greenfield architecture |
