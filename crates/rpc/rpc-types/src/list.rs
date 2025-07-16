//! Types for list endpoints (blocks and transactions)

use core::fmt;
use std::num::ParseIntError;

use katana_primitives::block::BlockNumber;
use katana_primitives::transaction::TxNumber;
use serde::{Deserialize, Serialize};

use crate::block::BlockWithTxHashes;
use crate::transaction::Tx;

/// Represents a continuation token for implementing paging in block and transaction queries.
///
/// This struct stores the necessary information to resume fetching blocks or transactions
/// from a specific point relative to the given filter passed as parameter to the
/// `starknet_getBlocks` or `starknet_getTransactions` API.
///
/// The JSON-RPC specification does not specify the format of the continuation token,
/// so how the node should handle it is implementation specific.
#[derive(PartialEq, Eq, Debug, Clone, Default)]
pub struct ContinuationToken {
    /// The item (block/transaction) number to continue from.
    pub item_n: u64,
}

#[derive(PartialEq, Eq, Debug, thiserror::Error)]
pub enum ContinuationTokenError {
    #[error("Invalid data")]
    InvalidToken,
    #[error("Invalid format: {0}")]
    ParseFailed(ParseIntError),
}

impl ContinuationToken {
    pub fn parse(token: &str) -> Result<Self, ContinuationTokenError> {
        let item_n = u64::from_str_radix(token, 16).map_err(ContinuationTokenError::ParseFailed)?;
        Ok(ContinuationToken { item_n })
    }
}

impl fmt::Display for ContinuationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}", self.item_n)
    }
}

/// Request parameters for the `starknet_getBlocks` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlocksRequest {
    /// The starting block number (inclusive). For descending order, this should be higher than `end_block`.
    pub from: BlockNumber,

    /// The ending block number (inclusive). If not provided, returns blocks starting from `start_block`.
    /// For descending order, this should be lower than `start_block`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<BlockNumber>,

    /// The maximum number of blocks to return. If not provided, returns all blocks in the range.
    /// This acts as a limit to prevent excessively large responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_size: Option<u64>,
}

/// Response for the `starknet_getBlocks` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlocksResponse {
    /// The list of blocks.
    pub blocks: Vec<BlockWithTxHashes>,

    /// A pointer to the last element of the delivered page, use this token in a subsequent query to
    /// obtain the next page. If the value is `None`, don't add it to the response as clients might
    /// use `contains_key` as a check for the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
}

/// Request parameters for the `starknet_getTransactions` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionsRequest {
    /// The starting transaction number (inclusive). For descending order, this should be higher than `end_tx`.
    pub from: TxNumber,

    /// The ending transaction number (inclusive). If not provided, returns transactions starting from `start_tx`.
    /// For descending order, this should be lower than `start_tx`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<TxNumber>,

    /// The maximum number of transactions to return. If not provided, returns all transactions in the range.
    /// This acts as a limit to prevent excessively large responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_size: Option<u64>,
}

/// Response for the `starknet_getTransactions` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionsResponse {
    /// The list of transactions.
    pub transactions: Vec<Tx>,

    /// A pointer to the last element of the delivered page, use this token in a subsequent query to
    /// obtain the next page. If the value is `None`, don't add it to the response as clients might
    /// use `contains_key` as a check for the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn list_continuation_token_parse_works() {
        assert_eq!(ContinuationToken::parse("0").unwrap(), ContinuationToken { item_n: 0 });
        assert_eq!(ContinuationToken::parse("1e").unwrap(), ContinuationToken { item_n: 30 });
    }

    #[test]
    fn list_continuation_token_parse_should_fail() {
        assert_eq!(
            ContinuationToken::parse("0,").unwrap_err(),
            ContinuationTokenError::InvalidToken
        );
        assert_eq!(
            ContinuationToken::parse("0,0,0").unwrap_err(),
            ContinuationTokenError::InvalidToken
        );
    }

    #[test]
    fn list_continuation_token_parse_u64_should_fail() {
        matches!(
            ContinuationToken::parse("2y").unwrap_err(),
            ContinuationTokenError::ParseFailed(_)
        );
    }
}
