//! Settlement ("L1") contract for the L2 -> L1 direction.
//!
//! `claim_score` consumes a message that the appchain `achievements` contract
//! emitted and saya settled onto the piltover core. Consumption only succeeds
//! once the message has been registered by a settled state update, so this is
//! the moment the cross-chain round trip completes. The view functions let the
//! frontend react to the settled score.

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
    /// appchain `achievements` contract at `from_address`. Reverts until the
    /// message has been settled onto the piltover core by saya.
    fn claim_score(ref self: T, from_address: ContractAddress, player: felt252, score: felt252);
    fn last_score(self: @T) -> felt252;
    fn last_player(self: @T) -> felt252;
    fn total_synced(self: @T) -> u64;
    fn score_of(self: @T, player: felt252) -> felt252;
}

#[starknet::contract]
mod score_registry {
    use starknet::storage::{
        Map, StorageMapReadAccess, StorageMapWriteAccess, StoragePointerReadAccess,
        StoragePointerWriteAccess,
    };
    use starknet::ContractAddress;
    use super::{IPiltoverMessagingDispatcher, IPiltoverMessagingDispatcherTrait};

    #[storage]
    struct Storage {
        piltover: ContractAddress,
        last_score: felt252,
        last_player: felt252,
        total: u64,
        scores: Map<felt252, felt252>,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        ScoreClaimed: ScoreClaimed,
    }

    #[derive(Drop, starknet::Event)]
    struct ScoreClaimed {
        #[key]
        player: felt252,
        score: felt252,
        new_total: u64,
    }

    #[constructor]
    fn constructor(ref self: ContractState, piltover: ContractAddress) {
        self.piltover.write(piltover);
    }

    #[abi(embed_v0)]
    impl ScoreRegistryImpl of super::IScoreRegistry<ContractState> {
        fn claim_score(
            ref self: ContractState, from_address: ContractAddress, player: felt252, score: felt252,
        ) {
            // Consume the message from the piltover core. The (from_address, payload)
            // pair plus this contract as the implicit `to_address` reconstruct the
            // hash registered by the settled state update; reverts if not yet settled.
            let piltover = IPiltoverMessagingDispatcher {
                contract_address: self.piltover.read(),
            };
            piltover.consume_message_from_appchain(from_address, array![player, score].span());

            self.scores.write(player, score);
            self.last_score.write(score);
            self.last_player.write(player);
            let new_total = self.total.read() + 1;
            self.total.write(new_total);

            self.emit(ScoreClaimed { player, score, new_total });
        }

        fn last_score(self: @ContractState) -> felt252 {
            self.last_score.read()
        }

        fn last_player(self: @ContractState) -> felt252 {
            self.last_player.read()
        }

        fn total_synced(self: @ContractState) -> u64 {
            self.total.read()
        }

        fn score_of(self: @ContractState, player: felt252) -> felt252 {
            self.scores.read(player)
        }
    }
}
