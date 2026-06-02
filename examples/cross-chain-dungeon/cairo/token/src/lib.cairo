//! Settlement-layer ("L1", Starknet Sepolia) plain Starknet contracts for the
//! dungeon economy. None of these are Dojo worlds.
//!
//! Two ERC20s with distinct roles:
//! - `game_token` — GAME, the entry credit ("Dungeon Credit"). Bought with USDC and
//!   spent to enter the dungeon. Minting is restricted to authorized minters (the
//!   sale), plus an open `dev_mint` faucet so the demo is playable without real USDC.
//! - `gold_token` — GOLD ("Dungeon Gold"), the dungeon winnings. Collected on the
//!   appchain (L2), accumulated, and **minted here on L1 when a player banks** — the
//!   bank world is its only minter. No faucet: GOLD is earned, never granted.
//!
//! Plus two plain contracts:
//! - `token_sale` — buy GAME with **real Circle USDC** at a fixed rate. This is the
//!   "depend on an external settlement-layer contract" showcase: it calls USDC's
//!   `transfer_from`, then mints GAME to the buyer.
//! - `entry` — charge a GAME entry fee, then send the L1→L2 message that starts a
//!   dungeon run on the appchain (`mint_run`).

use starknet::ContractAddress;

/// GAME (entry credit) minting surface (beyond the standard ERC20 mixin).
#[starknet::interface]
pub trait IGameToken<T> {
    /// Mint to `to`. Caller must be an authorized minter (the sale).
    fn mint(ref self: T, to: ContractAddress, amount: u256);
    /// Owner-only: grant/revoke minting rights.
    fn set_minter(ref self: T, minter: ContractAddress, allowed: bool);
    /// Open dev faucet: mint `amount` to the caller. For local development only —
    /// lets you play without acquiring real USDC. Not for production.
    fn dev_mint(ref self: T, amount: u256);
    fn is_minter(self: @T, account: ContractAddress) -> bool;
}

/// GOLD (dungeon winnings) minting surface. Like GAME but with no faucet — GOLD is
/// only minted by the bank world when a player banks their accumulated haul.
#[starknet::interface]
pub trait IGoldToken<T> {
    /// Mint to `to`. Caller must be an authorized minter (the bank world).
    fn mint(ref self: T, to: ContractAddress, amount: u256);
    /// Owner-only: grant/revoke minting rights.
    fn set_minter(ref self: T, minter: ContractAddress, allowed: bool);
    fn is_minter(self: @T, account: ContractAddress) -> bool;
}

/// The slice of the piltover core's messaging interface `entry` needs (L1 → L2).
#[starknet::interface]
pub trait IPiltoverSend<T> {
    fn send_message_to_appchain(
        ref self: T, to_address: ContractAddress, selector: felt252, payload: Span<felt252>,
    ) -> (felt252, felt252);
}

#[starknet::interface]
pub trait ITokenSale<T> {
    /// Buy GAME_TOKEN with USDC at the fixed rate. Caller must first `approve` the
    /// sale to spend `usdc_amount` of USDC. Mints `usdc_amount * rate` GAME.
    fn buy(ref self: T, usdc_amount: u256);
    fn rate(self: @T) -> u256;
}

#[starknet::interface]
pub trait IEntry<T> {
    /// Charge the GAME_TOKEN entry fee (caller must `approve` first), then send the
    /// L1 → L2 `mint_run` message that starts a run on the appchain.
    fn enter(ref self: T);
    fn entry_fee(self: @T) -> u256;
}

// ---------------------------------------------------------------------------

#[starknet::contract]
pub mod game_token {
    use openzeppelin::access::ownable::OwnableComponent;
    use openzeppelin::token::erc20::{ERC20Component, ERC20HooksEmptyImpl};
    use starknet::storage::{Map, StorageMapReadAccess, StorageMapWriteAccess};
    use starknet::{ContractAddress, get_caller_address};
    use super::IGameToken;

    component!(path: ERC20Component, storage: erc20, event: ERC20Event);
    component!(path: OwnableComponent, storage: ownable, event: OwnableEvent);

    #[abi(embed_v0)]
    impl ERC20MixinImpl = ERC20Component::ERC20MixinImpl<ContractState>;
    #[abi(embed_v0)]
    impl OwnableMixinImpl = OwnableComponent::OwnableMixinImpl<ContractState>;
    impl ERC20InternalImpl = ERC20Component::InternalImpl<ContractState>;
    impl OwnableInternalImpl = OwnableComponent::InternalImpl<ContractState>;

    #[storage]
    struct Storage {
        #[substorage(v0)]
        erc20: ERC20Component::Storage,
        #[substorage(v0)]
        ownable: OwnableComponent::Storage,
        minters: Map<ContractAddress, bool>,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        #[flat]
        ERC20Event: ERC20Component::Event,
        #[flat]
        OwnableEvent: OwnableComponent::Event,
    }

    #[constructor]
    fn constructor(ref self: ContractState, owner: ContractAddress) {
        self.ownable.initializer(owner);
        self.erc20.initializer("Dungeon Credit", "GAME");
    }

    #[abi(embed_v0)]
    impl GameTokenImpl of IGameToken<ContractState> {
        fn mint(ref self: ContractState, to: ContractAddress, amount: u256) {
            assert(self.minters.read(get_caller_address()), 'Not an authorized minter');
            self.erc20.mint(to, amount);
        }

        fn set_minter(ref self: ContractState, minter: ContractAddress, allowed: bool) {
            self.ownable.assert_only_owner();
            self.minters.write(minter, allowed);
        }

        fn dev_mint(ref self: ContractState, amount: u256) {
            // Open faucet — local dev convenience only.
            self.erc20.mint(get_caller_address(), amount);
        }

        fn is_minter(self: @ContractState, account: ContractAddress) -> bool {
            self.minters.read(account)
        }
    }
}

// ---------------------------------------------------------------------------

#[starknet::contract]
pub mod gold_token {
    use openzeppelin::access::ownable::OwnableComponent;
    use openzeppelin::token::erc20::{ERC20Component, ERC20HooksEmptyImpl};
    use starknet::storage::{Map, StorageMapReadAccess, StorageMapWriteAccess};
    use starknet::{ContractAddress, get_caller_address};
    use super::IGoldToken;

    component!(path: ERC20Component, storage: erc20, event: ERC20Event);
    component!(path: OwnableComponent, storage: ownable, event: OwnableEvent);

    #[abi(embed_v0)]
    impl ERC20MixinImpl = ERC20Component::ERC20MixinImpl<ContractState>;
    #[abi(embed_v0)]
    impl OwnableMixinImpl = OwnableComponent::OwnableMixinImpl<ContractState>;
    impl ERC20InternalImpl = ERC20Component::InternalImpl<ContractState>;
    impl OwnableInternalImpl = OwnableComponent::InternalImpl<ContractState>;

    #[storage]
    struct Storage {
        #[substorage(v0)]
        erc20: ERC20Component::Storage,
        #[substorage(v0)]
        ownable: OwnableComponent::Storage,
        minters: Map<ContractAddress, bool>,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        #[flat]
        ERC20Event: ERC20Component::Event,
        #[flat]
        OwnableEvent: OwnableComponent::Event,
    }

    #[constructor]
    fn constructor(ref self: ContractState, owner: ContractAddress) {
        self.ownable.initializer(owner);
        self.erc20.initializer("Dungeon Gold", "GOLD");
    }

    #[abi(embed_v0)]
    impl GoldTokenImpl of IGoldToken<ContractState> {
        fn mint(ref self: ContractState, to: ContractAddress, amount: u256) {
            assert(self.minters.read(get_caller_address()), 'Not an authorized minter');
            self.erc20.mint(to, amount);
        }

        fn set_minter(ref self: ContractState, minter: ContractAddress, allowed: bool) {
            self.ownable.assert_only_owner();
            self.minters.write(minter, allowed);
        }

        fn is_minter(self: @ContractState, account: ContractAddress) -> bool {
            self.minters.read(account)
        }
    }
}

// ---------------------------------------------------------------------------

#[starknet::contract]
pub mod token_sale {
    use openzeppelin::token::erc20::interface::{IERC20Dispatcher, IERC20DispatcherTrait};
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::{ContractAddress, get_caller_address};
    use super::{IGameTokenDispatcher, IGameTokenDispatcherTrait, ITokenSale};

    #[storage]
    struct Storage {
        usdc: ContractAddress,
        game_token: ContractAddress,
        treasury: ContractAddress,
        rate: u256,
    }

    #[constructor]
    fn constructor(
        ref self: ContractState,
        usdc: ContractAddress,
        game_token: ContractAddress,
        treasury: ContractAddress,
        rate: u256,
    ) {
        self.usdc.write(usdc);
        self.game_token.write(game_token);
        self.treasury.write(treasury);
        self.rate.write(rate);
    }

    #[abi(embed_v0)]
    impl TokenSaleImpl of ITokenSale<ContractState> {
        fn buy(ref self: ContractState, usdc_amount: u256) {
            let buyer = get_caller_address();
            // Pull USDC from the buyer into the treasury (external contract call).
            IERC20Dispatcher { contract_address: self.usdc.read() }
                .transfer_from(buyer, self.treasury.read(), usdc_amount);
            // Mint the corresponding GAME at the fixed rate (sale is a minter).
            IGameTokenDispatcher { contract_address: self.game_token.read() }
                .mint(buyer, usdc_amount * self.rate.read());
        }

        fn rate(self: @ContractState) -> u256 {
            self.rate.read()
        }
    }
}

// ---------------------------------------------------------------------------

#[starknet::contract]
pub mod entry {
    use openzeppelin::token::erc20::interface::{IERC20Dispatcher, IERC20DispatcherTrait};
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::{
        ContractAddress, get_block_timestamp, get_caller_address,
    };
    use core::poseidon::poseidon_hash_span;
    use super::{IEntry, IPiltoverSendDispatcher, IPiltoverSendDispatcherTrait};

    /// Selector of the appchain `game` world's `mint_run` l1_handler.
    const MINT_RUN: felt252 = selector!("mint_run");

    #[storage]
    struct Storage {
        game_token: ContractAddress,
        entry_fee: u256,
        sink: ContractAddress,
        piltover: ContractAddress,
        appchain_game: ContractAddress,
    }

    #[constructor]
    fn constructor(
        ref self: ContractState,
        game_token: ContractAddress,
        entry_fee: u256,
        sink: ContractAddress,
        piltover: ContractAddress,
        appchain_game: ContractAddress,
    ) {
        self.game_token.write(game_token);
        self.entry_fee.write(entry_fee);
        self.sink.write(sink);
        self.piltover.write(piltover);
        self.appchain_game.write(appchain_game);
    }

    #[abi(embed_v0)]
    impl EntryImpl of IEntry<ContractState> {
        fn enter(ref self: ContractState) {
            let player = get_caller_address();

            // Charge the entry fee in GAME_TOKEN (caller must approve first).
            IERC20Dispatcher { contract_address: self.game_token.read() }
                .transfer_from(player, self.sink.read(), self.entry_fee.read());

            // Per-run seed: deterministic from the player + L1 block time.
            let player_felt: felt252 = player.into();
            let seed = poseidon_hash_span(
                array![player_felt, get_block_timestamp().into()].span(),
            );

            // Send L1 → L2: start the run on the appchain. mint_run's `from_address`
            // on L2 will be this Entry contract; payload is [player, seed].
            IPiltoverSendDispatcher { contract_address: self.piltover.read() }
                .send_message_to_appchain(
                    self.appchain_game.read(), MINT_RUN, array![player_felt, seed].span(),
                );
        }

        fn entry_fee(self: @ContractState) -> u256 {
            self.entry_fee.read()
        }
    }
}
