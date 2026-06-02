# Contracts (Dojo worlds + the token economy)

[← services](./services.md) · Next: [deployment →](./deployment.md)

Two Dojo worlds (`cairo/game` on the appchain, `cairo/score` — the **bank** —
on Sepolia) and four plain Starknet contracts (`cairo/token`: `game_token`,
`gold_token`, `token_sale`, `entry`). The Dojo building blocks (models, systems,
`dojo_init`, writer permissions) are covered in the
[cross-chain-game contracts chapter](../../cross-chain-game/docs/contracts.md) — here
we focus on what's specific to this demo: the dungeon, the two-token economy
(**GAME** to play, **GOLD** to win), and the cross-chain wiring.

## The dungeon game

The appchain world (`cairo/game/src/lib.cairo`, namespace `game`) is a push-your-luck
roguelite. State is one model per player:

```cairo
#[dojo::model]
pub struct RunState {
    #[key] pub player: felt252,   // the settlement-layer player (from the entry message)
    pub alive: bool, pub depth: u32, pub hp: u32, pub max_hp: u32,
    pub gold: u64, pub room_kind: u8, pub enemy_hp: u32, pub potions: u32,
    pub seed: felt252, pub action_count: u64, pub run_no: u64,
}
```

**Runs are keyed by the settlement-layer player, not the caller.** Appchain actions
are signed by the local dev account, so each action takes a `player` argument and
operates on that player's run. A deliberate demo simplification (the dev key can act
on any run); the vault, leaderboard, and bank still key on the real player because the
entry message and the withdraw payload both carry it.

**One transaction per action.** The `IDungeon` interface is `move_room`, `attack`,
`loot`, `use_item`, `extract`, `withdraw` — each its own appchain tx. Rooms are
rolled on entry (monster 45% / treasure 25% / trap 15% / shrine 10% / empty 5%);
combat, traps, and loot scale with depth. Randomness is `poseidon(seed, action_count)`
— deterministic per run (a future upgrade is Cartridge VRF). Every action emits an
`ActionTaken` event (the message feed) keyed by a global sequence, carrying the run's
`run_no` so the client can group the feed by run without correlating events.

**The GOLD vault.** Gold collected in a run is realized only by **extracting**. Two
per-player models hold the cross-run state on the appchain:

```cairo
#[dojo::model] pub struct Vault       { #[key] player: felt252, gold: u64, withdraw_no: u64 }
#[dojo::model] pub struct Leaderboard { #[key] player: felt252, best_score: u64, runs: u64, total_gold: u64 }
```

**How a run ends — the cross-chain lesson:**

- **`extract` (alive) banks into the vault.** It adds `run.gold` to the player's
  `Vault.gold` (L2, no message), records the run on the leaderboard, and emits
  `RunEnded { died: false }`. Accumulates across many runs.
- **Death (HP → 0) is local and forfeits.** A killing blow ends the run, emits
  `RunEnded { died: true }`, and the in-progress gold is lost — it never reached the
  vault. Already-extracted gold is safe. The player is still out the entry fee.

So **extract** is the push-your-luck decision (lock this run's gold into the vault, or
push deeper and risk losing it). Committing the vault to L1 is a separate step —
**withdraw** — so a player banks many runs at once.

## L1 → L2: start a run from an `#[l1_handler]`

The instant direction. `entry.enter()` on Sepolia charges the fee and calls
piltover's `send_message_to_appchain`; the appchain relays it into:

```cairo
#[l1_handler]
fn mint_run(ref self: ContractState, from_address: felt252, player: felt252, seed: felt252) { … }
```

`from_address` (injected by the relayer) is the `entry` contract — provenance the
appchain can trust. `player` and `seed` come from the payload. The selector
`mint_run` must match what `entry` sends (`selector!("mint_run")`).

## L2 → L1: withdraw the vault, bank on Sepolia

The settled direction, two halves — **but batched**: one withdrawal banks the whole
accumulated vault, not one message per run. **Send** from the appchain (`withdraw`):

```cairo
send_message_to_l1_syscall(config.registry.into(),
    array![player, amount.into(), withdraw_no.into()].span()).unwrap_syscall();
// then: vault.gold = 0; vault.withdraw_no += 1; emit Withdrawal{..}
```

`amount` is the entire `Vault.gold`; `withdraw_no` is a per-player nonce so each of a
player's withdrawals is a distinct (consumable) message.

**Consume** in the `bank` world (`cairo/score/src/lib.cairo`):

```cairo
piltover.consume_message_from_appchain(from_address, array![player, amount, withdraw_no].span());
// then: mint GOLD = amount * reward_per_gold; emit Banked{..}
IGoldTokenMintDispatcher { contract_address: config.gold_token }.mint(to, minted);
```

**The payload contract is sacred:** `[player, amount, withdraw_no]` must be
reconstructed exactly on both ends (and `from_address` is the appchain game system),
or the consume reverts. Minted GOLD is `amount * reward_per_gold` in GOLD base units
(1 gold → 1 GOLD by default).

**The leaderboard lives entirely on the appchain (L2).** It's a per-player
`game-Leaderboard` model, updated on every run end with the run's `score`
(`DEPTH_WEIGHT*depth + gold`); the client ranks players by `best_score`. The bank
world holds no scoring — it's purely the settlement-side GOLD mint. The client
reconciles each L2 `Withdrawal` against its L1 `Banked` (matched on `withdraw_no`) to
drive the bank button.

## The token economy — two tokens

`cairo/token/src/lib.cairo` holds two ERC20s with distinct roles plus two plain
contracts:

- **`game_token` (GAME, "Dungeon Credit")** — the **entry credit**. `mint` is
  minter-only (the sale); `set_minter` owner-only; `dev_mint` is an open faucet
  (dev only) so you can play without real USDC.
- **`gold_token` (GOLD, "Dungeon Gold")** — the **winnings**. Minted on L1 only when
  a player banks; the bank world is its sole minter. **No faucet** — GOLD is earned.
- **`token_sale`** — `buy(usdc_amount)` does `USDC.transfer_from(buyer, treasury,
  usdc_amount)` then mints `usdc_amount * rate` GAME. This is the **external
  dependency**: USDC is a contract the demo references by address, never deploys.
- **`entry`** — `enter()` pulls `entry_fee` GAME from the caller and **burns it**
  (`game_token.burn`, i.e. a transfer to the zero address — the fee leaves
  circulation), then sends the `mint_run` message to the appchain game system. The
  caller is the run's player.

So the loop is **spend GAME to play, earn GOLD to keep**: USDC → GAME (sale) → enter
→ collect gold → extract (vault) → withdraw + bank → GOLD on L1.

## Wiring the worlds together

The same ordering trick as cross-chain-game: a world's `dojo_init` that needs
another's address goes later. Here:

1. Deploy `game_token` (GAME) and `gold_token` (GOLD).
2. Migrate `bank` on Sepolia — `dojo_init(piltover, gold_token, reward_per_gold)`.
3. Migrate `game` on the appchain — `dojo_init(registry = bank system)`, so
   `withdraw` knows where to send.
4. Deploy `token_sale` (GAME, USDC) and `entry` — `entry` needs piltover **and** the
   appchain game system address.
5. Grant `game_token` minter → the sale; `gold_token` minter → the bank system.

The reverse dependency (the bank consumer needs the appchain sender's address) is
supplied by the **client at call time** as `from_address`, so there's no deploy
cycle. Full order in [deployment.md](./deployment.md).

## The message-hash gotcha

A Starknet-settled appchain hashes **L1→L2** messages with **Poseidon**; Ethereum
L1s use keccak. saya 0.4.0 ships keccak, so on a Starknet settlement chain
(including Sepolia) the hash won't match and **every entry stalls in settlement**.
Use the patched saya — see [`../../cross-chain-game/saya-patch`](../../cross-chain-game/saya-patch)
and [services.md](./services.md#saya--the-prover-now-settling-to-a-real-chain).

Next: [build, deploy, and run the stack →](./deployment.md)
