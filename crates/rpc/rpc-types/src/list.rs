//! Types for list endpoints (blocks and transactions)

use core::fmt;

use katana_primitives::block::BlockNumber;
use katana_primitives::transaction::TxNumber;
use serde::{Deserialize, Serialize};
use starknet::core::types::ResultPageRequest;

use crate::block::BlockWithTxHashes;
use crate::receipt::TxReceiptWithBlockInfo;

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
#[error("invalid token `{token}`: {error}")]
pub struct ContinuationTokenError {
    error: std::num::ParseIntError,
    token: String,
}

impl ContinuationToken {
    pub fn parse(token: &str) -> Result<Self, ContinuationTokenError> {
        str::parse(token)
    }
}

impl std::str::FromStr for ContinuationToken {
    type Err = ContinuationTokenError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        u64::from_str_radix(s, 16)
            .map(|item_n| ContinuationToken { item_n })
            .map_err(|error| ContinuationTokenError { error, token: s.to_string() })
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
    /// The starting block number (inclusive). For descending order, this should be higher than
    /// `end_block`.
    pub from: BlockNumber,

    /// The ending block number (inclusive). If not provided, returns blocks starting from
    /// `start_block`. For descending order, this should be lower than `start_block`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<BlockNumber>,

    pub result_page_request: ResultPageRequest,
}

/// Response for the `starknet_getBlocks` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlocksResponse {
    /// The list of blocks.
    pub blocks: Vec<BlockWithTxHashes>,

    /// A pointer to the last element of the delivered page, use this token in a subsequent query
    /// to obtain the next page. If the value is `None`, don't add it to the response as
    /// clients might use `contains_key` as a check for the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
}

/// Request parameters for the `starknet_getTransactions` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionsRequest {
    /// The starting transaction number (inclusive). For descending order, this should be higher
    /// than `end_tx`.
    pub from: TxNumber,

    /// The ending transaction number (inclusive). If not provided, returns transactions starting
    /// from `start_tx`. For descending order, this should be lower than `start_tx`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<TxNumber>,

    pub result_page_request: ResultPageRequest,
}

/// Response for the `starknet_getTransactions` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionsResponse {
    /// The list of transactions.
    pub transactions: Vec<TxReceiptWithBlockInfo>,

    /// A pointer to the last element of the delivered page, use this token in a subsequent query
    /// to obtain the next page. If the value is `None`, don't add it to the response as
    /// clients might use `contains_key` as a check for the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
}

#[cfg(test)]
mod test {
    use super::*;

    #[rstest::rstest]
    #[case("0", 0)]
    #[case("1e", 30)]
    #[case("ff", 255)]
    fn list_continuation_token_parse_works(#[case] input: &str, #[case] expected: u64) {
        let input = ContinuationToken::parse(input).unwrap();
        let expected = ContinuationToken { item_n: expected };
        assert_eq!(input, expected);
    }

    #[rstest::rstest]
    #[case::trailing_comma("0,")]
    #[case::multiple_commas("0,0,0")]
    #[case::invalid_hex_char("2y")]
    fn list_continuation_token_parse_should_fail(#[case] input: &str) {
        assert!(ContinuationToken::parse(input).is_err());
    }
}
