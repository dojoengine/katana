#![allow(dead_code)]

use std::pin::Pin;
use std::sync::Arc;

use alloy_network::Ethereum;
use alloy_primitives::{Address, U256};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter, FilterBlockOption, FilterSet, Log, Topic};
use alloy_sol_types::{sol, SolEvent};
use alloy_transport::TransportError;
use anyhow::Result;
use futures::Future;
use katana_primitives::chain::ChainId;
use katana_primitives::eth::Address as EthAddress;
use katana_primitives::message::L1ToL2Message;
use katana_primitives::receipt::MessageToL1;
use katana_primitives::transaction::L1HandlerTx;
use katana_primitives::utils::transaction::{
    compute_l1_to_l2_message_hash, compute_l2_to_l1_message_hash,
};
use katana_primitives::{ContractAddress, Felt};
use tracing::{debug, trace};
use url::Url;

use crate::stream::collector::{GatherResult, MessageCollector, OrderedMessage};
use crate::LOG_TARGET;

/// Errors produced by [`EthereumCollector`].
#[derive(Debug, thiserror::Error)]
pub enum EthereumCollectorError {
    #[error("ethereum provider error: {0}")]
    Provider(#[from] TransportError),

    /// An Ethereum log was found whose shape didn't match the expected
    /// `LogMessageToL2` schema.
    #[error("malformed settlement chain message: {0}")]
    MalformedMessage(String),
}

sol! {
    #[sol(rpc, rename_all = "snakecase")]
    #[derive(serde::Serialize, serde::Deserialize)]
    interface StarknetMessagingLocal {
        function addMessageHashesFromL2(uint256[] msgHashes) external payable;
    }

    #[derive(Debug, PartialEq)]
    event LogMessageToL2(
        address indexed from_address,
        uint256 indexed to_address,
        uint256 indexed selector,
        uint256[] payload,
        uint256 nonce,
        uint256 fee
    );
}

/// Ethereum settlement chain message collector.
#[derive(Debug)]
pub struct EthereumCollector {
    provider: Arc<RootProvider<Ethereum>>,
    messaging_contract_address: Address,
}

impl EthereumCollector {
    pub fn new(rpc_url: Url, messaging_contract_address: EthAddress) -> Result<Self> {
        let provider = Arc::new(RootProvider::<Ethereum>::new_http(rpc_url));
        Ok(Self { provider, messaging_contract_address })
    }

    /// Fetches logs in the given block range.
    async fn fetch_logs(
        provider: &RootProvider<Ethereum>,
        contract_address: Address,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<Log>, EthereumCollectorError> {
        trace!(target: LOG_TARGET, from_block, to_block, "Fetching ethereum logs.");

        let filters = Filter {
            block_option: FilterBlockOption::Range {
                from_block: Some(BlockNumberOrTag::Number(from_block)),
                to_block: Some(BlockNumberOrTag::Number(to_block)),
            },
            address: FilterSet::<Address>::from(contract_address),
            topics: [
                Topic::from(LogMessageToL2::SIGNATURE_HASH),
                Default::default(),
                Default::default(),
                Default::default(),
            ],
        };

        let logs = provider
            .get_logs(&filters)
            .await?
            .into_iter()
            .filter(|log| log.block_number.is_some())
            .collect();

        Ok(logs)
    }
}

impl MessageCollector for EthereumCollector {
    type Error = EthereumCollectorError;

    fn latest_block(&self) -> Pin<Box<dyn Future<Output = Result<u64, Self::Error>> + Send + '_>> {
        Box::pin(async { Ok(self.provider.get_block_number().await?) })
    }

    fn gather(
        &self,
        from_block: u64,
        from_tx_index: u64,
        to_block: u64,
        chain_id: ChainId,
    ) -> Pin<Box<dyn Future<Output = Result<GatherResult, Self::Error>> + Send + '_>> {
        Box::pin(async move {
            let mut messages = vec![];

            let logs = Self::fetch_logs(
                &self.provider,
                self.messaging_contract_address,
                from_block,
                to_block,
            )
            .await?;

            for log in &logs {
                let Some(block) = log.block_number else { continue };
                let Some(tx_index) = log.transaction_index else { continue };
                let Some(l1_tx_hash) = log.transaction_hash else { continue };

                // Skip messages already processed on a previous run.
                if block == from_block && tx_index < from_tx_index {
                    continue;
                }

                debug!(target: LOG_TARGET, block, tx_index, "Converting log into L1HandlerTx.");

                if let Ok(tx) = l1_handler_tx_from_log(log.clone(), chain_id) {
                    messages.push(OrderedMessage { block, tx_index, l1_tx_hash: l1_tx_hash.0, tx });
                }
            }

            Ok(GatherResult { to_block, messages })
        })
    }
}

// --- Conversion functions ---

fn l1_handler_tx_from_log(
    log: Log,
    chain_id: ChainId,
) -> Result<L1HandlerTx, EthereumCollectorError> {
    let log = LogMessageToL2::decode_log(log.as_ref()).map_err(|e| {
        EthereumCollectorError::MalformedMessage(format!("decode LogMessageToL2 log: {e}"))
    })?;

    let from_address = log.from_address;
    let contract_address = ContractAddress::from(log.to_address);
    let entry_point_selector = felt_from_u256(log.selector);
    let nonce = felt_from_u256(log.nonce);
    let paid_fee_on_l1: u128 = log.fee.try_into().map_err(|_| {
        EthereumCollectorError::MalformedMessage(format!(
            "L1->L2 fee {} does not fit into u128",
            log.fee
        ))
    })?;
    let payload = log.payload.clone().into_iter().map(felt_from_u256).collect::<Vec<_>>();

    let message = L1ToL2Message {
        from_address,
        to_address: log.to_address.into(),
        entry_point_selector,
        payload: payload.clone(),
        nonce,
    };

    let message_hash = compute_l1_to_l2_message_hash(
        message.from_address,
        message.to_address,
        message.entry_point_selector,
        &message.payload,
        message.nonce,
    );

    let mut calldata = vec![Felt::from_bytes_be_slice(from_address.as_slice())];
    calldata.extend(payload.clone());

    Ok(L1HandlerTx {
        calldata,
        chain_id,
        message_hash,
        paid_fee_on_l1,
        nonce,
        entry_point_selector,
        version: Felt::ZERO,
        contract_address,
    })
}

/// With Ethereum, the messages are following the conventional starknet messaging.
fn parse_messages(messages: &[MessageToL1]) -> Vec<U256> {
    messages
        .iter()
        .map(|msg| {
            let hash = compute_l2_to_l1_message_hash(
                msg.from_address.into(),
                msg.to_address,
                &msg.payload,
            );
            U256::from_be_bytes(hash.into())
        })
        .collect()
}

fn felt_from_u256(v: U256) -> Felt {
    use std::str::FromStr;
    Felt::from_str(format!("{v:#064x}").as_str()).unwrap()
}

#[cfg(test)]
mod tests {

    use alloy_primitives::{address, b256, LogData, U256};
    use katana_primitives::chain::{ChainId, NamedChainId};
    use katana_primitives::felt;
    use katana_primitives::utils::transaction::compute_l1_to_l2_message_hash;
    use starknet::macros::selector;

    use super::*;

    #[test]
    fn l1_handler_tx_from_log_parse_ok() {
        let from_address = address!("be3C44c09bc1a3566F3e1CA12e5AbA0fA4Ca72Be");
        let to_address = felt!("0x39dc79e64f4bb3289240f88e0bae7d21735bef0d1a51b2bf3c4730cb16983e1");
        let selector = felt!("0x2f15cff7b0eed8b9beb162696cf4e3e0e35fa7032af69cd1b7d2ac67a13f40f");
        let payload = vec![Felt::ONE, Felt::TWO];
        let nonce = 783082_u64;
        let fee = 30000_u128;

        let expected_tx_hash =
            felt!("0x6182c63599a9638272f1ce5b5cadabece9c81c2d2b8f88ab7a294472b8fce8b");

        let event = LogMessageToL2::new(
            (
                b256!("db80dd488acf86d17c747445b0eabb5d57c541d3bd7b6b87af987858e5066b2b"),
                from_address,
                U256::from_be_slice(&to_address.to_bytes_be()),
                U256::from_be_slice(&selector.to_bytes_be()),
            ),
            (vec![U256::from(1), U256::from(2)], U256::from(nonce), U256::from(fee)),
        );

        let log = Log {
            inner: alloy_primitives::Log::<LogData> {
                address: address!("de29d060D45901Fb19ED6C6e959EB22d8626708e"),
                data: LogData::from(&event),
            },
            ..Default::default()
        };

        // SN_GOERLI.
        let chain_id = ChainId::Named(NamedChainId::Goerli);

        let message_hash = compute_l1_to_l2_message_hash(
            from_address.clone(),
            ContractAddress(to_address),
            selector,
            &payload,
            nonce.into(),
        );

        let calldata = vec![Felt::from_bytes_be_slice(from_address.as_slice())]
            .into_iter()
            .chain(payload.clone())
            .collect();

        let expected_tx = L1HandlerTx {
            calldata,
            chain_id,
            message_hash,
            paid_fee_on_l1: fee,
            version: Felt::ZERO,
            nonce: Felt::from(nonce),
            contract_address: to_address.into(),
            entry_point_selector: selector,
        };

        let actual_tx = l1_handler_tx_from_log(log, chain_id).expect("bad log format");

        assert_eq!(expected_tx, actual_tx);
        assert_eq!(expected_tx_hash, expected_tx.calculate_hash());
    }

    #[test]
    fn parse_msg_to_l1() {
        let from_address = selector!("from_address");
        let to_address = selector!("to_address");
        let payload = vec![Felt::ONE, Felt::TWO];

        let messages = vec![MessageToL1 { from_address: from_address.into(), to_address, payload }];

        let hashes = parse_messages(&messages);
        assert_eq!(hashes.len(), 1);
        assert_eq!(
            hashes[0],
            U256::from_str_radix(
                "5ba1d2e131360f15e26dd4f6ff10550685611cc25f75e7950b704adb04b36162",
                16
            )
            .unwrap()
        );
    }
}
