use std::net::SocketAddr;
use std::sync::Arc;

use katana_chain_spec::{dev, ChainSpec};
use katana_core::backend::storage::Database;
use katana_core::backend::Backend;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_node::config::dev::DevConfig;
use katana_node::config::paymaster::PaymasterConfig;
use katana_node::config::rpc::{RpcConfig, RpcModulesList, DEFAULT_RPC_ADDR};
use katana_node::config::sequencing::SequencingConfig;
use katana_node::config::Config;
use katana_node::{LaunchedNode, Node};
use katana_primitives::chain::ChainId;
use katana_primitives::{address, ContractAddress};
use katana_provider::BlockchainProvider;
use katana_rpc::HttpClient;
use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::BlockTag;
pub use starknet::core::types::StarknetError;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Url};
pub use starknet::providers::{Provider, ProviderError};
use starknet::signers::{LocalWallet, SigningKey};

#[derive(Debug)]
pub struct TestNode {
    node: LaunchedNode,
}

impl TestNode {
    pub async fn new() -> Self {
        Self::new_with_config(test_config()).await
    }

    pub async fn new_with_block_time(block_time: u64) -> Self {
        let mut config = test_config();
        config.sequencing.block_time = Some(block_time);
        Self::new_with_config(config).await
    }

    pub async fn new_with_config(config: Config) -> Self {
        Self {
            node: Node::build(config)
                .await
                .expect("failed to build node")
                .launch()
                .await
                .expect("failed to launch node"),
        }
    }

    /// Returns the address of the node's RPC server.
    pub fn rpc_addr(&self) -> &SocketAddr {
        self.node.rpc().addr()
    }

    pub fn backend(&self) -> &Arc<Backend<BlockifierFactory>> {
        self.node.node().backend()
    }

    /// Returns a reference to the blockchain provider.
    pub fn blockchain(&self) -> &BlockchainProvider<Box<dyn Database>> {
        self.backend().blockchain.provider()
    }

    /// Returns a reference to the launched node handle.
    pub fn handle(&self) -> &LaunchedNode {
        &self.node
    }

    pub fn starknet_provider(&self) -> JsonRpcClient<HttpTransport> {
        let url = Url::parse(&format!("http://{}", self.rpc_addr())).expect("failed to parse url");
        JsonRpcClient::new(HttpTransport::new(url))
    }

    pub fn account(&self) -> SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet> {
        let (address, account) =
            self.backend().chain_spec.genesis().accounts().next().expect("must have at least one");
        let private_key = account.private_key().expect("must exist");
        let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(private_key));

        let mut account = SingleOwnerAccount::new(
            self.starknet_provider(),
            signer,
            (*address).into(),
            self.backend().chain_spec.id().into(),
            ExecutionEncoding::New,
        );

        account.set_block_id(starknet::core::types::BlockId::Tag(BlockTag::Pending));

        account
    }

    /// Returns a HTTP client to the JSON-RPC server.
    pub fn rpc_http_client(&self) -> HttpClient {
        self.handle().rpc().http_client().expect("failed to get http client for the rpc server")
    }
}

pub fn test_config() -> Config {
    let sequencing = SequencingConfig::default();
    let dev = DevConfig { fee: false, account_validation: true, fixed_gas_prices: None };

    let mut chain = dev::ChainSpec { id: ChainId::SEPOLIA, ..Default::default() };
    chain.genesis.sequencer_address = address!("0x1");

    let rpc = RpcConfig {
        port: 0,
        addr: DEFAULT_RPC_ADDR,
        apis: RpcModulesList::all(),
        max_proof_keys: Some(100),
        max_event_page_size: Some(100),
        max_concurrent_estimate_fee_requests: None,
        ..Default::default()
    };

    let cartridge_api_url = Url::parse("http://localhost:6969").unwrap();

    Config {
        sequencing,
        rpc,
        dev,
        chain: ChainSpec::Dev(chain).into(),
        paymaster: Some(PaymasterConfig { cartridge_api_url }),
        ..Default::default()
    }
}
