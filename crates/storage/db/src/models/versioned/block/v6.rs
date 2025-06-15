use katana_primitives::block::{BlockHash, BlockNumber, GasPrice};
use katana_primitives::contract::ContractAddress;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::version::ProtocolVersion;
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, arbitrary::Arbitrary)]
pub struct Header {
    pub parent_hash: BlockHash,
    pub number: BlockNumber,
    pub state_diff_commitment: Felt,
    pub transactions_commitment: Felt,
    pub receipts_commitment: Felt,
    pub events_commitment: Felt,
    pub state_root: Felt,
    pub transaction_count: u32,
    pub events_count: u32,
    pub state_diff_length: u32,
    pub timestamp: u64,
    pub sequencer_address: ContractAddress,
    pub l1_gas_prices: GasPrice,
    pub l1_data_gas_prices: GasPrice,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub protocol_version: ProtocolVersion,
}

impl From<Header> for katana_primitives::block::Header {
    fn from(header: Header) -> Self {
        katana_primitives::block::Header {
            parent_hash: header.parent_hash,
            number: header.number,
            state_diff_commitment: header.state_diff_commitment,
            transactions_commitment: header.transactions_commitment,
            receipts_commitment: header.receipts_commitment,
            events_commitment: header.events_commitment,
            state_root: header.state_root,
            transaction_count: header.transaction_count,
            events_count: header.events_count,
            state_diff_length: header.state_diff_length,
            timestamp: header.timestamp,
            sequencer_address: header.sequencer_address,
            l1_gas_prices: header.l1_gas_prices,
            l1_data_gas_prices: header.l1_data_gas_prices,
            l1_da_mode: header.l1_da_mode,
            protocol_version: header.protocol_version,
            l2_gas_prices: GasPrice::MIN, /* this can't be zero for some reason, probably a check
                                           * performed by blockifier */
        }
    }
}
