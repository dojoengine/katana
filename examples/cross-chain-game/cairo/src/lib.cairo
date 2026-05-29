//! Appchain ("L2") contract for the cross-chain game store demo.
//!
//! The settlement layer ("L1") sends a message to this contract via Katana's
//! messaging service. The message invokes the `mint_game` `#[l1_handler]`, which
//! records the purchase. The demo frontend polls the view functions below to react
//! to the new on-chain state.

#[starknet::interface]
pub trait IGameMinter<T> {
    /// Total number of games minted across all buyers.
    fn total_minted(self: @T) -> u64;
    /// Number of games minted for a specific buyer (the settlement-side caller).
    fn minted_by(self: @T, owner: felt252) -> u64;
    /// The most recent buyer to mint a game.
    fn last_buyer(self: @T) -> felt252;
}

#[starknet::contract]
mod game_minter {
    use starknet::storage::{
        Map, StorageMapReadAccess, StorageMapWriteAccess, StoragePointerReadAccess,
        StoragePointerWriteAccess,
    };

    #[storage]
    struct Storage {
        total: u64,
        by_owner: Map<felt252, u64>,
        last_buyer: felt252,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        GameMinted: GameMinted,
    }

    #[derive(Drop, starknet::Event)]
    struct GameMinted {
        #[key]
        buyer: felt252,
        game_id: felt252,
        new_total: u64,
    }

    /// Handles a message relayed from the settlement layer.
    ///
    /// Only `#[l1_handler]` functions can be invoked by Katana's messaging service.
    /// The messaging service always prepends the settlement-side caller as
    /// `from_address`; the remaining arguments come from the message payload.
    #[l1_handler]
    fn mint_game(ref self: ContractState, from_address: felt252, game_id: felt252) {
        let new_total = self.total.read() + 1;
        self.total.write(new_total);

        let owned = self.by_owner.read(from_address) + 1;
        self.by_owner.write(from_address, owned);

        self.last_buyer.write(from_address);

        self.emit(GameMinted { buyer: from_address, game_id, new_total });
    }

    #[abi(embed_v0)]
    impl GameMinterImpl of super::IGameMinter<ContractState> {
        fn total_minted(self: @ContractState) -> u64 {
            self.total.read()
        }

        fn minted_by(self: @ContractState, owner: felt252) -> u64 {
            self.by_owner.read(owner)
        }

        fn last_buyer(self: @ContractState) -> felt252 {
            self.last_buyer.read()
        }
    }
}
