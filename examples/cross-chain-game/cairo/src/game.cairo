//! Appchain ("L2") game contract — the whole game loop lives here.
//!
//! 1. **Purchase** (L1 → L2): the settlement layer relays a message into the
//!    `mint_game` `#[l1_handler]`, which adds a game to the playable pool.
//! 2. **Play** (L2): `play_game` consumes one available game, rolls a score on
//!    chain, and finishes the game.
//! 3. **Publish** (L2 → L1): `play_game` immediately emits the score to the
//!    settlement `score_registry` via `send_message_to_l1` — no separate step.

use starknet::ContractAddress;

#[starknet::interface]
pub trait IGame<T> {
    /// Play one available game: rolls a score (1..=100), finishes the game, and
    /// publishes the score to L1 in the same transaction. Panics if no games are
    /// available to play.
    fn play_game(ref self: T) -> felt252;

    /// Total games purchased (minted from L1).
    fn total_minted(self: @T) -> u64;
    /// Games minted but not yet played — the playable pool.
    fn available(self: @T) -> u64;
    /// Total games played.
    fn total_played(self: @T) -> u64;
    /// Score of the most recently played game.
    fn last_score(self: @T) -> felt252;
    /// The settlement `score_registry` this contract publishes scores to.
    fn registry(self: @T) -> ContractAddress;
}

#[starknet::contract]
mod game {
    use core::poseidon::poseidon_hash_span;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::syscalls::send_message_to_l1_syscall;
    use starknet::{ContractAddress, SyscallResultTrait};

    #[storage]
    struct Storage {
        registry: ContractAddress,
        total_minted: u64,
        available: u64,
        total_played: u64,
        last_score: felt252,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        GameMinted: GameMinted,
        GamePlayed: GamePlayed,
    }

    /// Emitted when a purchase is relayed from L1 and a game enters the pool.
    #[derive(Drop, starknet::Event)]
    struct GameMinted {
        #[key]
        buyer: felt252,
        game_id: felt252,
        available: u64,
    }

    /// Emitted when a game is played. `score` is also published to L1.
    #[derive(Drop, starknet::Event)]
    struct GamePlayed {
        #[key]
        player: felt252,
        game_no: u64,
        score: felt252,
    }

    #[constructor]
    fn constructor(ref self: ContractState, registry: ContractAddress) {
        self.registry.write(registry);
    }

    /// Phase 1 — purchase relayed from L1. The messaging service prepends the
    /// settlement-side buyer as `from_address`; `game_id` comes from the payload.
    #[l1_handler]
    fn mint_game(ref self: ContractState, from_address: felt252, game_id: felt252) {
        self.total_minted.write(self.total_minted.read() + 1);
        let available = self.available.read() + 1;
        self.available.write(available);
        self.emit(GameMinted { buyer: from_address, game_id, available });
    }

    #[abi(embed_v0)]
    impl GameImpl of super::IGame<ContractState> {
        /// Phases 2 + 3 — play a game and publish its score to L1.
        fn play_game(ref self: ContractState) -> felt252 {
            let available = self.available.read();
            assert(available > 0, 'No games available to play');
            self.available.write(available - 1);

            let game_no = self.total_played.read() + 1;
            self.total_played.write(game_no);

            let score = roll(game_no);
            self.last_score.write(score);

            // Phase 3: publish the score to L1 automatically. `to_address` is the
            // settlement `score_registry`; payload `[player, score]` must match
            // what `score_registry::claim_score` passes to the piltover core.
            let player: felt252 = starknet::get_caller_address().into();
            send_message_to_l1_syscall(
                self.registry.read().into(), array![player, score].span(),
            )
                .unwrap_syscall();

            self.emit(GamePlayed { player, game_no, score });
            score
        }

        fn total_minted(self: @ContractState) -> u64 {
            self.total_minted.read()
        }
        fn available(self: @ContractState) -> u64 {
            self.available.read()
        }
        fn total_played(self: @ContractState) -> u64 {
            self.total_played.read()
        }
        fn last_score(self: @ContractState) -> felt252 {
            self.last_score.read()
        }
        fn registry(self: @ContractState) -> ContractAddress {
            self.registry.read()
        }
    }

    /// Rolls a pseudo-random score in `1..=100`. Not cryptographically secure —
    /// fine for a demo. Each play lands in its own block (provable rollup), so the
    /// block timestamp combined with the play counter varies the roll.
    fn roll(game_no: u64) -> felt252 {
        let ts: felt252 = starknet::get_block_timestamp().into();
        let seed = poseidon_hash_span(array![ts, game_no.into()].span());
        let seed_u256: u256 = seed.into();
        let score: u256 = (seed_u256 % 100) + 1;
        score.try_into().unwrap()
    }
}
