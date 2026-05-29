//! Contracts for the cross-chain game store demo.
//!
//! - `game_minter`  — appchain ("L2"). Receives L1 -> L2 messages (`mint_game`).
//! - `achievements` — appchain ("L2"). Emits L2 -> L1 messages (`sync_score`).
//! - `score_registry` — settlement ("L1"). Consumes the settled L2 -> L1 message.

mod game_minter;
mod achievements;
mod score_registry;
