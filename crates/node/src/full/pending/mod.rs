use std::sync::Arc;
use std::time::Duration;

use katana_gateway::client::Client;
use katana_gateway::types::{
    ConfirmedTransaction, ErrorCode, GatewayError, PreConfirmedBlock, StateDiff,
    StateUpdateWithBlock,
};
use katana_primitives::block::{BlockHash, BlockNumber};
use katana_primitives::state::StateUpdates;
use katana_provider::api::state::StateFactoryProvider;
use parking_lot::Mutex;
use tokio::sync::watch;
use tracing::error;

use crate::full::pending::state::PreconfStateProvider;
use crate::full::tip_watcher::TipSubscription;

pub mod state;

const DEFAULT_INTERVAL: Duration = Duration::from_millis(500);

pub struct PreconfBlockWatcher {
    interval: Duration,
    gateway_client: Client,

    // from pipeline
    latest_synced_block: watch::Receiver<BlockNumber>,
    // from tip watcher (actual tip of the chain)
    latest_block: TipSubscription,

    // shared state
    pending_block_id: Arc<Mutex<BlockNumber>>,
    pending_state_updates: Arc<Mutex<StateUpdates>>,
}

impl PreconfBlockWatcher {
    pub fn new(gateway_client: Client, tip_subscription: TipSubscription) -> Self {
        todo!()
    }

    pub async fn run(&mut self) {
        let mut current_preconf_block_num = 0;

        loop {
            let latest_chain_tip = self.latest_block.tip();
            let latest_synced_block_num = *self.latest_synced_block.borrow();

            if latest_synced_block_num >= latest_chain_tip {
                let preconf_block_num = latest_synced_block_num + 1;

                match self.gateway_client.get_preconfirmed_block(preconf_block_num).await {
                    Ok(preconf_block) => {
                        let preconf_state_diff: StateUpdates = preconf_block
                            .transaction_state_diffs
                            .into_iter()
                            .fold(StateDiff::default(), |acc, diff| {
                                if let Some(diff) = diff {
                                    acc.merge(diff)
                                } else {
                                    acc
                                }
                            })
                            .into();

                        // update shared state
                        *self.pending_block_id.lock() = current_preconf_block_num;
                        *self.pending_state_updates.lock() = preconf_state_diff;

                        // increment to get the next preconf block number
                        current_preconf_block_num = preconf_block_num + 1;
                    }

                    // this could either be because the latest block is still not synced to the
                    // chain's tip, in which case we just skip to the next
                    // iteration.
                    Err(katana_gateway::client::Error::Sequencer(error))
                        if error.code == ErrorCode::BlockNotFound =>
                    {
                        continue
                    }

                    Err(err) => panic!("{err}"),
                }
            }

            tokio::select! {
                biased;

                res = self.latest_synced_block.changed() => {
                    if let Err(err) = res {
                        error!(error = ?err, "Error receiving latest block number.");
                        break;
                    }
                }

                _ = tokio::time::sleep(self.interval) => {}
            }
        }
    }
}

pub struct PreconfStateFactory<P: StateFactoryProvider> {
    // from pipeline
    latest_synced_block: watch::Receiver<BlockNumber>,
    gateway_client: Client,
    provider: P,

    // shared state
    preconf_block_id: Arc<Mutex<BlockNumber>>,
    preconf_block: Arc<Mutex<PreConfirmedBlock>>,
    preconf_state_updates: Arc<Mutex<StateUpdates>>,
}

impl<P: StateFactoryProvider> PreconfStateFactory<P> {
    pub fn new(
        state_factory_provider: P,
        gateway_client: Client,
        chain_tip_subscription: TipSubscription,
    ) -> Self {
        todo!()
    }

    pub fn state(&self) -> PreconfStateProvider {
        let latest_block_num = *self.latest_synced_block.borrow();
        let latest_state = self.provider.historical(latest_block_num.into()).unwrap().unwrap();

        PreconfStateProvider {
            base: latest_state,
            gateway: self.gateway_client.clone(),
            pending_block_id: self.preconf_block_id.lock().clone(),
            pending_state_updates: self.preconf_state_updates.lock().clone(),
        }
    }

    pub fn state_updates(&self) -> StateUpdates {
        self.preconf_state_updates.lock().clone()
    }

    pub fn block(&self) -> PreConfirmedBlock {
        self.preconf_block.lock().clone()
    }

    pub fn transactions(&self) -> Vec<ConfirmedTransaction> {
        self.preconf_block.lock().transactions.clone()
    }
}
