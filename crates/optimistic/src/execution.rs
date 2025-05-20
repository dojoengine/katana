use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::Result;
use futures::StreamExt;
use katana_pool::{pending::PendingTransactions, TransactionPool};
use tokio::sync::watch::Receiver;

use crate::pool::TxPool;

pub struct Service {
    block_rx: Receiver<Option<BlockEnv>>,
    pending_transactions: PendingTransactions<
        <TxPool as TransactionPool>::Transaction,
        <TxPool as TransactionPool>::Ordering,
    >,
}

impl Future for Service {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        return Poll::Pending;
    }
}
