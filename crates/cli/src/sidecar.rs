use std::net::SocketAddr;

use anyhow::{anyhow, Result};
#[cfg(feature = "vrf")]
pub use cartridge::vrf::{
    derive_vrf_accounts, VrfBootstrapResult, VrfDerivedAccounts, VrfService, VrfServiceConfig,
    VrfServiceProcess, VRF_SERVER_PORT,
};
use katana_chain_spec::ChainSpec;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::{DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS};
pub use katana_paymaster::{
    format_felt, wait_for_paymaster_ready, PaymasterService, PaymasterServiceConfig,
    PaymasterServiceConfigBuilder, PaymasterSidecarProcess,
};
use katana_primitives::{ContractAddress, Felt};
use url::Url;

use crate::options::PaymasterOptions;
#[cfg(feature = "vrf")]
use crate::options::VrfOptions;

/// Default API key for the paymaster sidecar.
pub const DEFAULT_PAYMASTER_API_KEY: &str = "paymaster_katana";

pub async fn bootstrap_paymaster(
    options: &PaymasterOptions,
    paymaster_url: Url,
    rpc_url: SocketAddr,
    chain: &ChainSpec,
) -> Result<PaymasterService> {
    let (relayer_addr, relayer_pk) = prefunded_account(chain, 0)?;
    let (gas_tank_addr, gas_tank_pk) = prefunded_account(chain, 1)?;
    let (estimate_account_addr, estimate_account_pk) = prefunded_account(chain, 2)?;

    let port = paymaster_url.port().unwrap();

    let mut builder = PaymasterServiceConfigBuilder::new()
        .rpc(rpc_url)
        .port(port)
        .api_key(DEFAULT_PAYMASTER_API_KEY)
        .relayer(relayer_addr, relayer_pk)
        .gas_tank(gas_tank_addr, gas_tank_pk)
        .estimate_account(estimate_account_addr, estimate_account_pk)
        .tokens(DEFAULT_ETH_FEE_TOKEN_ADDRESS, DEFAULT_STRK_FEE_TOKEN_ADDRESS);

    if let Some(bin) = &options.bin {
        builder = builder.program_path(bin.clone());
    }

    let mut paymaster = PaymasterService::new(builder.build().await?);
    paymaster.bootstrap().await?;

    Ok(paymaster)
}

pub async fn bootstrap_vrf(
    options: &VrfOptions,
    rpc_addr: SocketAddr,
    chain: &ChainSpec,
) -> Result<VrfService> {
    let rpc_url = local_rpc_url(&rpc_addr);
    let (account_address, pk) = prefunded_account(chain, 0)?;

    let result = cartridge::vrf::bootstrap_vrf(rpc_url, account_address, pk).await?;

    let mut vrf_service = VrfService::new(VrfServiceConfig {
        secret_key: result.secret_key,
        vrf_account_address: result.vrf_account_address,
        vrf_private_key: result.vrf_account_private_key,
    });

    if let Some(path) = options.bin.clone() {
        vrf_service = vrf_service.path(path);
    }

    Ok(vrf_service)
}

pub fn prefunded_account(chain_spec: &ChainSpec, index: u16) -> Result<(ContractAddress, Felt)> {
    let (address, allocation) = chain_spec
        .genesis()
        .accounts()
        .nth(index as usize)
        .ok_or_else(|| anyhow!("prefunded account index {} out of range", index))?;

    let private_key = match allocation {
        GenesisAccountAlloc::DevAccount(account) => account.private_key,
        _ => return Err(anyhow!("prefunded account {} has no private key", address)),
    };

    Ok((*address, private_key))
}

pub fn local_rpc_url(addr: &SocketAddr) -> Url {
    let host = match addr.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => {
            std::net::IpAddr::V4([127, 0, 0, 1].into())
        }
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => {
            std::net::IpAddr::V4([127, 0, 0, 1].into())
        }
        ip => ip,
    };

    Url::parse(&format!("http://{}:{}", host, addr.port())).expect("valid rpc url")
}
