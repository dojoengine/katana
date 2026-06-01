# Querying and driving the dungeon from the client

[← deployment](./deployment.md) · [guide index](./README.md)

The client splits the same way as any appchain app (see
[cross-chain-game's client chapter](../../cross-chain-game/docs/client.md)): **send
transactions** to systems / piltover / the token contracts, **read state** from
Torii (plus a few raw RPC facts). The whole data layer is `app/src/chain.ts`; the
poll loop and UI are in `app/src/App.tsx`; the wallet is `app/src/wallet.tsx`.

This app keeps the **hand-written terminal CSS** (no tailwind/shadcn) but uses the
same **live Torii subscriptions** as cross-chain-game: `subscribeToriiUpdates`
(`chain.ts`) connects a `@dojoengine/torii-wasm` `ToriiClient` to both worlds
(`game` on the appchain, `score` on Sepolia) and refetches the instant a model is
set or an event is emitted (`onEntityUpdated` / `onEventMessageUpdated`). A slow
5s interval remains as a fallback and for the RPC-only facts that have no
subscription (token balances, the piltover settled height, the appchain tip). If
the wasm client can't connect, it logs a warning and the slow poll carries the UI.

## The read model: Torii tables + RPC facts

Reads come from Torii SQL (`GET /sql?query=…`, JSON rows; felt columns are hex
strings collapsed with `Number(BigInt(v))`):

| What | Source |
| --- | --- |
| Live run (HP/gold/depth/room/potions) | `game-RunState` model, keyed by the player |
| World counters | `game-Stats` |
| Action feed (the message log) | `game-ActionTaken` event table |
| Leaderboard | `score-Leaderboard` model (Sepolia Torii) |
| Banked runs | `score-RunBanked` event table |

`ActionTaken.kind`/`outcome` are short-string felts — the client decodes them back
to ASCII for the log (`move`, `attack`, `kill`, `trap`, `shrine`, …).

A few things aren't world state, so they bypass Torii and hit RPC directly:

- **Token balances** — `USDC.balanceOf` / `GAME_TOKEN.balanceOf` on Sepolia.
- **Settled height** — piltover `get_state()[1]` (drives the "settled N / tip M"
  gauge), and the appchain tip from `getBlockNumber`.
- **Entry fee** — `entry.entry_fee()`.

## The write path

Writes are ordinary signed transactions. Settlement-layer writes (Sepolia) are
signed by the wallet's L1 account; appchain actions by the local dev account:

- **Buy GAME (Sepolia):** `buyGame` — a multicall of `USDC.approve(sale, amt)` +
  `token_sale.buy(amt)`.
- **Dev-mint (Sepolia):** `devMint` — `game_token.dev_mint(amount)`, the no-USDC faucet.
- **Enter (L1→L2):** `enterDungeon` — multicall `game_token.approve(entry, fee)` +
  `entry.enter()`. Starts the run for the L1 signer's address.
- **Play (appchain):** `moveRoom` / `attack` / `loot` / `useItem` / `extract` — each
  calls the `game` system with the player address, signed by the dev account.
- **Bank (L2→L1):** `claimRun(player, score, loot)` — `score.claim_run` on Sepolia,
  once settled.

Two practical notes carried over from cross-chain-game:

- **Serialize same-account writes.** Buy / enter / bank are all signed by the one
  settlement account, so they funnel through a promise-chain mutex
  (`withSettlementLock`) to avoid racing the nonce.
- **A write returns before the read updates.** Actions wait for the receipt, then the
  1.5s poll loop re-reads Torii; the UI catches up a beat later (eventually consistent).

## Driving the bank step

`extract` clears the run on the appchain, so the score/loot to bank can't be read
from `RunState` afterward. Instead the client reconciles two feeds: `game-RunEnded`
(extracted runs, `died = 0`) against `score-RunBanked` (already-banked). The first
extract beyond the banked count is the **pending** one; the **Bank** button enables
once `settledBlock ≥` the extract's appchain block (saya has settled it). See
`getExtracts` / `getBankCount` in `chain.ts`.

## Wallets (operator default, optional Controller)

By default the client signs Sepolia transactions with the **operator account** (a
real funded account from `deployments.json`) — no login needed. The header **login**
button can swap in a [Cartridge Controller](https://github.com/cartridge-gg/controller).

Crucially, the Controller here is **Sepolia-only**: `StarknetConfig` is configured
with the single Sepolia chain, and the appchain play actions always use the local
dev account. There's no `switchStarknetChain`, so this sidesteps the
hosted-keychain-can't-switch-to-a-local-appchain limitation that cross-chain-game
ran into. Session policies scope the Controller to the demo's Sepolia entrypoints
(USDC/GAME `approve`, `buy`, `enter`, `claim_run`) so they're gasless session calls.

## Poll + derive

The client keeps no authoritative state of its own: one 1.5s interval re-reads
everything (run, stats, feed, leaderboard, balances, settled/tip, pending extract)
and the UI derives from that — so a page reload reproduces the same state. The
dungeon view, the vitals, the enabled/disabled action buttons (e.g. **Attack** only
in combat, **Extract** only out of combat), and the message log are all functions of
the latest read.

---

That's the loop: [architecture](./architecture.md) (two worlds, one local chain,
the token economy) → [services](./services.md) (one Katana, piltover/saya/Torii on
Sepolia) → [contracts](./contracts.md) (the dungeon + the messaging directions) →
[deployment](./deployment.md) (deploy economy + worlds) → this read/write client.
