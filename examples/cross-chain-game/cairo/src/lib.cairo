//! Contracts for the cross-chain game demo.
//!
//! - `game`           — appchain ("L2"). Purchase (mint_game l1_handler), play
//!                       (roll + finish), and publish score to L1 (send_message_to_l1).
//! - `score_registry` — settlement ("L1"). Consumes the published score after saya
//!                       settles the appchain block.

mod game;
mod score_registry;
