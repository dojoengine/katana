use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use alloy_primitives::B256;
use anyhow::Result;
use futures::{Future, FutureExt, Stream};
use katana_primitives::chain::ChainId;
use katana_primitives::hash::StarkHash;
use katana_primitives::transaction::L1HandlerTx;
use katana_primitives::{hash, Felt};
use starknet::core::types::{BlockId, EmittedEvent, EventFilter};
use starknet::macros::selector;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{AnyProvider, JsonRpcClient, Provider};
use tokio::time::{interval_at, Instant, Interval};
use tracing::{debug, error, trace, warn};
use url::Url;

use super::{Error, MessagingOutcome, MessengerResult, LOG_TARGET};

/// TODO: This may come from the configuration.
pub const MESSAGE_SENT_EVENT_KEY: Felt = selector!("MessageSent");

type GatherFuture = Pin<Box<dyn Future<Output = MessengerResult<(u64, u64, Vec<L1HandlerTx>)>> + Send>>;

#[allow(missing_debug_implementations)]
pub struct StarknetMessaging {
    provider: Arc<AnyProvider>,
    messaging_contract_address: Felt,
    chain_id: ChainId,
    interval: Interval,
    from_block: u64,
    gather_fut: Option<GatherFuture>,
}

impl StarknetMessaging {
    pub async fn new(
        rpc_url: &str,
        contract_address: &str,
        chain_id: ChainId,
        from_block: u64,
        interval_secs: u64,
    ) -> Result<Self> {
        let provider = Arc::new(AnyProvider::JsonRpcHttp(JsonRpcClient::new(
            HttpTransport::new(Url::parse(rpc_url)?),
        )));

        let messaging_contract_address = Felt::from_hex(contract_address)?;

        let duration = Duration::from_secs(interval_secs);
        let mut interval = interval_at(Instant::now() + duration, duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        Ok(Self {
            provider,
            messaging_contract_address,
            chain_id,
            interval,
            from_block,
            gather_fut: None,
        })
    }

    async fn fetch_events(
        provider: &AnyProvider,
        contract_address: Felt,
        from_block: BlockId,
        to_block: BlockId,
    ) -> Result<Vec<EmittedEvent>> {
        trace!(target: LOG_TARGET, from_block = ?from_block, to_block = ?to_block, "Fetching logs.");

        let mut events = vec![];

        let filter = EventFilter {
            from_block: Some(from_block),
            to_block: Some(to_block),
            address: Some(contract_address),
            keys: Some(vec![vec![MESSAGE_SENT_EVENT_KEY]]),
        };

        let chunk_size = 200;
        let mut continuation_token: Option<String> = None;

        loop {
            let event_page =
                provider.get_events(filter.clone(), continuation_token, chunk_size).await?;

            event_page.events.into_iter().for_each(|event| {
                if event.block_number.is_some() {
                    events.push(event);
                }
            });

            continuation_token = event_page.continuation_token;

            if continuation_token.is_none() {
                break;
            }
        }

        Ok(events)
    }

    /// Returns (to_block, last_event_index, transactions).
    async fn gather_messages(
        provider: Arc<AnyProvider>,
        contract_address: Felt,
        chain_id: ChainId,
        from_block: u64,
    ) -> MessengerResult<(u64, u64, Vec<L1HandlerTx>)> {
        let max_blocks = 200;

        let chain_latest_block: u64 = match provider.block_number().await {
            Ok(n) => n,
            Err(_) => {
                warn!(
                    target: LOG_TARGET,
                    "Couldn't fetch settlement chain last block number. \nSkipped, retry at the \
                     next tick."
                );
                return Err(Error::GatherError);
            }
        };

        if from_block > chain_latest_block {
            return Ok((chain_latest_block, 0, vec![]));
        }

        // +1 as the from_block counts as 1 block fetched.
        let to_block = if from_block + max_blocks + 1 < chain_latest_block {
            from_block + max_blocks
        } else {
            chain_latest_block
        };

        let mut l1_handler_txs: Vec<L1HandlerTx> = vec![];
        let mut last_event_index: u64 = 0;

        let events = Self::fetch_events(
            &provider,
            contract_address,
            BlockId::Number(from_block),
            BlockId::Number(to_block),
        )
        .await
        .map_err(|_| Error::GatherError)?;

        for (idx, e) in events.iter().enumerate() {
            debug!(
                target: LOG_TARGET,
                event = ?e,
                "Converting event into L1HandlerTx."
            );

            if let Ok(tx) = l1_handler_tx_from_event(e, chain_id) {
                l1_handler_txs.push(tx);
                last_event_index = idx as u64;
            }
        }

        Ok((to_block, last_event_index, l1_handler_txs))
    }
}

impl Stream for StarknetMessaging {
    type Item = MessagingOutcome;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if this.interval.poll_tick(cx).is_ready() && this.gather_fut.is_none() {
            this.gather_fut = Some(Box::pin(Self::gather_messages(
                this.provider.clone(),
                this.messaging_contract_address,
                this.chain_id,
                this.from_block,
            )));
        }

        if let Some(mut fut) = this.gather_fut.take() {
            match fut.poll_unpin(cx) {
                Poll::Ready(Ok((last_block, tx_index, transactions))) => {
                    this.from_block = last_block + 1;
                    return Poll::Ready(Some(MessagingOutcome {
                        settlement_block: last_block,
                        tx_index,
                        transactions,
                    }));
                }
                Poll::Ready(Err(e)) => {
                    error!(
                        target: LOG_TARGET,
                        block = %this.from_block,
                        error = %e,
                        "Gathering messages for block."
                    );
                    return Poll::Pending;
                }
                Poll::Pending => this.gather_fut = Some(fut),
            }
        }

        Poll::Pending
    }
}

fn l1_handler_tx_from_event(event: &EmittedEvent, chain_id: ChainId) -> Result<L1HandlerTx> {
    if event.keys[0] != MESSAGE_SENT_EVENT_KEY {
        error!(
            target: LOG_TARGET,
            event_key = ?event.keys[0],
            "Event can't be converted into L1HandlerTx."
        );
        return Err(Error::GatherError.into());
    }

    if event.keys.len() != 4 || event.data.len() < 2 {
        error!(target: LOG_TARGET, "Event MessageSentToAppchain is not well formatted.");
    }

    let from_address = event.keys[2];
    let to_address = event.keys[3];
    let entry_point_selector = event.data[0];
    let nonce = event.data[1];

    let mut calldata = vec![from_address];
    calldata.extend(&event.data[3..]);

    let message_hash = compute_starknet_to_appchain_message_hash(
        from_address,
        to_address,
        nonce,
        entry_point_selector,
        &calldata,
    );

    let message_hash = B256::from_slice(message_hash.to_bytes_be().as_slice());

    Ok(L1HandlerTx {
        nonce,
        calldata,
        chain_id,
        message_hash,
        paid_fee_on_l1: 30000_u128,
        entry_point_selector,
        version: Felt::ZERO,
        contract_address: to_address.into(),
    })
}

fn compute_starknet_to_appchain_message_hash(
    from_address: Felt,
    to_address: Felt,
    nonce: Felt,
    entry_point_selector: Felt,
    payload: &[Felt],
) -> Felt {
    let mut buf: Vec<Felt> =
        vec![from_address, to_address, nonce, entry_point_selector, Felt::from(payload.len())];

    for p in payload {
        buf.push(*p);
    }

    hash::Poseidon::hash_array(&buf)
}

#[cfg(test)]
mod tests {
    use katana_primitives::utils::transaction::compute_l1_handler_tx_hash;
    use starknet::macros::felt;

    use super::*;

    #[test]
    fn l1_handler_tx_from_event_parse_ok() {
        let from_address = selector!("from_address");
        let to_address = selector!("to_address");
        let selector = selector!("selector");
        let chain_id = ChainId::parse("KATANA").unwrap();
        let nonce = Felt::ONE;
        let calldata = vec![from_address, Felt::THREE];

        let transaction_hash: Felt = compute_l1_handler_tx_hash(
            Felt::ZERO,
            to_address.into(),
            selector,
            &calldata,
            chain_id.into(),
            nonce,
        );

        let event = EmittedEvent {
            from_address: felt!(
                "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
            ),
            keys: vec![MESSAGE_SENT_EVENT_KEY, selector!("random_hash"), from_address, to_address],
            data: vec![selector, nonce, Felt::from(calldata.len() as u128), Felt::THREE],
            block_hash: Some(selector!("block_hash")),
            block_number: Some(0),
            transaction_hash,
        };

        let message_hash = compute_starknet_to_appchain_message_hash(
            from_address,
            to_address,
            nonce,
            selector,
            &calldata,
        );

        let expected = L1HandlerTx {
            nonce,
            calldata,
            chain_id,
            message_hash: B256::from_slice(message_hash.to_bytes_be().as_slice()),
            paid_fee_on_l1: 30000_u128,
            version: Felt::ZERO,
            entry_point_selector: selector,
            contract_address: to_address.into(),
        };

        let tx = l1_handler_tx_from_event(&event, chain_id).unwrap();

        assert_eq!(tx, expected);
    }

    #[test]
    #[should_panic]
    fn l1_handler_tx_from_event_parse_bad_selector() {
        let from_address = selector!("from_address");
        let to_address = selector!("to_address");
        let selector = selector!("selector");
        let nonce = Felt::ONE;
        let calldata = [from_address, Felt::THREE];
        let transaction_hash = Felt::ZERO;

        let event = EmittedEvent {
            from_address: felt!(
                "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
            ),
            keys: vec![
                selector!("AnOtherUnexpectedEvent"),
                selector!("random_hash"),
                from_address,
                to_address,
            ],
            data: vec![selector, nonce, Felt::from(calldata.len() as u128), Felt::THREE],
            block_hash: Some(selector!("block_hash")),
            block_number: Some(0),
            transaction_hash,
        };

        let _tx = l1_handler_tx_from_event(&event, ChainId::default()).unwrap();
    }

    #[test]
    #[should_panic]
    fn l1_handler_tx_from_event_parse_missing_key_data() {
        let from_address = selector!("from_address");
        let _to_address = selector!("to_address");
        let _selector = selector!("selector");
        let _nonce = Felt::ONE;
        let _calldata = [from_address, Felt::THREE];
        let transaction_hash = Felt::ZERO;

        let event = EmittedEvent {
            from_address: felt!(
                "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
            ),
            keys: vec![selector!("AnOtherUnexpectedEvent"), selector!("random_hash"), from_address],
            data: vec![],
            block_hash: Some(selector!("block_hash")),
            block_number: Some(0),
            transaction_hash,
        };

        let _tx = l1_handler_tx_from_event(&event, ChainId::default()).unwrap();
    }
}
