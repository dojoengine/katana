//! Appchain ("L2") game world — the whole game loop, as a Dojo system.
//!
//! 1. **Purchase** (L1 → L2): the settlement layer relays a message into the
//!    `mint_game` `#[l1_handler]`, which adds a game to the playable pool.
//! 2. **Play** (L2): `play_game` consumes one available game, rolls a score on
//!    chain, and finishes the game.
//! 3. **Publish** (L2 → L1): `play_game` immediately emits the score to the
//!    settlement `score_registry` via `send_message_to_l1` — no separate step.
//!
//! State lives in Dojo models (indexed by Torii); progress is reported through
//! Dojo events so the frontend can rebuild its feeds from the indexer.

#[starknet::interface]
pub trait IGame<T> {
    /// Play one available game: rolls a score (1..=100), finishes the game, and
    /// publishes the score to L1 in the same transaction. Panics if no games are
    /// available to play. Returns the rolled score.
    fn play_game(ref self: T) -> u64;
}

#[dojo::contract]
pub mod game {
    use core::poseidon::poseidon_hash_span;
    use dojo::event::EventStorage;
    use dojo::model::ModelStorage;
    use dojo::world::WorldStorage;
    use starknet::syscalls::send_message_to_l1_syscall;
    use starknet::{ContractAddress, SyscallResultTrait, get_block_timestamp, get_caller_address};
    use super::IGame;

    /// Singleton key for the per-world counters and config models.
    const SINGLETON: u8 = 0;

    /// Running counters for the whole game (one row, keyed by `SINGLETON`).
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct Stats {
        #[key]
        pub id: u8,
        pub total_minted: u64,
        pub available: u64,
        pub total_played: u64,
        pub last_score: u64,
    }

    /// Settlement `score_registry` this world publishes scores to (set at init).
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct GameConfig {
        #[key]
        pub id: u8,
        pub registry: ContractAddress,
    }

    /// Per-player playable-credit balance, keyed by the L2 player address. A
    /// purchase credits the buyer (relayed from L1); a play consumes one of the
    /// caller's own credits. Credits are owned, not a shared pool.
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct Credits {
        #[key]
        pub player: felt252,
        pub balance: u64,
    }

    /// Emitted when a purchase is relayed from L1 and a game enters the pool.
    /// Keyed by the unique mint sequence so Torii keeps one row per mint
    /// (Dojo event-message tables upsert by key).
    #[derive(Copy, Drop, Serde)]
    #[dojo::event]
    pub struct GameMinted {
        #[key]
        pub mint_no: u64,
        pub buyer: felt252,
        pub game_id: felt252,
        pub available: u64,
    }

    /// Emitted when a game is played. `score` is also published to L1. Keyed by
    /// the unique play sequence so Torii keeps one row per play.
    #[derive(Copy, Drop, Serde)]
    #[dojo::event]
    pub struct GamePlayed {
        #[key]
        pub game_no: u64,
        pub player: felt252,
        pub score: u64,
    }

    /// Record the settlement registry address and seed the counters.
    fn dojo_init(self: @ContractState, registry: ContractAddress) {
        let mut world = self.world_default();
        world.write_model(@GameConfig { id: SINGLETON, registry });
        world
            .write_model(
                @Stats {
                    id: SINGLETON, total_minted: 0, available: 0, total_played: 0, last_score: 0,
                },
            );
    }

    /// Phase 1 — purchase relayed from L1. `from_address` is the L1 store contract
    /// that sent the message; the buyer/player to credit comes through the payload
    /// (so it works whether the L1 and L2 accounts are the same or not). Credits
    /// the player's own balance, not a shared pool.
    #[l1_handler]
    fn mint_game(ref self: ContractState, from_address: felt252, player: felt252, game_id: felt252) {
        let mut world = self.world_default();
        let mut stats: Stats = world.read_model(SINGLETON);
        stats.total_minted += 1;
        stats.available += 1;
        world.write_model(@stats);
        let mut credits: Credits = world.read_model(player);
        credits.balance += 1;
        world.write_model(@credits);
        world
            .emit_event(
                @GameMinted {
                    mint_no: stats.total_minted,
                    buyer: player,
                    game_id,
                    available: stats.available,
                },
            );
    }

    #[abi(embed_v0)]
    impl GameImpl of IGame<ContractState> {
        /// Phases 2 + 3 — play a game and publish its score to L1.
        fn play_game(ref self: ContractState) -> u64 {
            let mut world = self.world_default();

            // Spend one of the *caller's own* credits (not a shared pool).
            let player: felt252 = get_caller_address().into();
            let mut credits: Credits = world.read_model(player);
            assert(credits.balance > 0, 'No credits to play');
            credits.balance -= 1;
            world.write_model(@credits);

            let mut stats: Stats = world.read_model(SINGLETON);
            stats.available -= 1;
            let game_no = stats.total_played + 1;
            stats.total_played = game_no;
            let score = roll(game_no);
            stats.last_score = score;
            world.write_model(@stats);

            // Phase 3: publish the score to L1 automatically. `to_address` is the
            // settlement `score_registry`; payload `[player, score]` must match
            // what `score_registry::claim_score` passes to the piltover core.
            let config: GameConfig = world.read_model(SINGLETON);
            send_message_to_l1_syscall(config.registry.into(), array![player, score.into()].span())
                .unwrap_syscall();

            world.emit_event(@GamePlayed { game_no, player, score });
            score
        }
    }

    #[generate_trait]
    impl InternalImpl of InternalTrait {
        /// The game world uses the `"game"` namespace.
        fn world_default(self: @ContractState) -> WorldStorage {
            self.world(@"game")
        }
    }

    /// Rolls a pseudo-random score in `1..=100`. Not cryptographically secure —
    /// fine for a demo. Each play lands in its own block (provable rollup), so the
    /// block timestamp combined with the play counter varies the roll.
    fn roll(game_no: u64) -> u64 {
        let ts: felt252 = get_block_timestamp().into();
        let seed = poseidon_hash_span(array![ts, game_no.into()].span());
        let seed_u256: u256 = seed.into();
        let score: u256 = (seed_u256 % 100) + 1;
        score.try_into().unwrap()
    }
}
