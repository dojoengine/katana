//! VRF (Verifiable Random Function) support for Cartridge.
//!
//! This module provides:
//! - VRF client for communicating with the VRF server
//! - Bootstrap logic for deploying VRF contracts
//! - Sidecar process management

mod client;

pub mod bootstrap;
pub mod sidecar;

pub use bootstrap::{
    bootstrap_vrf, derive_vrf_accounts, vrf_secret_key_from_account_key, VrfBootstrap,
    VrfBootstrapConfig, VrfBootstrapResult, VrfDerivedAccounts, BOOTSTRAP_TIMEOUT,
    VRF_ACCOUNT_SALT, VRF_CONSUMER_SALT,
};
pub use client::*;
pub use sidecar::{
    resolve_executable, wait_for_http_ok, Error as VrfSidecarError, Result as VrfSidecarResult,
    VrfService, VrfServiceConfig, VrfServiceProcess, SIDECAR_TIMEOUT, VRF_SERVER_PORT,
};
