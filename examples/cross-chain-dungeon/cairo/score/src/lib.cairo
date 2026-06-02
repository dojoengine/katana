//! Settlement ("L1", Starknet Sepolia) **bank** world for the L2 → L1 direction.
//!
//! `bank` consumes a message the appchain `game` world emitted on **withdraw** and
//! saya settled onto the piltover core. Consumption only succeeds once the message
//! has been registered by a settled state update, so this is the moment the
//! cross-chain round trip completes. On success it **mints GOLD** to the player for
//! the whole accumulated amount they withdrew — closing the play-on-L2 / own-on-L1
//! loop.
//!
//! Unlike the dungeon's leaderboard (which lives entirely on the appchain), this
//! world holds no scoring: it is purely the settlement-side mint. State lives in
//! Dojo models/events (indexed by Torii) so the frontend can react to each bank.

use starknet::ContractAddress;

/// Minimal view of the piltover core's messaging interface (the only method the
/// bank needs). The full interface lives in the piltover crate.
#[starknet::interface]
pub trait IPiltoverMessaging<T> {
    fn consume_message_from_appchain(
        ref self: T, from_address: ContractAddress, payload: Span<felt252>,
    ) -> felt252;
}

/// The slice of GOLD this world calls. The bank must be an authorized GOLD minter
/// (granted at deploy) for the mint to succeed.
#[starknet::interface]
pub trait IGoldTokenMint<T> {
    fn mint(ref self: T, to: ContractAddress, amount: u256);
}

#[starknet::interface]
pub trait IBank<T> {
    /// Consume the settled L2 → L1 message `(player, amount, withdraw_no)` emitted by
    /// the appchain `game` world at `from_address`, then mint the player's GOLD.
    /// Reverts until the message has been settled onto the piltover core by saya.
    fn bank(
        ref self: T,
        from_address: ContractAddress,
        player: felt252,
        amount: felt252,
        withdraw_no: felt252,
    );
}

#[dojo::contract]
pub mod bank {
    use dojo::event::EventStorage;
    use dojo::model::ModelStorage;
    use dojo::world::WorldStorage;
    use starknet::ContractAddress;
    use super::{
        IBank, IGoldTokenMintDispatcher, IGoldTokenMintDispatcherTrait,
        IPiltoverMessagingDispatcher, IPiltoverMessagingDispatcherTrait,
    };

    const SINGLETON: u8 = 0;

    /// Config + global bank counter (one row, keyed by `SINGLETON`).
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct BankConfig {
        #[key]
        pub id: u8,
        pub piltover: ContractAddress,
        pub gold_token: ContractAddress,
        /// Minted GOLD = amount * reward_per_gold, in GOLD base units (so the rate
        /// can carry the token's decimals — e.g. 1e18 = 1 GOLD per gold collected).
        pub reward_per_gold: u256,
        pub total_banks: u64,
    }

    /// Emitted when a withdrawal is banked on L1. Keyed by the unique bank sequence
    /// so Torii keeps one row per bank. `withdraw_no` echoes the L2 nonce so the
    /// client can reconcile each L2 `Withdrawal` against its L1 `Banked`.
    #[derive(Copy, Drop, Serde)]
    #[dojo::event]
    pub struct Banked {
        #[key]
        pub bank_no: u64,
        pub player: felt252,
        pub amount: u64,
        pub withdraw_no: u64,
        pub minted: u256,
    }

    /// Record the piltover core, the GOLD token, and the mint rate.
    fn dojo_init(
        self: @ContractState,
        piltover: ContractAddress,
        gold_token: ContractAddress,
        reward_per_gold: u256,
    ) {
        let mut world = self.world_default();
        world
            .write_model(
                @BankConfig {
                    id: SINGLETON, piltover, gold_token, reward_per_gold, total_banks: 0,
                },
            );
    }

    #[abi(embed_v0)]
    impl BankImpl of IBank<ContractState> {
        fn bank(
            ref self: ContractState,
            from_address: ContractAddress,
            player: felt252,
            amount: felt252,
            withdraw_no: felt252,
        ) {
            let mut world = self.world_default();
            let mut config: BankConfig = world.read_model(SINGLETON);

            // Consume the settled message. The (from_address, payload) pair plus this
            // contract as the implicit `to_address` reconstruct the hash registered by
            // the settled state update; reverts if not yet settled.
            let piltover = IPiltoverMessagingDispatcher { contract_address: config.piltover };
            piltover
                .consume_message_from_appchain(
                    from_address, array![player, amount, withdraw_no].span(),
                );

            let amount_u64: u64 = amount.try_into().unwrap();
            let withdraw_no_u64: u64 = withdraw_no.try_into().unwrap();

            // Mint GOLD to the player (this world must be an authorized GOLD minter).
            let minted: u256 = amount_u64.into() * config.reward_per_gold;
            let to: ContractAddress = player.try_into().unwrap();
            IGoldTokenMintDispatcher { contract_address: config.gold_token }.mint(to, minted);

            config.total_banks += 1;
            world.write_model(@config);

            world
                .emit_event(
                    @Banked {
                        bank_no: config.total_banks,
                        player,
                        amount: amount_u64,
                        withdraw_no: withdraw_no_u64,
                        minted,
                    },
                );
        }
    }

    #[generate_trait]
    impl InternalImpl of InternalTrait {
        /// The bank world uses the `"bank"` namespace.
        fn world_default(self: @ContractState) -> WorldStorage {
            self.world(@"bank")
        }
    }
}
