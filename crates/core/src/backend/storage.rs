use katana_provider::api::block::{BlockIdReader, BlockProvider, BlockWriter};
use katana_provider::api::contract::ContractClassWriter;
use katana_provider::api::env::BlockEnvProvider;
use katana_provider::api::stage::StageCheckpointProvider;
use katana_provider::api::state::{StateFactoryProvider, StateWriter};
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::transaction::{
    ReceiptProvider, TransactionProvider, TransactionStatusProvider, TransactionTraceProvider,
    TransactionsProviderExt,
};
use katana_provider::api::trie::TrieWriter;
use katana_provider::MutableProvider;

pub trait ProviderRO:
    BlockIdReader
    + BlockProvider
    + TransactionProvider
    + TransactionStatusProvider
    + TransactionTraceProvider
    + TransactionsProviderExt
    + ReceiptProvider
    + StateUpdateProvider
    + StateFactoryProvider
    + BlockEnvProvider
    + 'static
    + Send
    + Sync
    + core::fmt::Debug
{
}

pub trait ProviderRW:
    MutableProvider
    + ProviderRO
    + BlockWriter
    + StateWriter
    + ContractClassWriter
    + TrieWriter
    + StageCheckpointProvider
{
}

impl<T> ProviderRO for T where
    T: BlockProvider
        + BlockIdReader
        + TransactionProvider
        + TransactionStatusProvider
        + TransactionTraceProvider
        + TransactionsProviderExt
        + ReceiptProvider
        + StateUpdateProvider
        + StateFactoryProvider
        + BlockEnvProvider
        + 'static
        + Send
        + Sync
        + core::fmt::Debug
{
}

impl<T> ProviderRW for T where
    T: ProviderRO
        + MutableProvider
        + BlockWriter
        + StateWriter
        + ContractClassWriter
        + TrieWriter
        + StageCheckpointProvider
{
}
