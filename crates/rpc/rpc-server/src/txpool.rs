use std::collections::BTreeMap;

use jsonrpsee::core::{async_trait, RpcResult};
use katana_pool::{PoolTransaction, TransactionPool};
use katana_primitives::ContractAddress;
use katana_rpc_api::txpool::TxPoolApiServer;
use katana_rpc_types::txpool::{TxPoolContent, TxPoolInspect, TxPoolStatus, TxPoolTransaction};

/// Handler for the `txpool_*` RPC namespace.
///
/// Operates on the node's local transaction pool (not a network-wide mempool).
/// Generic over any [`TransactionPool`] implementation, so it works with both
/// the sequencer pool ([`TxPool`]) and the full-node pool ([`FullNodePool`]).
#[allow(missing_debug_implementations)]
pub struct TxPoolApi<P> {
    pool: P,
}

impl<P> TxPoolApi<P> {
    pub fn new(pool: P) -> Self {
        Self { pool }
    }
}

impl<P: TransactionPool> TxPoolApi<P> {
    fn build_content(&self, filter: Option<ContractAddress>) -> TxPoolContent {
        let txs = self.pool.take_transactions_snapshot();
        let mut pending: BTreeMap<ContractAddress, BTreeMap<_, _>> = BTreeMap::new();

        for tx in txs {
            let sender = tx.sender();

            if let Some(addr) = filter {
                if sender != addr {
                    continue;
                }
            }

            let entry = TxPoolTransaction {
                hash: tx.hash(),
                nonce: tx.nonce(),
                sender,
                max_fee: tx.max_fee(),
                tip: tx.tip(),
            };

            pending.entry(sender).or_default().insert(tx.nonce(), entry);
        }

        TxPoolContent { pending, queued: BTreeMap::new() }
    }
}

#[async_trait]
impl<P: TransactionPool + 'static> TxPoolApiServer for TxPoolApi<P> {
    async fn txpool_status(&self) -> RpcResult<TxPoolStatus> {
        let pending = self.pool.size() as u64;
        Ok(TxPoolStatus { pending, queued: 0 })
    }

    async fn txpool_content(&self) -> RpcResult<TxPoolContent> {
        Ok(self.build_content(None))
    }

    async fn txpool_content_from(&self, address: ContractAddress) -> RpcResult<TxPoolContent> {
        Ok(self.build_content(Some(address)))
    }

    async fn txpool_inspect(&self) -> RpcResult<TxPoolInspect> {
        let txs = self.pool.take_transactions_snapshot();
        let mut pending: BTreeMap<ContractAddress, BTreeMap<_, _>> = BTreeMap::new();

        for tx in txs {
            let summary = format!(
                "hash={:#x} nonce={:#x} max_fee={} tip={}",
                tx.hash(),
                tx.nonce(),
                tx.max_fee(),
                tx.tip(),
            );

            pending.entry(tx.sender()).or_default().insert(tx.nonce(), summary);
        }

        Ok(TxPoolInspect { pending, queued: BTreeMap::new() })
    }
}
