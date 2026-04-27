use alloy_primitives::B256;
use num_bigint::BigUint;
use starknet_rust_core::types::{BlockId, BlockTag, Felt, FunctionCall};
use starknet_rust_core::utils::get_selector_from_name;
use starknet_rust_providers::jsonrpc::HttpTransport;
use starknet_rust_providers::{JsonRpcClient, Provider};
use url::Url;

use crate::Error;

/// Minimal Starknet RPC client for AMD TEE registry cache queries.
#[derive(Debug, Clone)]
pub struct StarknetRegistryClient {
    provider: JsonRpcClient<HttpTransport>,
    contract_address: Felt,
}

impl StarknetRegistryClient {
    /// Create a new client from RPC URL and contract address.
    pub fn new(rpc_url: &str, contract_address: Felt) -> Self {
        let url = Url::parse(rpc_url).expect("Invalid Starknet RPC URL");
        let transport = HttpTransport::new(url);
        let provider = JsonRpcClient::new(transport);
        Self {
            provider,
            contract_address,
        }
    }

    /// Create a new client from hex string contract address.
    pub fn from_hex(rpc_url: &str, contract_address: &str) -> Result<Self, Error> {
        let contract_address = Felt::from_hex(contract_address)
            .map_err(|e| Error::Starknet(format!("Invalid contract address: {e}")))?;
        Ok(Self::new(rpc_url, contract_address))
    }

    /// Fetch the trusted certificate prefix length for a single report.
    pub async fn fetch_trusted_prefix_len(
        &self,
        processor_model: u8,
        cert_digests: &[B256],
    ) -> Result<u8, Error> {
        if cert_digests.is_empty() {
            return Err(Error::Starknet("Certificate chain is empty".to_string()));
        }

        let selector = get_selector_from_name("check_trusted_intermediate_certs")
            .map_err(|e| Error::Starknet(format!("Selector error: {e}")))?;

        let calldata = encode_check_trusted_intermediate_certs(processor_model, cert_digests)?;

        let call = FunctionCall {
            contract_address: self.contract_address,
            entry_point_selector: selector,
            calldata,
        };

        let result = self
            .provider
            .call(&call, BlockId::Tag(BlockTag::Latest))
            .await
            .map_err(|e| Error::Starknet(format!("RPC call failed: {e}")))?;

        if result.is_empty() {
            return Err(Error::Starknet("Empty response from Starknet".to_string()));
        }

        let len = felt_to_u64(&result[0])?;
        if len != 1 {
            return Err(Error::Starknet(format!("Unexpected result length: {len}")));
        }
        if result.len() < 2 {
            return Err(Error::Starknet("Missing prefix length".to_string()));
        }
        felt_to_u8(&result[1])
    }
}

fn encode_check_trusted_intermediate_certs(
    processor_model: u8,
    cert_digests: &[B256],
) -> Result<Vec<Felt>, Error> {
    let mut calldata: Vec<Felt> = vec![
        // processor_models: Span<ProcessorType>
        Felt::from(1u64),
        Felt::from(processor_model as u64),
        // report_certs: Span<Span<u256>>
        Felt::from(1u64),
        Felt::from(cert_digests.len() as u64),
    ];

    for digest in cert_digests {
        let (low, high) = b256_to_u256_felts(digest)?;
        calldata.push(low);
        calldata.push(high);
    }

    Ok(calldata)
}

fn b256_to_u256_felts(value: &B256) -> Result<(Felt, Felt), Error> {
    let big = BigUint::from_bytes_be(value.as_slice());
    let mask = (BigUint::from(1u8) << 128) - BigUint::from(1u8);
    let low = &big & &mask;
    let high = big >> 128;
    Ok((biguint_to_felt(&low), biguint_to_felt(&high)))
}

fn biguint_to_felt(value: &BigUint) -> Felt {
    let mut bytes = [0u8; 32];
    let value_bytes = value.to_bytes_be();
    let start = 32 - value_bytes.len();
    bytes[start..].copy_from_slice(&value_bytes);
    Felt::from_bytes_be(&bytes)
}

fn felt_to_u64(value: &Felt) -> Result<u64, Error> {
    let bytes = value.to_bytes_be();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[24..]);
    if bytes[..24].iter().any(|b| *b != 0) {
        return Err(Error::Starknet("Felt does not fit in u64".to_string()));
    }
    Ok(u64::from_be_bytes(buf))
}

fn felt_to_u8(value: &Felt) -> Result<u8, Error> {
    let value = felt_to_u64(value)?;
    if value > u8::MAX as u64 {
        return Err(Error::Starknet("Felt does not fit in u8".to_string()));
    }
    Ok(value as u8)
}
