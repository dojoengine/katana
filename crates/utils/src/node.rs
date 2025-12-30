use std::net::SocketAddr;
use std::sync::Arc;

use katana_chain_spec::{dev, ChainSpec};
use katana_core::backend::storage::{ProviderRO, ProviderRW};
use katana_core::backend::Backend;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_node::config::dev::DevConfig;
use katana_node::config::rpc::{RpcConfig, RpcModulesList, DEFAULT_RPC_ADDR};
use katana_node::config::sequencing::{MiningMode, SequencingConfig};
use katana_node::config::Config;
use katana_node::{LaunchedNode, Node};
use katana_primitives::address;
use katana_primitives::chain::ChainId;
use katana_provider::{DbProviderFactory, ForkProviderFactory, ProviderFactory};
use katana_rpc_server::HttpClient;
use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::BlockTag;
pub use starknet::core::types::StarknetError;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Url};
pub use starknet::providers::{Provider, ProviderError};
use starknet::signers::{LocalWallet, SigningKey};

pub type ForkTestNode = TestNode<ForkProviderFactory>;

#[derive(Debug)]
pub struct TestNode<P = DbProviderFactory>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    node: LaunchedNode<P>,
}

impl TestNode {
    pub async fn new() -> Self {
        Self::new_with_config(test_config()).await
    }

    pub async fn new_with_block_time(block_time: u64) -> Self {
        let mut config = test_config();
        config.sequencing.mining = MiningMode::Interval(block_time);
        Self::new_with_config(config).await
    }

    pub async fn new_with_config(config: Config) -> Self {
        Self {
            node: Node::build(config)
                .expect("failed to build node")
                .launch()
                .await
                .expect("failed to launch node"),
        }
    }
}

impl ForkTestNode {
    pub async fn new_forked_with_config(config: Config) -> Self {
        Self {
            node: Node::build_forked(config)
                .await
                .expect("failed to build node")
                .launch()
                .await
                .expect("failed to launch node"),
        }
    }
}

impl<P> TestNode<P>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    /// Returns the address of the node's RPC server.
    pub fn rpc_addr(&self) -> &SocketAddr {
        self.node.rpc().addr()
    }

    pub fn backend(&self) -> &Arc<Backend<BlockifierFactory, P, ChainSpec>> {
        self.node.node().backend()
    }

    /// Returns a reference to the launched node handle.
    pub fn handle(&self) -> &LaunchedNode<P> {
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

        account.set_block_id(starknet::core::types::BlockId::Tag(BlockTag::PreConfirmed));

        account
    }

    /// Returns a HTTP client to the JSON-RPC server.
    pub fn rpc_http_client(&self) -> HttpClient {
        self.handle().rpc().http_client().expect("failed to get http client for the rpc server")
    }

    /// Returns a HTTP client to the JSON-RPC server.
    pub fn starknet_rpc_client(&self) -> katana_rpc_client::starknet::Client {
        let client = self.rpc_http_client();
        katana_rpc_client::starknet::Client::new_with_client(client)
    }
}

pub fn test_config() -> Config {
    let sequencing = SequencingConfig::default();
    let dev = DevConfig {
        total_accounts: 10,
        account_seed: String::new(),
        fee: false,
        account_validation: true,
        fixed_gas_prices: None,
    };

    let mut chain = dev::ChainSpec { id: ChainId::SEPOLIA, ..Default::default() };
    chain.genesis.sequencer_address = address!("0x1");

    let rpc = RpcConfig {
        port: 0,
        #[cfg(feature = "explorer")]
        explorer: true,
        addr: DEFAULT_RPC_ADDR,
        apis: RpcModulesList::all(),
        max_proof_keys: Some(100),
        max_event_page_size: Some(100),
        max_concurrent_estimate_fee_requests: None,
        ..Default::default()
    };

    Config { sequencing, rpc, dev, chain: ChainSpec::Dev(chain).into(), ..Default::default() }
}
