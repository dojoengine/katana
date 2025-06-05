use jsonrpsee::server::middleware::http::ProxyGetRequestLayer;
use jsonrpsee::{Methods, ResponsePayload, RpcModule};
use serde_json::json;

/// Simple health check endpoint.
#[derive(Debug)]
pub struct HealthCheck;

impl HealthCheck {
    const METHOD: &'static str = "health";
    const PROXY_PATH: &'static str = "/";

    pub(crate) fn proxy() -> ProxyGetRequestLayer {
        Self::proxy_with_path(Self::PROXY_PATH)
    }

    fn proxy_with_path(path: &str) -> ProxyGetRequestLayer {
        ProxyGetRequestLayer::new([(path, Self::METHOD)]).expect("path starts with /")
    }
}

impl From<HealthCheck> for Methods {
    fn from(_: HealthCheck) -> Self {
        let mut module = RpcModule::new(());

        module
            .register_method(HealthCheck::METHOD, |_, _, _| {
                ResponsePayload::success(json!({ "health": true }))
            })
            .unwrap();

        module.into()
    }
}
