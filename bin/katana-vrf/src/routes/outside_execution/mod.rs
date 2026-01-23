pub mod context;
pub mod signature;
pub mod types;
pub mod vrf_types;

use crate::routes::outside_execution::context::{RequestContext, VrfContext};
use crate::routes::outside_execution::signature::sign_outside_execution;
use crate::routes::outside_execution::types::{
    Call, OutsideExecution, OutsideExecutionV2, SignedOutsideExecution,
};
use crate::routes::outside_execution::vrf_types::{build_submit_random_call, RequestRandom};
use crate::state::SharedState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use cainome_cairo_serde::CairoSerde;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use starknet::core::types::Felt;
use starknet::core::utils::CairoShortStringToFeltError;
use starknet::macros::felt;
use starknet::providers::ProviderError;
use starknet::signers::{LocalWallet, SigningKey};
use tracing::debug;

pub const ANY_CALLER: Felt = felt!("0x414e595f43414c4c4552"); // ANY_CALLER

#[derive(Debug, Serialize, Deserialize)]
pub struct OutsideExecutionRequest {
    pub request: SignedOutsideExecution,
    pub context: RequestContext,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OutsideExecutionResult {
    pub result: SignedOutsideExecution,
}

// receive an OutsideExecution
// check for request_random
// build call [submit_random , execute_from_outside ]
// return signed OutsideExecution
pub async fn vrf_outside_execution(
    State(state): State<SharedState>,
    Json(payload): Json<OutsideExecutionRequest>,
) -> Result<Json<OutsideExecutionResult>, Errors> {
    debug!("received payload {payload:?}");

    let app_state = state.get().await;

    let signed_outside_execution = payload.request.clone();
    let outside_execution = signed_outside_execution.outside_execution.clone();
    let vrf_context = VrfContext::build_from(payload.context, &app_state)?;

    let (maybe_request_random_call, position) =
        RequestRandom::get_request_random_call(&outside_execution);

    if maybe_request_random_call.is_none() {
        return Err(Errors::NoRequestRandom);
    }
    let calls_len = outside_execution.calls().len();
    if position + 1 >= calls_len {
        return Err(Errors::NoCallAfterRequestRandom);
    }

    let request_random_call = maybe_request_random_call.unwrap();
    let request_random = RequestRandom::cairo_deserialize(&request_random_call.calldata, 0)?;

    let seed = request_random.compute_seed(&vrf_context).await?;

    let sumbit_random_call = build_submit_random_call(&vrf_context, seed);
    let execute_from_outside_call = signed_outside_execution.build_execute_from_outside_call();

    debug!("request_random: {:?}", request_random);
    debug!("seed: {:?}", seed);

    let calls = vec![sumbit_random_call, execute_from_outside_call];

    let signed_outside_execution = build_signed_outside_execution_v2(
        vrf_context.vrf_account_address.0,
        vrf_context.vrf_signer,
        vrf_context.chain_id,
        calls,
    )
    .await;

    Ok(Json(OutsideExecutionResult {
        result: signed_outside_execution,
    }))
}

pub async fn build_signed_outside_execution_v2(
    account_address: Felt,
    signer: LocalWallet,
    chain_id: Felt,
    calls: Vec<Call>,
) -> SignedOutsideExecution {
    let outside_execution = build_outside_execution_v2(calls);

    let signature =
        sign_outside_execution(&outside_execution, chain_id, account_address, signer).await;

    SignedOutsideExecution {
        address: account_address,
        outside_execution,
        signature,
    }
}
pub fn build_outside_execution_v2(calls: Vec<Call>) -> OutsideExecution {
    let now = Utc::now().timestamp() as u64;
    OutsideExecution::V2(OutsideExecutionV2 {
        caller: ANY_CALLER,
        execute_after: 0,
        execute_before: now + 600,
        calls,
        nonce: SigningKey::from_random().secret_scalar(),
    })
}

#[derive(Debug)]
pub enum Errors {
    NoRequestRandom,
    NoCallAfterRequestRandom,
    ProviderError(String),
    CairoSerdeError(String),
    RequestContextError(String),
    CairoShortStringToFeltError(String),
    UrlParserError(String),
}

impl IntoResponse for Errors {
    fn into_response(self) -> axum::response::Response {
        match self {
            Errors::NoRequestRandom => (
                StatusCode::NOT_FOUND,
                Json("No request_random call".to_string()),
            )
                .into_response(),
            Errors::NoCallAfterRequestRandom => (
                StatusCode::NOT_FOUND,
                Json("No call after request_random".to_string()),
            )
                .into_response(),
            Errors::ProviderError(msg) => (
                StatusCode::NOT_FOUND,
                Json(format!("Provider error: {msg}").to_string()),
            )
                .into_response(),
            Errors::CairoSerdeError(msg) => (
                StatusCode::NOT_FOUND,
                Json(format!("Cairo serde error: {msg}").to_string()),
            )
                .into_response(),
            Errors::RequestContextError(msg) => (
                StatusCode::NOT_FOUND,
                Json(format!("Request context error: {msg}").to_string()),
            )
                .into_response(),
            Errors::CairoShortStringToFeltError(msg) => (
                StatusCode::NOT_FOUND,
                Json(format!("Shortstring error: {msg}").to_string()),
            )
                .into_response(),
            Errors::UrlParserError(msg) => (
                StatusCode::NOT_FOUND,
                Json(format!("Url parser error: {msg}").to_string()),
            )
                .into_response(),
        }
    }
}

impl From<ProviderError> for Errors {
    fn from(value: ProviderError) -> Self {
        Errors::ProviderError(value.to_string())
    }
}

impl From<cainome_cairo_serde::Error> for Errors {
    fn from(value: cainome_cairo_serde::Error) -> Self {
        Errors::CairoSerdeError(value.to_string())
    }
}

impl From<CairoShortStringToFeltError> for Errors {
    fn from(value: CairoShortStringToFeltError) -> Self {
        Errors::CairoShortStringToFeltError(value.to_string())
    }
}

impl From<url::ParseError> for Errors {
    fn from(value: url::ParseError) -> Self {
        Errors::UrlParserError(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::outside_execution::types::{
        Call, OutsideExecution, OutsideExecutionV2, SignedOutsideExecution,
    };
    use crate::routes::outside_execution::vrf_types::{RequestRandom, Source};
    use crate::state::{AppState, SharedState};
    use crate::utils::felt_to_scalar;
    use axum::extract::State;
    use axum::Json;
    use cainome_cairo_serde::{CairoSerde, ContractAddress};
    use stark_vrf::generate_public_key;
    use starknet::core::types::Felt;
    use starknet::macros::{felt, selector};
    use starknet::signers::{LocalWallet, SigningKey};
    use std::sync::{Arc, RwLock};

    #[tokio::test]
    async fn wraps_request_random_call() {
        let secret_key = felt!("0x1");
        let public_key = generate_public_key(felt_to_scalar(secret_key));
        let vrf_account_address = ContractAddress::from(felt!("0x777"));
        let vrf_signer = LocalWallet::from(SigningKey::from_secret_scalar(felt!("0x2")));

        let app_state = AppState {
            secret_key,
            public_key,
            vrf_account_address,
            vrf_signer,
        };
        let shared_state = SharedState(Arc::new(RwLock::new(app_state)));

        let request_random = RequestRandom {
            caller: ContractAddress::from(felt!("0x123")),
            source: Source::Salt(felt!("0x456")),
        };
        let request_random_call = Call {
            to: vrf_account_address.0,
            selector: selector!("request_random"),
            calldata: RequestRandom::cairo_serialize(&request_random),
        };

        let user_call = Call {
            to: Felt::from(0x999_u128),
            selector: selector!("do_something"),
            calldata: vec![felt!("0x1")],
        };

        let outside_execution = OutsideExecution::V2(OutsideExecutionV2 {
            caller: ANY_CALLER,
            nonce: felt!("0x4"),
            execute_after: 0,
            execute_before: 100,
            calls: vec![request_random_call, user_call],
        });

        let signed = SignedOutsideExecution {
            address: felt!("0xabc"),
            outside_execution,
            signature: vec![felt!("0x5")],
        };
        let original_address = signed.address;

        let payload = OutsideExecutionRequest {
            request: signed.clone(),
            context: RequestContext {
                chain_id: "SN_SEPOLIA".to_string(),
                rpc_url: Some("http://localhost:5050".to_string()),
            },
        };

        let response = vrf_outside_execution(State(shared_state), Json(payload))
            .await
            .expect("vrf wrapping");
        let wrapped = response.0.result;

        let OutsideExecution::V2(v2) = wrapped.outside_execution else {
            panic!("expected v2 outside execution");
        };

        assert_eq!(v2.calls.len(), 2);
        assert_eq!(v2.calls[0].selector, selector!("submit_random"));
        assert_eq!(v2.calls[0].to, vrf_account_address.0);
        assert_eq!(v2.calls[1].selector, selector!("execute_from_outside_v2"));
        assert_eq!(v2.calls[1].to, original_address);
    }
}
