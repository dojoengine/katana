//! Settlement ("L1") arcade coin dispenser.
//!
//! `play_all` sends one L1 -> L2 message to *every* registered machine in a
//! single transaction, by calling the piltover core's `send_message_to_appchain`
//! once per machine. Each call bumps the core's *global, monotonic* L1->L2
//! message nonce, so the messages to machines #1, #2, … carry nonces that are
//! non-contiguous for those target contracts (each machine's own L2 account
//! nonce is still 0). Relaying all of them is exactly what katana PR #623 makes
//! work; before it, only the first machine's message would ever be mined.
//!
//! Sending from *this contract* (rather than from the client calling piltover
//! directly) means each machine's `insert_coin` sees the arcade as its message
//! `from_address` — provenance the machine can trust.

use starknet::ContractAddress;

/// The slice of the piltover core's messaging interface the arcade needs. The
/// full interface lives in the piltover crate
/// (`crates/contracts/contracts/piltover/src/messaging/interface.cairo`).
#[starknet::interface]
pub trait IMessaging<T> {
    fn send_message_to_appchain(
        ref self: T, to_address: ContractAddress, selector: felt252, payload: Span<felt252>,
    ) -> (felt252, felt252);
}

#[starknet::interface]
pub trait IArcade<T> {
    /// Drop one coin into every registered machine (one L1 tx -> N messages to
    /// N distinct target contracts). `player` is the appchain address to credit.
    fn play_all(ref self: T, player: felt252);
    /// Drop one coin into a single machine.
    fn play(ref self: T, machine: ContractAddress, player: felt252);
    /// The registered machine addresses (appchain contracts), in order.
    fn machines(self: @T) -> Array<ContractAddress>;
    /// Total coins dispensed across all machines.
    fn total_coins(self: @T) -> u64;
}

#[starknet::contract]
pub mod arcade {
    use starknet::storage::{
        StoragePointerReadAccess, StoragePointerWriteAccess, Vec, VecTrait, MutableVecTrait,
    };
    use starknet::ContractAddress;
    use super::{IArcade, IMessagingDispatcher, IMessagingDispatcherTrait};

    /// Selector of the appchain `machine` contract's `insert_coin` l1_handler.
    const INSERT_COIN: felt252 = selector!("insert_coin");

    #[storage]
    struct Storage {
        /// The piltover core (L1 mailbox) messages are sent through.
        piltover: ContractAddress,
        /// Registered machine addresses on the appchain.
        machines: Vec<ContractAddress>,
        /// Monotonic coin id, threaded into each message's payload.
        coin_id: u64,
    }

    #[constructor]
    fn constructor(
        ref self: ContractState, piltover: ContractAddress, machines: Array<ContractAddress>,
    ) {
        self.piltover.write(piltover);
        for m in machines {
            self.machines.push(m);
        }
    }

    #[abi(embed_v0)]
    impl ArcadeImpl of IArcade<ContractState> {
        fn play_all(ref self: ContractState, player: felt252) {
            let piltover = IMessagingDispatcher { contract_address: self.piltover.read() };
            let n = self.machines.len();
            for i in 0..n {
                let machine = self.machines.at(i).read();
                self.send_coin(piltover, machine, player);
            }
        }

        fn play(ref self: ContractState, machine: ContractAddress, player: felt252) {
            let piltover = IMessagingDispatcher { contract_address: self.piltover.read() };
            self.send_coin(piltover, machine, player);
        }

        fn machines(self: @ContractState) -> Array<ContractAddress> {
            let mut out = array![];
            let n = self.machines.len();
            for i in 0..n {
                out.append(self.machines.at(i).read());
            }
            out
        }

        fn total_coins(self: @ContractState) -> u64 {
            self.coin_id.read()
        }
    }

    #[generate_trait]
    impl InternalImpl of InternalTrait {
        /// Send one coin to `machine`. Each call increments the core's global
        /// L1->L2 message nonce (independent of which machine it targets).
        fn send_coin(
            ref self: ContractState,
            piltover: IMessagingDispatcher,
            machine: ContractAddress,
            player: felt252,
        ) {
            let coin_id = self.coin_id.read() + 1;
            self.coin_id.write(coin_id);
            piltover
                .send_message_to_appchain(
                    machine, INSERT_COIN, array![player, coin_id.into()].span(),
                );
        }
    }
}
