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
(`game` on the appchain, `bank` on Sepolia) and refetches the instant a model is
set or an event is emitted (`onEntityUpdated` / `onEventMessageUpdated`). A slow
5s interval remains as a fallback and for the RPC-only facts that have no
subscription (token balances, the piltover settled height, the appchain tip). If
the wasm client can't connect, it logs a warning and the slow poll carries the UI.

## The read model: Torii tables + RPC facts

Reads come from Torii SQL (`GET /sql?query=…`, JSON rows; felt columns are hex
strings collapsed with `Number(BigInt(v))`):

| What | Source |
| --- | --- |
| Live run (HP/gold/depth/room/potions) | `game-RunState` model, keyed by `run_no` (a player can have several open; the lobby lists the unfinished ones by `player`) |
| GOLD vault (accumulated, unbanked) | `game-Vault` model, keyed by the player |
| Leaderboard (per player, best score) | `game-Leaderboard` model (appchain Torii), ordered by `best_score` |
| World counters | `game-Stats` |
| Action feed (the message log) | `game-ActionTaken` event table |
| Withdrawals (L2) vs banks (L1) | `game-Withdrawal` vs `bank-Banked` event tables |

`ActionTaken.kind`/`outcome` are short-string felts — the client decodes them back
to ASCII for the log (`move`, `attack`, `kill`, `trap`, `shrine`, …).

A few things aren't world state, so they bypass Torii and hit RPC directly:

- **Token balances** — `USDC` / `GAME` / `GOLD` `balanceOf` on Sepolia.
- **Settled height** — piltover `get_state()[1]` (drives the "settled N / tip M"
  gauge), and the appchain tip from `getBlockNumber`.
- **Entry fee** — `entry.entry_fee()`.

## The write path

Writes are ordinary signed transactions. Settlement-layer writes (Sepolia) are
signed by the wallet's L1 account; appchain actions by the local dev account:

- **Buy GAME (Sepolia):** `buyGame` — a multicall of `USDC.approve(sale, amt)` +
  `token_sale.buy(amt)`.
- **Dev-mint (Sepolia):** `devMint` — `game_token.dev_mint(amount)`, the no-USDC faucet.
- **New game (L1→L2):** `enterDungeon` — multicall `game_token.approve(entry, fee)` +
  `entry.enter()`. Mints a fresh run (its own `run_no`) for the L1 signer's address;
  the lobby auto-selects it once it appears. A player can keep several runs open and
  resume any of them — `listRuns(player)` feeds the lobby's continue list.
- **Play (appchain):** `moveRoom` / `attack` / `loot` / `useItem` / `extract` — each
  calls the `game` system with the **`run_no`** being played, signed by the dev
  account; `withdraw(player)` sends the whole vault to L1. `extract` banks the run's
  gold into the vault; `withdraw` sends the whole vault to L1.
- **Bank (L2→L1):** `bankMany(player, rows)` — one Sepolia multicall of `bank.bank`,
  one call per settled withdrawal; each consumes its L2→L1 message and mints GOLD.

Two practical notes carried over from cross-chain-game:

- **Serialize same-account writes.** Buy / enter / bank are all signed by the one
  settlement account, so they funnel through a promise-chain mutex
  (`withSettlementLock`) to avoid racing the nonce. The appchain play actions do the
  same (`withAppchainLock`) — necessary because the appchain mines on a 5s interval,
  so the play path also reads its nonce and fee estimate from the **pre-confirmed**
  block and resolves on `PRE_CONFIRMED`. That whole story is its own chapter:
  [interval-mining.md](./interval-mining.md).
- **A write returns before the read updates.** Actions wait for the receipt, then the
  1.5s poll loop re-reads Torii; the UI catches up a beat later (eventually consistent).

## Driving the bank step

Banking is **batched** and split across a dedicated **Bank tab** (framed as the L1
operation). Players extract many runs into the `game-Vault`, then bank once:

1. **Withdraw (L2):** when `Vault.gold > 0`, the **Withdraw** button sends one message
   with the whole vault and emits a `game-Withdrawal { amount, withdraw_no }`.
2. **Settle:** saya proves and settles the withdrawal's appchain block onto piltover.
3. **Claim (L1):** the **Claim** button calls `bankMany`, banking *every settled*
   withdrawal in one multicall (each call consumes its message and mints GOLD).

Withdraw and Claim are two explicit buttons (one per chain), not a single auto-claiming
action: the client reconciles `game-Withdrawal` (L2) against `bank-Banked` (L1) — every
withdrawal past the banked count is **unclaimed** (oldest-first). The **Claim** button
enables once the oldest has settled (`settledBlock ≥` its block) and banks all that have
settled at once; any not-yet-settled ones stay queued (an unsettled `consume` would
revert the whole multicall). The Bank-tab badge shows the total bankable gold (amber
while awaiting saya, green once a withdrawal is settled). See `readVault` /
`getWithdrawals` / `getBankCount` in `chain.ts`.

The unclaimed withdrawals render as a list under the buttons; clicking one opens a
details modal — its L2 tx (linked to the appchain explorer), appchain block + settle
progress, and the exact L2→L1 message: route (`game system → bank system`), payload
`[player, amount, withdraw_no]`, and the poseidon **message hash** piltover registers
and `bank` consumes. `withdrawalMessageHash` in `chain.ts` mirrors the cairo
`compute_message_hash_appc_to_sn` (poseidon over `[from, to, payload_len, ...payload]`).

## Wallets (operator default, optional Controller)

By default the client signs with the **operator account** (a real funded Sepolia
account from `deployments.json`) on Sepolia and the **dev account** on the appchain —
no login needed. The header **login** button can swap in a
[Cartridge Controller](https://github.com/cartridge-gg/controller) that signs on
**both chains** as one identity.

`wallet.tsx` builds a two-chain Controller: `ControllerConnector` gets both RPCs and
`StarknetConfig` both chains. It exposes two signers — `l1Account` (buy / enter /
bank on Sepolia) and `l2Account` (the play actions on the appchain). Each wraps the
raw `controller.account` and **switches the keychain's chain** around the call:
`l1Account` switches to Sepolia first (a prior play may have left it on the
appchain); `l2Account` switches to the appchain (`shortString("DUNGEON")`), executes,
then switches back to Sepolia. We use `controller.account`, not starknet-react's
account (which is pinned to the Sepolia RPC and would estimate appchain fees against
Sepolia). Session policies cover the Sepolia entrypoints **and** the game-system play
actions, so both legs are session calls.

The **player** is the Controller address (identical on both chains): `enter` mints
the run for it, and play/withdraw key on it — so a Controller plays and banks the
runs it entered. The play actions take an optional `account` in `chain.ts`
(`moveRoom(runNo, account?)` …); with the Controller it's `l2Account`, otherwise the
dev account keeps its pre-confirmed nonce/estimate fast path.

The appchain leg needs `CONTROLLER=1 ./up.sh` (paymaster + Controller classes on the
appchain) and a Cartridge Controller login — the **hosted keychain** (x.cartridge.gg) by
default, or a self-hosted keychain as a fully-local fallback. Full setup, including
funding the Controller on real Sepolia, is in [controller.md](./controller.md).

## Poll + derive

The client keeps no authoritative state of its own: one 1.5s interval re-reads
everything (run, stats, feed, leaderboard, balances, vault, settled/tip, pending
withdrawal) and the UI derives from that — so a page reload reproduces the same state. The
dungeon view, the vitals, the enabled/disabled action buttons (e.g. **Attack** only
in combat, **Extract** only out of combat), and the message log are all functions of
the latest read.

---

That's the loop: [architecture](./architecture.md) (two worlds, one local chain,
the token economy) → [services](./services.md) (one Katana, piltover/saya/Torii on
Sepolia) → [contracts](./contracts.md) (the dungeon + the messaging directions) →
[deployment](./deployment.md) (deploy economy + worlds) → this read/write client.
