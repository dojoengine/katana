#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod client;
pub mod vrf;

pub use client::Client;
pub use vrf::{
    bootstrap_vrf, get_vrf_account, resolve_executable, wait_for_http_ok, InfoResponse,
    RequestContext, SignedOutsideExecution, VrfAccountCredentials, VrfBootstrap,
    VrfBootstrapConfig, VrfBootstrapResult, VrfClient, VrfClientError, VrfOutsideExecution,
    VrfService, VrfServiceConfig, VrfServiceProcess, VRF_ACCOUNT_SALT, VRF_CONSUMER_SALT,
    VRF_HARDCODED_SECRET_KEY, VRF_SERVER_PORT,
};

#[rustfmt::skip]
pub mod controller;
