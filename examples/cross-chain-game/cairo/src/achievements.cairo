//! Appchain ("L2") contract for the L2 -> L1 direction.
//!
//! `sync_score` emits a message to the settlement layer via the
//! `send_message_to_l1` syscall, addressed to the settlement-side
//! `score_registry`. After saya settles the appchain block, the message is
//! registered on the piltover core and the `score_registry` can consume it.

use starknet::ContractAddress;

#[starknet::interface]
pub trait IAchievements<T> {
    /// Emit an L2 -> L1 message carrying `(player, score)` to the settlement
    /// `score_registry`. `player` is the caller, so the settled score is
    /// attributed to whoever synced it.
    fn sync_score(ref self: T, score: felt252);
    /// The settlement-side registry this contract addresses its messages to.
    fn registry(self: @T) -> ContractAddress;
}

#[starknet::contract]
mod achievements {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::syscalls::send_message_to_l1_syscall;
    use starknet::{ContractAddress, SyscallResultTrait};

    #[storage]
    struct Storage {
        registry: ContractAddress,
    }

    #[constructor]
    fn constructor(ref self: ContractState, registry: ContractAddress) {
        self.registry.write(registry);
    }

    #[abi(embed_v0)]
    impl AchievementsImpl of super::IAchievements<ContractState> {
        fn sync_score(ref self: ContractState, score: felt252) {
            let player: felt252 = starknet::get_caller_address().into();
            let to_address: felt252 = self.registry.read().into();
            // Payload [player, score] must match what `score_registry` passes to
            // `consume_message_from_appchain`, or the message hashes won't match.
            send_message_to_l1_syscall(to_address, array![player, score].span())
                .unwrap_syscall();
        }

        fn registry(self: @ContractState) -> ContractAddress {
            self.registry.read()
        }
    }
}
