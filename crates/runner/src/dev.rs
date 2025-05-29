//! Katana Dev Client
//!
//! This simple clients exposes the Katana Dev API.
//!
//! This is a "duplicate" from the client that can be find
//! under the `katana-rpc-api` crate. However, this client
//! doesn't require to import `katana-rpc-api` which depends
//! on `katana-primitives` which introduce a coupling to the
//! cairo version that has to be used.
//!
//! In the same spirit that the Karana Runner is interacting
//! with Katana from the CLI to avoid any coupling, this
//! client is a simple client that doesn't depend on any
//! other crate of Katana.

use anyhow::Result;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;

#[derive(Debug, Clone)]
pub struct KatanaDevClient {
    client: HttpClient,
}

impl KatanaDevClient {
    pub fn new(url: &str) -> Result<Self> {
        let client = HttpClientBuilder::default().build(url)?;

        Ok(Self { client })
    }

    pub async fn generate_block(&self) -> Result<()> {
        self.client.request::<(), _>("dev_generateBlock", rpc_params![]).await?;
        Ok(())
    }
}
