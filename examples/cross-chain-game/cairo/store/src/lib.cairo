//! Settlement ("L1") store world — the game's storefront.
//!
//! This is where a purchase *should* originate in a real game: the player calls
//! `buy_game` on the game's own L1 contract, which runs the game's rules (charge
//! a token, check supply, rate-limit, …) and only then sends the L1 → L2 message
//! that mints a credit on the appchain. Nesting the `send_message_to_appchain`
//! call inside this function (rather than calling the piltover core directly
//! from the client) makes the payment/validation atomic with the message, and
//! makes the appchain's `mint_game` see *this contract* as the message sender —
//! so L2 can trust that a mint came from the official store.

use starknet::ContractAddress;

/// The slice of the piltover core's messaging interface the store needs to send
/// an L1 → L2 message. The full interface lives in the piltover crate.
#[starknet::interface]
pub trait IPiltoverSend<T> {
    fn send_message_to_appchain(
        ref self: T, to_address: ContractAddress, selector: felt252, payload: Span<felt252>,
    ) -> (felt252, felt252);
}

#[starknet::interface]
pub trait IStore<T> {
    /// Buy one game for `player`: run the store's rules, then send the L1 → L2
    /// message that credits `player` on the appchain `game` world. `player` is the
    /// appchain address to credit — the L1 buyer and the L2 player can differ (the
    /// dev account uses two keys; a Controller is the same address on both).
    fn buy_game(ref self: T, player: felt252, game_id: felt252);
}

#[dojo::contract]
pub mod store {
    use dojo::model::ModelStorage;
    use dojo::world::WorldStorage;
    use starknet::ContractAddress;
    use super::{IPiltoverSendDispatcher, IPiltoverSendDispatcherTrait, IStore};

    /// Selector of the appchain `game` world's `mint_game` l1_handler.
    const MINT_GAME: felt252 = selector!("mint_game");
    const SINGLETON: u8 = 0;

    /// Where to send purchases: the piltover core (L1) and the appchain `game`
    /// system the message targets. Set once at init.
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct StoreConfig {
        #[key]
        pub id: u8,
        pub piltover: ContractAddress,
        pub game: ContractAddress,
    }

    /// Running tally of games sold (stand-in for real store bookkeeping).
    #[derive(Copy, Drop, Serde)]
    #[dojo::model]
    pub struct Sales {
        #[key]
        pub id: u8,
        pub total: u64,
    }

    fn dojo_init(self: @ContractState, piltover: ContractAddress, game: ContractAddress) {
        let mut world = self.world_default();
        world.write_model(@StoreConfig { id: SINGLETON, piltover, game });
        world.write_model(@Sales { id: SINGLETON, total: 0 });
    }

    #[abi(embed_v0)]
    impl StoreImpl of IStore<ContractState> {
        fn buy_game(ref self: ContractState, player: felt252, game_id: felt252) {
            let mut world = self.world_default();

            // --- the game's rules go here ---
            // In a real game this is where you'd charge an ERC-20, check supply
            // or an allowlist, rate-limit the buyer, etc. We just tally the sale.
            let mut sales: Sales = world.read_model(SINGLETON);
            sales.total += 1;
            world.write_model(@sales);

            // Only after the rules pass do we send the L1 -> L2 message. Because
            // the store contract is the caller, `mint_game`'s `from_address` on
            // L2 is this contract — provenance the appchain can trust. The buyer/
            // player to credit travels in the payload (`from_address` is the store,
            // not the player).
            let config: StoreConfig = world.read_model(SINGLETON);
            IPiltoverSendDispatcher { contract_address: config.piltover }
                .send_message_to_appchain(config.game, MINT_GAME, array![player, game_id].span());
        }
    }

    #[generate_trait]
    impl InternalImpl of InternalTrait {
        /// The store world uses the `"store"` namespace.
        fn world_default(self: @ContractState) -> WorldStorage {
            self.world(@"store")
        }
    }
}
