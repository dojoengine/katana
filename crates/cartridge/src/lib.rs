#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod api;
pub mod vrf;

pub use api::CartridgeApiClient;
pub use vrf::server::{
    bootstrap_vrf, get_vrf_account, resolve_executable, wait_for_http_ok, VrfAccountCredentials,
    VrfBootstrap, VrfBootstrapConfig, VrfBootstrapResult, VrfServer, VrfServerConfig,
    VrfServiceProcess, VRF_ACCOUNT_SALT, VRF_CONSUMER_SALT, VRF_HARDCODED_SECRET_KEY,
    VRF_SERVER_PORT,
};

#[rustfmt::skip]
pub mod controller;
