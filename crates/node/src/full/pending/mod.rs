use std::sync::Arc;
use std::time::Duration;

use katana_gateway::client::Client;
use katana_gateway::types::{ConfirmedTransaction, ErrorCode, PreConfirmedBlock, StateDiff};
use katana_primitives::block::BlockNumber;
use katana_primitives::state::StateUpdates;
use katana_provider::api::state::StateFactoryProvider;
use parking_lot::Mutex;
use tokio::sync::watch;
use tracing::error;

use crate::full::pending::state::PreconfStateProvider;
use crate::full::tip_watcher::TipSubscription;

mod provider;
pub mod state;

#[derive(Debug)]
pub struct PreconfStateFactory<P: StateFactoryProvider> {
    // from pipeline
    latest_synced_block: watch::Receiver<BlockNumber>,
    gateway_client: Client,
    provider: P,

    // shared state
    shared_preconf_block: SharedPreconfBlockData,
}

impl<P: StateFactoryProvider> PreconfStateFactory<P> {
    pub fn new(
        state_factory_provider: P,
        gateway_client: Client,
        latest_synced_block: watch::Receiver<BlockNumber>,
        tip_subscription: TipSubscription,
    ) -> Self {
        let shared_preconf_block = SharedPreconfBlockData::default();

        let mut worker = PreconfBlockWatcher {
            interval: DEFAULT_INTERVAL,
            latest_block: tip_subscription,
            gateway_client: gateway_client.clone(),
            latest_synced_block: latest_synced_block.clone(),
            shared_preconf_block: shared_preconf_block.clone(),
        };

        tokio::spawn(async move { worker.run().await });

        Self {
            gateway_client,
            latest_synced_block,
            shared_preconf_block,
            provider: state_factory_provider,
        }
    }

    pub fn state(&self) -> PreconfStateProvider {
        let latest_block_num = *self.latest_synced_block.borrow();
        let base = self.provider.historical(latest_block_num.into()).unwrap().unwrap();

        let preconf_block = self.shared_preconf_block.inner.lock();
        let preconf_block_id = preconf_block.as_ref().map(|b| b.preconf_block_id);
        let preconf_state_updates = preconf_block.as_ref().map(|b| b.preconf_state_updates.clone());

        PreconfStateProvider {
            base,
            preconf_block_id,
            preconf_state_updates,
            gateway: self.gateway_client.clone(),
        }
    }

    pub fn state_updates(&self) -> Option<StateUpdates> {
        if let Some(preconf_data) = self.shared_preconf_block.inner.lock().as_ref() {
            Some(preconf_data.preconf_state_updates.clone())
        } else {
            None
        }
    }

    pub fn block(&self) -> Option<PreConfirmedBlock> {
        if let Some(preconf_data) = self.shared_preconf_block.inner.lock().as_ref() {
            Some(preconf_data.preconf_block.clone())
        } else {
            None
        }
    }

    pub fn transactions(&self) -> Option<Vec<ConfirmedTransaction>> {
        if let Some(preconf_data) = self.shared_preconf_block.inner.lock().as_ref() {
            Some(preconf_data.preconf_block.transactions.clone())
        } else {
            None
        }
    }
}

#[derive(Debug, Default, Clone)]
struct SharedPreconfBlockData {
    inner: Arc<Mutex<Option<PreconfBlockData>>>,
}

#[derive(Debug)]
struct PreconfBlockData {
    preconf_block_id: BlockNumber,
    preconf_block: PreConfirmedBlock,
    preconf_state_updates: StateUpdates,
}

const DEFAULT_INTERVAL: Duration = Duration::from_millis(500);

struct PreconfBlockWatcher {
    interval: Duration,
    gateway_client: Client,

    // from pipeline
    latest_synced_block: watch::Receiver<BlockNumber>,
    // from tip watcher (actual tip of the chain)
    latest_block: TipSubscription,

    // shared state
    shared_preconf_block: SharedPreconfBlockData,
}

impl PreconfBlockWatcher {
    async fn run(&mut self) {
        let mut current_preconf_block_num = *self.latest_synced_block.borrow() + 1;

        loop {
            if current_preconf_block_num >= self.latest_block.tip() {
                match self.gateway_client.get_preconfirmed_block(current_preconf_block_num).await {
                    Ok(preconf_block) => {
                        let preconf_state_diff: StateUpdates = preconf_block
                            .transaction_state_diffs
                            .clone()
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
                        let mut shared_data_lock = self.shared_preconf_block.inner.lock();
                        if let Some(block) = shared_data_lock.as_mut() {
                            block.preconf_block = preconf_block;
                            block.preconf_block_id = current_preconf_block_num;
                            block.preconf_state_updates = preconf_state_diff;
                        } else {
                            *shared_data_lock = Some(PreconfBlockData {
                                preconf_block,
                                preconf_state_updates: preconf_state_diff,
                                preconf_block_id: current_preconf_block_num,
                            })
                        }
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

                    let latest_synced_block_num = *self.latest_synced_block.borrow();
                    current_preconf_block_num = latest_synced_block_num + 1;
                }

                _ = tokio::time::sleep(self.interval) => {
                    current_preconf_block_num += 1;
                }
            }
        }
    }
}
