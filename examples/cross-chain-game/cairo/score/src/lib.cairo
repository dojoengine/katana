//! Settlement ("L1") score world for the L2 -> L1 direction.
//!
//! `claim_score` consumes a message that the appchain `game` world emitted and
//! the appchain's embedded settlement service settled onto the piltover core.
//! Consumption only succeeds once the
//! message has been registered by a settled state update, so this is the moment
//! the cross-chain round trip completes. State lives in Dojo models (indexed by
//! Torii) so the frontend can react to the settled score.

use starknet::ContractAddress;

/// Minimal view of the piltover core's messaging interface (the only method
/// `score_registry` needs). The full interface lives in the piltover crate.
#[starknet::interface]
pub trait IPiltoverMessaging<T> {
    fn consume_message_from_appchain(
        ref self: T, from_address: ContractAddress, payload: Span<felt252>,
    ) -> felt252;
}

#[starknet::interface]
pub trait IScoreRegistry<T> {
    /// Consume the settled L2 -> L1 message `(player, score)` emitted by the
    /// appchain `game` world at `from_address`. Reverts until the message has
    /// been settled onto the piltover core.
    fn claim_score(ref self: T, from_address: ContractAddress, player: felt252, score: felt252);
}

#[dojo::contract]
pub mod score_registry {
    use dojo::event::EventStorage;
    use dojo::model::ModelStorage;
    use dojo::world::WorldStorage;
    use starknet::ContractAddress;
    use super::{IPiltoverMessagingDispatcher, IPiltoverMessagingDispatcherTrait, IScoreRegistry};

    /// Singleton key for the leaderboard and config models.
    const SINGLETON: u8 = 0;

    /// Aggregate of every score banked to L1 (one row, keyed by `SINGLETON`).
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct Leaderboard {
        #[key]
        pub id: u8,
        pub last_score: u64,
        pub last_player: felt252,
        pub total: u64,
    }

    /// Latest banked score per player.
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct PlayerScore {
        #[key]
        pub player: felt252,
        pub score: u64,
    }

    /// The piltover core this world consumes settled messages from (set at init).
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct ScoreConfig {
        #[key]
        pub id: u8,
        pub piltover: ContractAddress,
    }

    /// Emitted when a settled score is banked to L1. Keyed by the unique claim
    /// sequence (`new_total`) so Torii keeps one row per banked run (Dojo
    /// event-message tables upsert by key).
    #[derive(Copy, Drop, Serde)]
    #[dojo::event]
    pub struct ScoreClaimed {
        #[key]
        pub claim_no: u64,
        pub player: felt252,
        pub score: u64,
    }

    /// Record the piltover core address and seed the leaderboard.
    fn dojo_init(self: @ContractState, piltover: ContractAddress) {
        let mut world = self.world_default();
        world.write_model(@ScoreConfig { id: SINGLETON, piltover });
        world.write_model(@Leaderboard { id: SINGLETON, last_score: 0, last_player: 0, total: 0 });
    }

    #[abi(embed_v0)]
    impl ScoreRegistryImpl of IScoreRegistry<ContractState> {
        fn claim_score(
            ref self: ContractState, from_address: ContractAddress, player: felt252, score: felt252,
        ) {
            let mut world = self.world_default();

            // Consume the message from the piltover core. The (from_address, payload)
            // pair plus this contract as the implicit `to_address` reconstruct the
            // hash registered by the settled state update; reverts if not yet settled.
            let config: ScoreConfig = world.read_model(SINGLETON);
            let piltover = IPiltoverMessagingDispatcher { contract_address: config.piltover };
            piltover.consume_message_from_appchain(from_address, array![player, score].span());

            let score_u64: u64 = score.try_into().unwrap();
            world.write_model(@PlayerScore { player, score: score_u64 });

            let mut board: Leaderboard = world.read_model(SINGLETON);
            board.last_score = score_u64;
            board.last_player = player;
            board.total += 1;
            world.write_model(@board);

            world.emit_event(@ScoreClaimed { claim_no: board.total, player, score: score_u64 });
        }
    }

    #[generate_trait]
    impl InternalImpl of InternalTrait {
        /// The score world uses the `"score"` namespace.
        fn world_default(self: @ContractState) -> WorldStorage {
            self.world(@"score")
        }
    }
}
