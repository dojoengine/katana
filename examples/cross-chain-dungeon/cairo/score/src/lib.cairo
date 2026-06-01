//! Settlement ("L1", Starknet Sepolia) score world for the L2 → L1 direction.
//!
//! `claim_run` consumes a message the appchain `game` world emitted on **extract**
//! and saya settled onto the piltover core. Consumption only succeeds once the
//! message has been registered by a settled state update, so this is the moment the
//! cross-chain round trip completes. On success it records the run on the
//! leaderboard and **mints a proportional GAME_TOKEN reward** to the player —
//! closing the spend-to-enter / earn-on-win loop.
//!
//! State lives in Dojo models (indexed by Torii) so the frontend can react to the
//! settled run.

use starknet::ContractAddress;

/// Minimal view of the piltover core's messaging interface (the only method
/// `score` needs). The full interface lives in the piltover crate.
#[starknet::interface]
pub trait IPiltoverMessaging<T> {
    fn consume_message_from_appchain(
        ref self: T, from_address: ContractAddress, payload: Span<felt252>,
    ) -> felt252;
}

/// The slice of GAME_TOKEN this world calls. `score` must be an authorized minter
/// (granted at deploy) for the reward mint to succeed.
#[starknet::interface]
pub trait IGameTokenMint<T> {
    fn mint(ref self: T, to: ContractAddress, amount: u256);
}

#[starknet::interface]
pub trait IScore<T> {
    /// Consume the settled L2 → L1 message `(player, score, loot)` emitted by the
    /// appchain `game` world at `from_address`, record it, and mint the reward.
    /// Reverts until the message has been settled onto the piltover core by saya.
    fn claim_run(
        ref self: T, from_address: ContractAddress, player: felt252, score: felt252, loot: felt252,
    );
}

#[dojo::contract]
pub mod score {
    use dojo::event::EventStorage;
    use dojo::model::ModelStorage;
    use dojo::world::WorldStorage;
    use starknet::ContractAddress;
    use super::{
        IGameTokenMintDispatcher, IGameTokenMintDispatcherTrait, IPiltoverMessagingDispatcher,
        IPiltoverMessagingDispatcherTrait, IScore,
    };

    const SINGLETON: u8 = 0;
    const BPS_DENOM: u64 = 10000;

    /// Per-player leaderboard row: best score and lifetime totals.
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct Leaderboard {
        #[key]
        pub player: felt252,
        pub best_score: u64,
        pub best_depth: u32,
        pub runs: u64,
        pub total_reward: u256,
    }

    /// Config + global claim counter (one row, keyed by `SINGLETON`).
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct ScoreConfig {
        #[key]
        pub id: u8,
        pub piltover: ContractAddress,
        pub game_token: ContractAddress,
        /// Reward = score * reward_bps / 10000 (in GAME_TOKEN base units).
        pub reward_bps: u64,
        pub total_claims: u64,
    }

    /// Emitted when a settled run is banked. Keyed by the unique claim sequence so
    /// Torii keeps one row per banked run.
    #[derive(Copy, Drop, Serde)]
    #[dojo::event]
    pub struct RunBanked {
        #[key]
        pub claim_no: u64,
        pub player: felt252,
        pub score: u64,
        pub loot: u64,
        pub reward: u256,
    }

    /// Record the piltover core, the GAME_TOKEN, and the reward rate (basis points).
    fn dojo_init(
        self: @ContractState,
        piltover: ContractAddress,
        game_token: ContractAddress,
        reward_bps: u64,
    ) {
        let mut world = self.world_default();
        world
            .write_model(
                @ScoreConfig { id: SINGLETON, piltover, game_token, reward_bps, total_claims: 0 },
            );
    }

    #[abi(embed_v0)]
    impl ScoreImpl of IScore<ContractState> {
        fn claim_run(
            ref self: ContractState,
            from_address: ContractAddress,
            player: felt252,
            score: felt252,
            loot: felt252,
        ) {
            let mut world = self.world_default();
            let mut config: ScoreConfig = world.read_model(SINGLETON);

            // Consume the settled message. The (from_address, payload) pair plus
            // this contract as the implicit `to_address` reconstruct the hash
            // registered by the settled state update; reverts if not yet settled.
            let piltover = IPiltoverMessagingDispatcher { contract_address: config.piltover };
            piltover.consume_message_from_appchain(from_address, array![player, score, loot].span());

            let score_u64: u64 = score.try_into().unwrap();
            let loot_u64: u64 = loot.try_into().unwrap();

            // Mint the reward to the player (this world must be an authorized minter).
            let reward: u256 = (score_u64 * config.reward_bps / BPS_DENOM).into();
            let to: ContractAddress = player.try_into().unwrap();
            IGameTokenMintDispatcher { contract_address: config.game_token }.mint(to, reward);

            // Update the player's leaderboard row.
            let mut row: Leaderboard = world.read_model(player);
            row.runs += 1;
            row.total_reward += reward;
            if score_u64 > row.best_score {
                row.best_score = score_u64;
            }
            let depth_u32: u32 = (score_u64 / 80).try_into().unwrap(); // mirrors DEPTH_WEIGHT=80
            if depth_u32 > row.best_depth {
                row.best_depth = depth_u32;
            }
            world.write_model(@row);

            config.total_claims += 1;
            world.write_model(@config);

            world
                .emit_event(
                    @RunBanked {
                        claim_no: config.total_claims, player, score: score_u64, loot: loot_u64,
                        reward,
                    },
                );
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
