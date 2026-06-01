# Contracts (Dojo worlds + the token economy)

[← services](./services.md) · Next: [deployment →](./deployment.md)

Two Dojo worlds (`cairo/game`, `cairo/score`) and three plain Starknet contracts
(`cairo/token`: `game_token`, `token_sale`, `entry`). The Dojo building blocks
(models, systems, `dojo_init`, writer permissions) are covered in the
[cross-chain-game contracts chapter](../../cross-chain-game/docs/contracts.md) — here
we focus on what's specific to this demo: the dungeon, the economy, and the
cross-chain wiring.

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
on any run); the leaderboard and reward still key on the real player because the
entry message and the extract payload both carry it.

**One transaction per action.** The `IDungeon` interface is `move_room`, `attack`,
`loot`, `use_item`, `extract` — each its own appchain tx. Rooms are rolled on entry
(monster 45% / treasure 25% / trap 15% / shrine 10% / empty 5%); combat, traps, and
loot scale with depth. Randomness is `poseidon(seed, action_count)` — deterministic
per run (a future upgrade is Cartridge VRF). Every action emits an `ActionTaken`
event (the message feed) keyed by a global sequence, and carries the run's
`run_no` so the client can group the feed by run without correlating events.

**How a run ends — the cross-chain lesson:**

- **`extract` (alive) settles.** It computes `score = DEPTH_WEIGHT*depth + gold`,
  calls `send_message_to_l1([player, score, loot])`, emits `RunEnded { died: false }`,
  and clears the run. This is the only path that reaches Sepolia.
- **Death (HP → 0) is local.** A killing blow finalizes the run and emits
  `RunEnded { died: true }` on the appchain **only** — no message, no reward, the
  haul forfeited. The player is still out the entry fee.

So the L2→L1 commit *is* the gameplay decision: push deeper for more, or extract to
make it real on Sepolia before a bad roll ends you.

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

## L2 → L1: extract on the appchain, bank on Sepolia

The settled direction, two halves. **Send** from the appchain:

```cairo
send_message_to_l1_syscall(config.registry.into(),
    array![player, score.into(), gold.into()].span()).unwrap_syscall();
```

**Consume** in the `score` world (`cairo/score/src/lib.cairo`):

```cairo
piltover.consume_message_from_appchain(from_address, array![player, score, loot].span());
// then: write the leaderboard row, and mint the reward
IGameTokenMintDispatcher { contract_address: config.game_token }.mint(to, reward);
```

**The payload contract is sacred:** `[player, score, loot]` must be reconstructed
exactly on both ends (and `from_address` is the appchain game system), or the
consume reverts. The reward is `score * reward_per_point` in GAME_TOKEN base units.

## The token economy

Three plain Starknet contracts in `cairo/token/src/lib.cairo`:

- **`game_token`** — an OpenZeppelin ERC20 ("Dungeon Gold" / `DGOLD`). `mint` is
  restricted to **authorized minters** (the sale and the score world, granted at
  deploy); `set_minter` is owner-only; `dev_mint` is an open faucet (dev only).
- **`token_sale`** — `buy(usdc_amount)` does `USDC.transfer_from(buyer, treasury,
  usdc_amount)` then mints `usdc_amount * rate` GAME. This is the **external
  dependency**: USDC is a contract the demo references by address, never deploys.
- **`entry`** — `enter()` charges `entry_fee` GAME via `transfer_from`, then sends
  the `mint_run` message to the appchain game system. The caller is the run's player.

Because the score world and the sale both **mint** GAME, the deploy step grants
both `set_minter` rights (see [deployment.md](./deployment.md)).

## Wiring the worlds together

The same ordering trick as cross-chain-game: a world's `dojo_init` that needs
another's address goes later. Here:

1. Deploy `game_token` (its address feeds the score world + sale).
2. Migrate `score` on Sepolia — `dojo_init(piltover, game_token, reward_per_point)`.
3. Migrate `game` on the appchain — `dojo_init(registry = score system)`, so
   `extract` knows where to send.
4. Deploy `entry` — needs piltover **and** the appchain game system address.
5. Grant `game_token` minter rights to the sale and the score system.

The reverse dependency (the score consumer needs the appchain sender's address) is
supplied by the **client at call time** as `from_address`, so there's no deploy
cycle. Full order in [deployment.md](./deployment.md).

## The message-hash gotcha

A Starknet-settled appchain hashes **L1→L2** messages with **Poseidon**; Ethereum
L1s use keccak. saya 0.4.0 ships keccak, so on a Starknet settlement chain
(including Sepolia) the hash won't match and **every entry stalls in settlement**.
Use the patched saya — see [`../../cross-chain-game/saya-patch`](../../cross-chain-game/saya-patch)
and [services.md](./services.md#saya--the-prover-now-settling-to-a-real-chain).

Next: [build, deploy, and run the stack →](./deployment.md)
