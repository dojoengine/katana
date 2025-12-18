use std::cmp::Ordering;
use std::ops::RangeInclusive;

use anyhow::Context;
use katana_primitives::block::{BlockHash, BlockHashOrNumber, BlockNumber};
use katana_primitives::contract::ContractAddress;
use katana_primitives::event::ContinuationToken;
use katana_primitives::receipt::Event;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_provider::api::block::BlockProvider;
use katana_provider::api::transaction::ReceiptProvider;
use katana_provider::api::ProviderError;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::event::EmittedEvent;
use katana_rpc_types::PreConfirmedBlockWithReceipts;

pub type EventQueryResult<T> = Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid cursor")]
    InvalidCursor,
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug)]
pub enum EventBlockId {
    Pending,
    Num(BlockNumber),
}

/// An object to specify how events should be filtered.
#[derive(Debug, Default, Clone)]
pub struct Filter {
    /// The contract address to filter by.
    ///
    /// If `None`, all events are considered. If `Some`, only events emitted by the specified
    /// contract are considered.
    pub address: Option<ContractAddress>,
    /// The keys to filter by.
    pub keys: Option<Vec<Vec<Felt>>>,
}

/// Internal cursor
#[derive(Debug, Clone, PartialEq)]
pub struct Cursor {
    block: u64,
    txn: PartialCursor,
}

impl Cursor {
    pub fn new(block: u64, txn: usize, event: usize) -> Self {
        Self { block, txn: PartialCursor { idx: txn, event } }
    }

    pub fn new_block(block: u64) -> Self {
        Self { block, txn: PartialCursor::default() }
    }

    pub fn into_rpc_cursor(self) -> ContinuationToken {
        ContinuationToken {
            block_n: self.block,
            txn_n: self.txn.idx as u64,
            event_n: self.txn.event as u64,
        }
    }
}

/// A partial cursor that points to a specific event WITHIN a transaction.
#[derive(Debug, Clone, PartialEq, Default)]
struct PartialCursor {
    /// The transaction index within a block.
    idx: usize,
    /// The event index within a transaction.
    event: usize,
}

impl PartialCursor {
    fn into_full(self, block: BlockNumber) -> Cursor {
        Cursor { block, txn: self }
    }
}

#[allow(unused)]
pub fn fetch_pending_events(
    pending_block: &PreConfirmedBlockWithReceipts,
    filter: &Filter,
    chunk_size: u64,
    cursor: Option<Cursor>,
    buffer: &mut Vec<EmittedEvent>,
) -> EventQueryResult<Cursor> {
    let cursor = cursor.unwrap_or(Cursor::new_block(pending_block.block_number));

    // process individual transactions in the block.
    // the iterator will start with txn index == cursor.txn.idx
    for (tx_idx, (tx_hash, tx_events)) in pending_block
        .transactions
        .iter()
        .map(|receipt| (receipt.receipt.transaction_hash, receipt.receipt.receipt.events()))
        .enumerate()
        .skip(cursor.txn.idx)
    {
        if tx_idx == cursor.txn.idx {
            match tx_events.len().cmp(&cursor.txn.event) {
                Ordering::Equal | Ordering::Greater => {}
                Ordering::Less => continue,
            }
        }

        // we should only skip for the last txn pointed by the cursor.
        let next_event = if tx_idx == cursor.txn.idx { cursor.txn.event } else { 0 };
        let partial_cursor = fetch_tx_events(
            next_event,
            None,
            None,
            tx_idx,
            tx_hash,
            tx_events,
            filter,
            chunk_size as usize,
            buffer,
        )?;

        if let Some(c) = partial_cursor {
            return Ok(c.into_full(pending_block.block_number));
        }
    }

    // if we reach here, it means we have processed all the transactions in the pending block.
    // we return a cursor that points to the next tx in the pending block.
    let next_pending_tx_idx = pending_block.transactions.len();
    Ok(Cursor::new(pending_block.block_number, next_pending_tx_idx, 0))
}

/// Returns `true` if reach the end of the block range.
pub fn fetch_events_at_blocks(
    provider: impl BlockProvider + ReceiptProvider,
    block_range: RangeInclusive<BlockNumber>,
    filter: &Filter,
    chunk_size: u64,
    cursor: Option<Cursor>,
    buffer: &mut Vec<EmittedEvent>,
) -> EventQueryResult<Option<Cursor>> {
    let cursor = cursor.unwrap_or(Cursor::new_block(*block_range.start()));

    // update the block range to start from the block pointed by the cursor.
    let block_range = cursor.block..=*block_range.end();

    for block_num in block_range {
        // collect all receipts at `block_num` block.
        let block_hash =
            provider.block_hash_by_num(block_num)?.context("Block hash should exist")?;

        let block_id = BlockHashOrNumber::from(block_num);
        let receipts =
            provider.receipts_by_block(block_id)?.context("Block receipts should exist")?;
        let txs =
            provider.transactions_by_block(block_id)?.context("Block transactions should exist")?;
        let tx_hashes = txs.into_iter().map(|tx| tx.hash).collect::<Vec<TxHash>>();

        if block_num == cursor.block {
            match receipts.len().cmp(&cursor.txn.idx) {
                Ordering::Equal | Ordering::Greater => {}
                Ordering::Less => continue,
            }
        }

        // we should only skip for the last block pointed by the cursor.
        let total_tx_to_skip = if block_num == cursor.block { cursor.txn.idx } else { 0 };

        // skip number of transactions as specified in the continuation token
        for (tx_idx, (tx_hash, events)) in tx_hashes
            .into_iter()
            .zip(receipts.iter().map(|r| r.events()))
            .enumerate()
            .skip(total_tx_to_skip)
        {
            // Determine the next event index to start processing.
            let next_event =
            // Check if the block AND tx we're currently processing is exactly the one pointed by the cursor.
            //
            // If yes, then we check whether (1) the event index pointed by the cursor is less than
            // OR (2) exceed the total number of events in the current transaction.
            if block_num == cursor.block && tx_idx == cursor.txn.idx {
                // If its (1), then that means there are still some events left to process in
                // the current transaction. Else if its (2), meaning the cursor is pointing to either the
                // last event or out of bound, which we can just skip to the next transaction.
                match cursor.txn.event.cmp(&events.len()) {
                    Ordering::Less => cursor.txn.event,
                    Ordering::Greater | Ordering::Equal => continue,
                }
            }
            // If we're not processing the block and tx pointed by the cursor, then we start from 0
            else {
                0
            };

            let partial_cursor = fetch_tx_events(
                next_event,
                Some(block_num),
                Some(block_hash),
                tx_idx,
                tx_hash,
                events,
                filter,
                chunk_size as usize,
                buffer,
            )?;

            if let Some(c) = partial_cursor {
                return Ok(Some(c.into_full(block_num)));
            }
        }
    }

    // if we reach here, it means we have processed all the blocks in the range.
    // therefore we don't need to return a cursor.
    Ok(None)
}

/// An iterator that yields events (with their original indices) that match the given filters.
#[derive(Debug)]
struct FilteredEvents<'a, I: Iterator<Item = (usize, &'a Event)>> {
    iter: I,
    filter: &'a Filter,
}

impl<'a, I: Iterator<Item = (usize, &'a Event)>> FilteredEvents<'a, I> {
    fn new(iter: I, filter: &'a Filter) -> Self {
        Self { iter, filter }
    }
}

impl<'a, I: Iterator<Item = (usize, &'a Event)>> Iterator for FilteredEvents<'a, I> {
    type Item = (usize, &'a Event);

    fn next(&mut self) -> Option<Self::Item> {
        for (idx, event) in self.iter.by_ref() {
            // Skip this event if there is an address filter but doesn't match the address of the
            // event.
            if self.filter.address.is_some_and(|addr| addr != event.from_address) {
                continue;
            }

            // Check if the event matches the keys filter
            let is_matched = match &self.filter.keys {
                None => true,
                // From starknet-api spec:
                // Per key (by position), designate the possible values to be matched for events to
                // be returned. Empty array designates 'any' value"
                Some(filters) => filters.iter().enumerate().all(|(i, keys)| {
                    // Lets say we want to filter events which are either named `Event1` or `Event2`
                    // and custom key `0x1` or `0x2` Filter:
                    // [[sn_keccak("Event1"), sn_keccak("Event2")], ["0x1", "0x2"]]

                    // This checks: number of keys in event >= number of keys in filter (we check >
                    // i and not >= i because i is zero indexed) because
                    // otherwise this event doesn't contain all the keys we
                    // requested
                    event.keys.len() > i &&
                         // This checks: Empty array desginates 'any' value
                         (keys.is_empty()
                         ||
                         // This checks: If this events i'th value is one of the requested value in filter_keys[i]
                         keys.contains(&event.keys[i]))
                }),
            };

            if is_matched {
                return Some((idx, event));
            }
        }

        None
    }
}

/// Fetches events from a transaction, applying filters and respecting chunk size limits.
///
/// Returns a cursor if it couldn't include all the events of the current transaction because
/// the buffer is already full. Otherwise, if it is able to include all the transactions,
/// returns None.
///
/// # Arguments
///
/// * `next_event_idx` - The index of the transaction in the current transaction to start from
/// * `block_number` - Block number of the current transaction
/// * `block_hash` - Block hash of the current transaction
/// * `tx_idx` - Index of the current transaction in the block
/// * `tx_hash` - Hash of the current transaction
/// * `events` - All events in the current transaction
/// * `filter` - The filter to apply on the events
/// * `chunk_size` - Maximum number of events that can be taken, based on user-specified chunk size
/// * `buffer` - Buffer to store the matched events
#[allow(clippy::too_many_arguments)]
fn fetch_tx_events(
    next_event_idx: usize,
    block_number: Option<BlockNumber>,
    block_hash: Option<BlockHash>,
    tx_idx: usize,
    tx_hash: TxHash,
    tx_events: &[Event],
    filter: &Filter,
    chunk_size: usize,
    buffer: &mut Vec<EmittedEvent>,
) -> EventQueryResult<Option<PartialCursor>> {
    // calculate the remaining capacity based on the chunk size and the current
    // number of events we have taken.
    let total_can_take = chunk_size.saturating_sub(buffer.len());

    // Enumerate events first to preserve original indices, then filter.
    // skip events according to the continuation token.
    let filtered = FilteredEvents::new(tx_events.iter().enumerate(), filter)
        .map(|(event_idx, e)| {
            (
                event_idx,
                EmittedEvent {
                    block_hash,
                    block_number,
                    keys: e.keys.clone(),
                    data: e.data.clone(),
                    transaction_hash: tx_hash,
                    from_address: e.from_address,
                    transaction_index: tx_idx as u64,
                    event_index: event_idx as u64,
                },
            )
        })
        .skip(next_event_idx)
        .take(total_can_take)
        .collect::<Vec<_>>();

    // remaining possible events that we haven't seen due to the chunk size limit.
    let total_events_traversed = next_event_idx + total_can_take;

    // get the index of the last matching event that we have reached. if there is not
    // matching events (ie `filtered` is empty) we point to the end of the chunk
    // we've covered thus far using the iterator..
    let last_event_idx = filtered.last().map(|(idx, _)| *idx).unwrap_or(total_events_traversed);
    buffer.extend(filtered.into_iter().map(|(_, event)| event));

    if buffer.len() >= chunk_size {
        // the next time we have to fetch the events, we will start from this index.
        let new_last_event = if total_can_take == 0 {
            // start from the same event pointed by the
            // current cursor..
            last_event_idx
        } else {
            // start at the next event of the last event we've filtered out.
            last_event_idx + 1
        };

        // if there are still more events that we haven't fetched yet for this tx.
        if new_last_event < tx_events.len() {
            return Ok(Some(PartialCursor { idx: tx_idx, event: new_last_event }));
        }
    }

    Ok(None)
}

impl From<Error> for StarknetApiError {
    fn from(error: Error) -> Self {
        match error {
            Error::InvalidCursor => Self::InvalidContinuationToken,
            Error::Provider(e) => e.into(),
            Error::Other(e) => e.into(),
        }
    }
}

impl From<ContinuationToken> for Cursor {
    fn from(token: ContinuationToken) -> Self {
        Cursor::new(token.block_n, token.txn_n as usize, token.event_n as usize)
    }
}
