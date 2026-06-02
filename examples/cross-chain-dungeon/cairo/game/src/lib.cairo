//! Appchain ("L2") dungeon world — a push-your-luck roguelite, as a Dojo system.
//!
//! The cross-chain loop:
//! 1. **Enter** (L1 → L2): the settlement layer charges GAME_TOKEN and relays a
//!    message into the `mint_run` `#[l1_handler]`, which starts a run for the
//!    player at the dungeon entrance.
//! 2. **Play** (L2): one transaction per action — `move_room`, `attack`, `loot`,
//!    `use_item`. Each rolls a pseudo-random outcome and mutates the run. Risk and
//!    reward both climb with depth.
//! 3. **Extract** (L2): `extract` ends the run *alive* and banks its GOLD into the
//!    player's on-chain **vault**, accumulating across many runs. No L1 message yet.
//! 4. **Withdraw / bank** (L2 → L1): `withdraw` publishes one message carrying the
//!    *whole* vault total to the settlement `bank` world, which mints that much GOLD
//!    on L1, then resets the vault. Players bank once for many runs.
//!
//! **Death is local.** If HP hits 0 the run ends on the appchain only — the
//! in-progress run's gold is forfeit (it never reached the vault). Already-extracted
//! gold is safe in the vault. That asymmetry is the lesson: appchain value is
//! provisional until you commit it to L1 — here, by banking the vault.
//!
//! Runs are keyed by `player` (the settlement-layer address carried in the entry
//! message), not by the caller — appchain actions are signed by the dev key here,
//! so the client passes which player's run to act on. A deliberate demo
//! simplification (the dev key can act on any run); see PLAN.md "Identity mapping".
//!
//! State lives in Dojo models (indexed by Torii); progress is reported through
//! Dojo events so the frontend can rebuild its feeds from the indexer.

#[starknet::interface]
pub trait IDungeon<T> {
    /// Descend one room. If a monster blocks the way this is a *flee attempt*
    /// (may fail and cost HP). Otherwise advances a depth and rolls the new room,
    /// applying on-entry effects (trap damage, shrine heal, monster ambush).
    fn move_room(ref self: T, player: felt252);
    /// Strike the blocking monster. It strikes back. Repeat until it dies (drops
    /// gold) or your HP reaches 0 (death — forfeit). Panics if not in combat.
    fn attack(ref self: T, player: felt252);
    /// Grab the loot in a treasure room (small mimic chance bites back). Panics if
    /// the current room holds no treasure.
    fn loot(ref self: T, player: felt252);
    /// Quaff a potion to heal. Panics if you hold none or are already at full HP.
    fn use_item(ref self: T, player: felt252);
    /// Climb out alive: bank this run's GOLD into the player's vault (accumulates
    /// across runs) and record it on the leaderboard. Sends no L1 message. Panics if
    /// dead or mid-combat. Returns the gold added to the vault.
    fn extract(ref self: T, player: felt252) -> u64;
    /// Bank the vault to L1: send one message `[player, amount, withdraw_no]` to the
    /// settlement `bank` world (which mints that much GOLD), then reset the vault.
    /// Panics if the vault is empty. Returns the amount sent.
    fn withdraw(ref self: T, player: felt252) -> u64;
}

#[dojo::contract]
pub mod game {
    use core::poseidon::poseidon_hash_span;
    use dojo::event::EventStorage;
    use dojo::model::ModelStorage;
    use dojo::world::WorldStorage;
    use starknet::syscalls::send_message_to_l1_syscall;
    use starknet::{ContractAddress, SyscallResultTrait};
    use super::IDungeon;

    /// Singleton key for the per-world counters and config models.
    const SINGLETON: u8 = 0;

    /// Room kinds (stored on the run; mirrored by the client for rendering).
    const ENTRANCE: u8 = 0;
    const MONSTER: u8 = 1;
    const TREASURE: u8 = 2;
    const TRAP: u8 = 3;
    const SHRINE: u8 = 4;
    const EMPTY: u8 = 5;

    const MAX_HP: u32 = 100;
    /// score = DEPTH_WEIGHT * depth + gold (on extract).
    const DEPTH_WEIGHT: u64 = 80;

    /// Running counters for the whole world (one row, keyed by `SINGLETON`).
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct Stats {
        #[key]
        pub id: u8,
        pub total_runs: u64,
        pub active_runs: u64,
        pub total_actions: u64,
        pub total_banked: u64,
        pub total_ended: u64,
    }

    /// Settlement `bank` system this world withdraws to (set at init).
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct GameConfig {
        #[key]
        pub id: u8,
        pub registry: ContractAddress,
    }

    /// Per-player GOLD vault: gold extracted from runs accumulates here (L2) until
    /// the player banks it to L1 via `withdraw`. `withdraw_no` is a per-player nonce
    /// that makes each withdraw's L2→L1 message unique.
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct Vault {
        #[key]
        pub player: felt252,
        pub gold: u64,
        pub withdraw_no: u64,
    }

    /// Per-player leaderboard, lives entirely on the appchain (L2). Ranked off-chain
    /// by `best_score` (a run's score = DEPTH_WEIGHT * depth + gold). Updated on
    /// every run end (extract or death).
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct Leaderboard {
        #[key]
        pub player: felt252,
        pub best_score: u64,
        pub runs: u64,
        pub total_gold: u64,
    }

    /// One live run, keyed by the settlement-layer player address. `alive == false`
    /// (the default for an unseen key) means "no active run".
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct RunState {
        #[key]
        pub player: felt252,
        pub alive: bool,
        pub depth: u32,
        pub hp: u32,
        pub max_hp: u32,
        pub gold: u64,
        pub room_kind: u8,
        pub enemy_hp: u32,
        pub potions: u32,
        pub seed: felt252,
        pub action_count: u64,
        pub run_no: u64,
    }

    /// Emitted when an entry message is relayed from L1 and a run begins. Keyed by
    /// the unique run sequence so Torii keeps one row per run.
    #[derive(Copy, Drop, Serde)]
    #[dojo::event]
    pub struct RunStarted {
        #[key]
        pub run_no: u64,
        pub player: felt252,
        pub seed: felt252,
    }

    /// Emitted on every play action — the roguelike message feed. Keyed by the
    /// unique action sequence so Torii keeps one row per action.
    #[derive(Copy, Drop, Serde)]
    #[dojo::event]
    pub struct ActionTaken {
        #[key]
        pub action_no: u64,
        pub player: felt252,
        pub run_no: u64,
        pub kind: felt252,
        pub outcome: felt252,
        pub depth: u32,
        pub hp: u32,
        pub gold: u64,
    }

    /// Emitted when a run ends — both on extract (`died: false`) and on death
    /// (`died: true`). Extract banks `loot` gold into the vault; death forfeits it.
    /// Keyed by the unique end sequence.
    #[derive(Copy, Drop, Serde)]
    #[dojo::event]
    pub struct RunEnded {
        #[key]
        pub end_no: u64,
        pub player: felt252,
        pub score: u64,
        pub loot: u64,
        pub died: bool,
    }

    /// Emitted when a player banks their vault to L1. Keyed by the global bank
    /// sequence so Torii keeps one row per withdrawal. `withdraw_no` is the
    /// per-player nonce carried in the L2→L1 payload (so the L1 bank can reconstruct
    /// and consume the message).
    #[derive(Copy, Drop, Serde)]
    #[dojo::event]
    pub struct Withdrawal {
        #[key]
        pub bank_no: u64,
        pub player: felt252,
        pub amount: u64,
        pub withdraw_no: u64,
    }

    /// Record the settlement `score` system address and seed the counters.
    fn dojo_init(self: @ContractState, registry: ContractAddress) {
        let mut world = self.world_default();
        world.write_model(@GameConfig { id: SINGLETON, registry });
        world
            .write_model(
                @Stats {
                    id: SINGLETON,
                    total_runs: 0,
                    active_runs: 0,
                    total_actions: 0,
                    total_banked: 0,
                    total_ended: 0,
                },
            );
    }

    /// Phase 1 — entry relayed from L1. The messaging service prepends the
    /// settlement-side `Entry` contract as `from_address`; `player` and `seed`
    /// come from the payload. Starts (or restarts) the player's run at the entrance.
    #[l1_handler]
    fn mint_run(ref self: ContractState, from_address: felt252, player: felt252, seed: felt252) {
        let mut world = self.world_default();

        let mut stats: Stats = world.read_model(SINGLETON);
        stats.total_runs += 1;
        stats.active_runs += 1;
        world.write_model(@stats);

        world
            .write_model(
                @RunState {
                    player,
                    alive: true,
                    depth: 0,
                    hp: MAX_HP,
                    max_hp: MAX_HP,
                    gold: 0,
                    room_kind: ENTRANCE,
                    enemy_hp: 0,
                    potions: 1,
                    seed,
                    action_count: 0,
                    run_no: stats.total_runs,
                },
            );

        world.emit_event(@RunStarted { run_no: stats.total_runs, player, seed });
    }

    #[abi(embed_v0)]
    impl DungeonImpl of IDungeon<ContractState> {
        fn move_room(ref self: ContractState, player: felt252) {
            let mut world = self.world_default();
            let mut run: RunState = world.read_model(player);
            assert(run.alive, 'No active run');
            run.action_count += 1;
            let action_no = self.bump_actions();

            // Blocked by a monster → flee attempt.
            if run.enemy_hp > 0 {
                if rng(run.seed, run.action_count, 100) < 55 {
                    run.enemy_hp = 0; // escaped — fall through and advance
                } else {
                    let dmg = enemy_dmg(run.depth, run.seed, run.action_count);
                    if dmg >= run.hp {
                        self.end_run_dead(player, run);
                        return;
                    }
                    run.hp -= dmg;
                    run.room_kind = MONSTER;
                    world.write_model(@run);
                    self.log(action_no, player, 'move', 'flee_fail', run);
                    return;
                }
            }

            // Advance one room and roll its contents.
            run.depth += 1;
            let kind = roll_room(run.seed, run.action_count);
            run.room_kind = kind;
            let mut outcome = 'enter';

            if kind == MONSTER {
                run.enemy_hp = enemy_hp(run.depth);
                outcome = 'ambush';
            } else if kind == TRAP {
                let dmg = trap_dmg(run.depth, run.seed, run.action_count);
                if dmg >= run.hp {
                    self.end_run_dead(player, run);
                    return;
                }
                run.hp -= dmg;
                outcome = 'trap';
            } else if kind == SHRINE {
                let heal = 10 + rng(run.seed, run.action_count, 16).try_into().unwrap();
                run.hp = cap(run.hp + heal, run.max_hp);
                outcome = 'shrine';
            } else if kind == TREASURE {
                outcome = 'treasure';
            }

            world.write_model(@run);
            self.log(action_no, player, 'move', outcome, run);
        }

        fn attack(ref self: ContractState, player: felt252) {
            let mut world = self.world_default();
            let mut run: RunState = world.read_model(player);
            assert(run.alive, 'No active run');
            assert(run.enemy_hp > 0, 'Nothing to attack');
            run.action_count += 1;
            let action_no = self.bump_actions();

            // You strike.
            let hit = 12 + rng(run.seed, run.action_count, 9).try_into().unwrap();
            if hit >= run.enemy_hp {
                // Monster falls — collect gold, room clears.
                run.enemy_hp = 0;
                run.room_kind = EMPTY;
                run.gold += kill_gold(run.depth, run.seed, run.action_count);
                world.write_model(@run);
                self.log(action_no, player, 'attack', 'kill', run);
                return;
            }
            run.enemy_hp -= hit;

            // It strikes back.
            let back = enemy_dmg(run.depth, run.seed, run.action_count + 1);
            if back >= run.hp {
                self.end_run_dead(player, run);
                return;
            }
            run.hp -= back;
            world.write_model(@run);
            self.log(action_no, player, 'attack', 'trade', run);
        }

        fn loot(ref self: ContractState, player: felt252) {
            let mut world = self.world_default();
            let mut run: RunState = world.read_model(player);
            assert(run.alive, 'No active run');
            assert(run.room_kind == TREASURE, 'No treasure here');
            run.action_count += 1;
            let action_no = self.bump_actions();

            // Mimic! ~18% of chests bite back.
            if rng(run.seed, run.action_count, 100) < 18 {
                let dmg = 10 + 2 * run.depth;
                run.room_kind = EMPTY;
                if dmg >= run.hp {
                    self.end_run_dead(player, run);
                    return;
                }
                run.hp -= dmg;
                world.write_model(@run);
                self.log(action_no, player, 'loot', 'mimic', run);
                return;
            }

            run.gold += treasure_gold(run.depth, run.seed, run.action_count);
            let mut outcome = 'gold';
            if rng(run.seed, run.action_count + 7, 100) < 40 {
                run.potions += 1;
                outcome = 'gold_potion';
            }
            run.room_kind = EMPTY; // looted once
            world.write_model(@run);
            self.log(action_no, player, 'loot', outcome, run);
        }

        fn use_item(ref self: ContractState, player: felt252) {
            let mut world = self.world_default();
            let mut run: RunState = world.read_model(player);
            assert(run.alive, 'No active run');
            assert(run.potions > 0, 'No potions');
            assert(run.hp < run.max_hp, 'Already at full HP');
            run.action_count += 1;
            let action_no = self.bump_actions();

            run.potions -= 1;
            run.hp = cap(run.hp + 35, run.max_hp);
            world.write_model(@run);
            self.log(action_no, player, 'use_item', 'heal', run);
        }

        fn extract(ref self: ContractState, player: felt252) -> u64 {
            let mut world = self.world_default();
            let mut run: RunState = world.read_model(player);
            assert(run.alive, 'No active run');
            assert(run.enemy_hp == 0, 'Cannot extract in combat');

            let gold = run.gold;
            let score = DEPTH_WEIGHT * run.depth.into() + gold;

            // Bank this run's gold into the player's vault (L2, no L1 message yet).
            let mut vault: Vault = world.read_model(player);
            vault.gold += gold;
            world.write_model(@vault);

            run.alive = false;
            world.write_model(@run);

            let mut stats: Stats = world.read_model(SINGLETON);
            stats.active_runs -= 1;
            stats.total_ended += 1;
            world.write_model(@stats);

            self.record_run(player, score, gold);
            world
                .emit_event(
                    @RunEnded { end_no: stats.total_ended, player, score, loot: gold, died: false },
                );
            gold
        }

        fn withdraw(ref self: ContractState, player: felt252) -> u64 {
            let mut world = self.world_default();
            let mut vault: Vault = world.read_model(player);
            assert(vault.gold > 0, 'Vault is empty');

            let amount = vault.gold;
            let withdraw_no = vault.withdraw_no;

            // Publish one message to L1 carrying the whole vault. `to_address` is the
            // settlement `bank` system; the payload `[player, amount, withdraw_no]`
            // must match what `bank::bank` reconstructs, or the consume reverts. The
            // nonce keeps each of a player's withdrawals a distinct message.
            let config: GameConfig = world.read_model(SINGLETON);
            send_message_to_l1_syscall(
                config.registry.into(),
                array![player, amount.into(), withdraw_no.into()].span(),
            )
                .unwrap_syscall();

            vault.gold = 0;
            vault.withdraw_no += 1;
            world.write_model(@vault);

            let mut stats: Stats = world.read_model(SINGLETON);
            stats.total_banked += 1;
            world.write_model(@stats);

            world
                .emit_event(
                    @Withdrawal { bank_no: stats.total_banked, player, amount, withdraw_no },
                );
            amount
        }
    }

    #[generate_trait]
    impl InternalImpl of InternalTrait {
        /// The dungeon world uses the `"game"` namespace.
        fn world_default(self: @ContractState) -> WorldStorage {
            self.world(@"game")
        }

        /// Increment the global action counter and return the new value (used as
        /// the `ActionTaken` key so every action gets its own indexed row).
        fn bump_actions(self: @ContractState) -> u64 {
            let mut world = self.world_default();
            let mut stats: Stats = world.read_model(SINGLETON);
            stats.total_actions += 1;
            world.write_model(@stats);
            stats.total_actions
        }

        /// Emit the per-action feed event from the post-mutation run snapshot.
        fn log(
            self: @ContractState,
            action_no: u64,
            player: felt252,
            kind: felt252,
            outcome: felt252,
            run: RunState,
        ) {
            let mut world = self.world_default();
            world
                .emit_event(
                    @ActionTaken {
                        action_no,
                        player,
                        run_no: run.run_no,
                        kind,
                        outcome,
                        depth: run.depth,
                        hp: run.hp,
                        gold: run.gold,
                    },
                );
        }

        /// Record a finished run on the per-player leaderboard (L2): bump the run
        /// count, add the gold kept (0 on death), and raise the best score.
        fn record_run(self: @ContractState, player: felt252, score: u64, gold_kept: u64) {
            let mut world = self.world_default();
            let mut lb: Leaderboard = world.read_model(player);
            lb.runs += 1;
            lb.total_gold += gold_kept;
            if score > lb.best_score {
                lb.best_score = score;
            }
            world.write_model(@lb);
        }

        /// Finalize a dead run: forfeit the in-progress gold (it never reached the
        /// vault), end locally, **send no L2 → L1 message**. Still recorded on the
        /// leaderboard by its depth-based score.
        fn end_run_dead(self: @ContractState, player: felt252, mut run: RunState) {
            let mut world = self.world_default();
            run.alive = false;
            run.hp = 0;
            world.write_model(@run);

            let mut stats: Stats = world.read_model(SINGLETON);
            stats.active_runs -= 1;
            stats.total_ended += 1;
            world.write_model(@stats);

            let score = DEPTH_WEIGHT * run.depth.into();
            self.record_run(player, score, 0);
            world
                .emit_event(
                    @RunEnded {
                        end_no: stats.total_ended, player, score, loot: run.gold, died: true,
                    },
                );
        }
    }

    // ----- free helpers (pure) -----

    /// Pseudo-random `0..span-1` from the run seed and a nonce. Deterministic per
    /// run (note: a savvy player could pre-simulate — Cartridge VRF is the upgrade
    /// for true unpredictability; see PLAN.md).
    fn rng(seed: felt252, nonce: u64, span: u64) -> u64 {
        let h = poseidon_hash_span(array![seed, nonce.into()].span());
        let hu: u256 = h.into();
        (hu % span.into()).try_into().unwrap()
    }

    /// Weighted room roll: monster 45% / treasure 25% / trap 15% / shrine 10% /
    /// empty 5%.
    fn roll_room(seed: felt252, nonce: u64) -> u8 {
        let r = rng(seed, nonce, 100);
        if r < 45 {
            MONSTER
        } else if r < 70 {
            TREASURE
        } else if r < 85 {
            TRAP
        } else if r < 95 {
            SHRINE
        } else {
            EMPTY
        }
    }

    fn enemy_hp(depth: u32) -> u32 {
        20 + 8 * depth
    }

    fn enemy_dmg(depth: u32, seed: felt252, nonce: u64) -> u32 {
        let jitter: u32 = rng(seed, nonce + 101, 5).try_into().unwrap();
        6 + 2 * depth + jitter
    }

    fn trap_dmg(depth: u32, seed: felt252, nonce: u64) -> u32 {
        let jitter: u32 = rng(seed, nonce + 202, 5).try_into().unwrap();
        8 + 3 * depth + jitter
    }

    fn treasure_gold(depth: u32, seed: felt252, nonce: u64) -> u64 {
        10 + 5 * depth.into() + rng(seed, nonce + 303, 9)
    }

    fn kill_gold(depth: u32, seed: felt252, nonce: u64) -> u64 {
        5 + 4 * depth.into() + rng(seed, nonce + 404, 7)
    }

    /// Clamp `v` to at most `max`.
    fn cap(v: u32, max: u32) -> u32 {
        if v > max {
            max
        } else {
            v
        }
    }
}
