use jsonrpsee::types::{ErrorObjectOwned, Request};
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::{FeeSource, OutsideExecution};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use starknet::core::types::SimulationFlagForEstimateFee;

pub(super) const STARKNET_ESTIMATE_FEE: &str = "starknet_estimateFee";
pub(super) const CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE: &str = "cartridge_addExecuteFromOutside";
pub(super) const CARTRIDGE_ADD_EXECUTE_FROM_OUTSIDE_TX: &str =
    "cartridge_addExecuteOutsideTransaction";

pub(super) fn parse_params<T: DeserializeOwned>(
    request: &Request<'_>,
) -> Result<T, ErrorObjectOwned> {
    request.params().parse()
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub(super) struct AddExecuteOutsideParams {
    pub address: ContractAddress,
    pub outside_execution: OutsideExecution,
    pub signature: Vec<Felt>,
    pub fee_source: Option<FeeSource>,
}

#[derive(Deserialize)]
pub(super) struct EstimateFeeParams {
    #[serde(alias = "request")]
    pub transactions: Vec<BroadcastedTx>,
    #[serde(alias = "simulationFlags")]
    pub simulation_flags: Vec<SimulationFlagForEstimateFee>,
    #[serde(alias = "blockId")]
    pub block_id: BlockIdOrTag,
}

#[derive(Deserialize)]
pub(super) struct AddExecuteOutsidePositionalParams(
    ContractAddress,
    OutsideExecution,
    Vec<Felt>,
    #[serde(default)] Option<FeeSource>,
);

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum AddExecuteOutsideRequestParams {
    Named(AddExecuteOutsideParams),
    Positional(AddExecuteOutsidePositionalParams),
}

impl From<AddExecuteOutsideRequestParams> for AddExecuteOutsideParams {
    fn from(value: AddExecuteOutsideRequestParams) -> Self {
        match value {
            AddExecuteOutsideRequestParams::Named(params) => params,
            AddExecuteOutsideRequestParams::Positional(params) => Self {
                address: params.0,
                outside_execution: params.1,
                signature: params.2,
                fee_source: params.3,
            },
        }
    }
}

#[derive(Deserialize)]
pub(super) struct EstimateFeePositionalParams(
    Vec<BroadcastedTx>,
    Vec<SimulationFlagForEstimateFee>,
    BlockIdOrTag,
);

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum EstimateFeeRequestParams {
    Named(EstimateFeeParams),
    Positional(EstimateFeePositionalParams),
}

impl From<EstimateFeeRequestParams> for EstimateFeeParams {
    fn from(value: EstimateFeeRequestParams) -> Self {
        match value {
            EstimateFeeRequestParams::Named(params) => params,
            EstimateFeeRequestParams::Positional(params) => {
                Self { transactions: params.0, simulation_flags: params.1, block_id: params.2 }
            }
        }
    }
}
