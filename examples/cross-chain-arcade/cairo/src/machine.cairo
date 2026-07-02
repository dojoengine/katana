//! Appchain ("L2") arcade machine — one deployed instance per machine.
//!
//! A machine receives coins from L1 through the `insert_coin` `#[l1_handler]`.
//! The settlement layer relays each `send_message_to_appchain` (sent by the L1
//! `arcade` contract) as an `L1HandlerTx` that runs this handler. Because the
//! `arcade` contract is the message sender on L1, `from_address` here is the
//! arcade — provenance the machine can trust (a real machine would assert it).
//!
//! Several machines are deployed from this same class at distinct addresses, so
//! each is a *distinct target contract*. That is the whole point of the demo:
//! before katana PR #623, only the first target in a fan-out would ever receive
//! its coin (the global L1->L2 message nonce was mis-gated as a per-contract
//! account nonce), and every later machine's message stalled with InvalidNonce.

#[starknet::interface]
pub trait IMachine<T> {
    /// Human-readable name (a short string felt), set at deploy time.
    fn name(self: @T) -> felt252;
    /// Total coins this machine has received from L1.
    fn coins(self: @T) -> u64;
    /// The last player credited on this machine.
    fn last_player(self: @T) -> felt252;
    /// Coins credited to a specific player on this machine.
    fn credits(self: @T, player: felt252) -> u64;
}

#[starknet::contract]
pub mod machine {
    use starknet::storage::{
        Map, StoragePointerReadAccess, StoragePointerWriteAccess, StorageMapReadAccess,
        StorageMapWriteAccess,
    };
    use super::IMachine;

    #[storage]
    struct Storage {
        name: felt252,
        coins: u64,
        last_player: felt252,
        credits: Map<felt252, u64>,
    }

    /// Emitted every time a coin is relayed in from L1.
    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        CoinInserted: CoinInserted,
    }

    #[derive(Drop, starknet::Event)]
    struct CoinInserted {
        #[key]
        player: felt252,
        coin_id: felt252,
        coins: u64,
    }

    #[constructor]
    fn constructor(ref self: ContractState, name: felt252) {
        self.name.write(name);
    }

    /// Receive a coin relayed from L1. `from_address` is the L1 `arcade`
    /// contract that sent the message; the player to credit travels in the
    /// payload (the L1 buyer and the L2 player may be different accounts). A
    /// production machine would `assert(from_address == arcade)` here.
    #[l1_handler]
    fn insert_coin(
        ref self: ContractState, from_address: felt252, player: felt252, coin_id: felt252,
    ) {
        let coins = self.coins.read() + 1;
        self.coins.write(coins);
        self.last_player.write(player);
        self.credits.write(player, self.credits.read(player) + 1);
        self.emit(CoinInserted { player, coin_id, coins });
    }

    #[abi(embed_v0)]
    impl MachineImpl of IMachine<ContractState> {
        fn name(self: @ContractState) -> felt252 {
            self.name.read()
        }
        fn coins(self: @ContractState) -> u64 {
            self.coins.read()
        }
        fn last_player(self: @ContractState) -> felt252 {
            self.last_player.read()
        }
        fn credits(self: @ContractState, player: felt252) -> u64 {
            self.credits.read(player)
        }
    }
}
