use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use katana_chain_spec::{dev, ChainSpec};
use katana_core::backend::Backend;
use katana_primitives::address;
use katana_primitives::chain::ChainId;
use katana_provider::{
    DbProviderFactory, ForkProviderFactory, ProviderFactory, ProviderRO, ProviderRW,
};
use katana_rpc_server::HttpClient;
use katana_sequencer_node::config::dev::DevConfig;
use katana_sequencer_node::config::grpc::{GrpcConfig, DEFAULT_GRPC_ADDR};
use katana_sequencer_node::config::rpc::{RpcConfig, RpcModulesList, DEFAULT_RPC_ADDR};
use katana_sequencer_node::config::sequencing::SequencingConfig;
use katana_sequencer_node::config::Config;
use katana_sequencer_node::{LaunchedNode, Node};
use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::BlockTag;
pub use starknet::core::types::StarknetError;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Url};
pub use starknet::providers::{Provider, ProviderError};
use starknet::signers::{LocalWallet, SigningKey};

/// Errors that can occur when migrating contracts to a test node.
#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("Failed to create temp directory: {0}")]
    TempDir(#[from] std::io::Error),
    #[error("Git clone failed: {0}")]
    GitClone(String),
    #[error("Scarb build failed: {0}")]
    ScarbBuild(String),
    #[error("Sozo migrate failed: {0}")]
    SozoMigrate(String),
    #[error("Missing genesis account private key")]
    MissingPrivateKey,
    #[error("Spawn blocking task failed: {0}")]
    SpawnBlocking(#[from] tokio::task::JoinError),
}

pub type ForkTestNode = TestNode<ForkProviderFactory>;

#[derive(Debug)]
pub struct TestNode<P = DbProviderFactory>
where
    P: ProviderFactory + Clone,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    node: LaunchedNode<P>,
    /// Temp directory holding a copied database snapshot. Cleaned up on drop.
    _db_temp_dir: Option<tempfile::TempDir>,
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
                .expect("failed to build node")
                .launch()
                .await
                .expect("failed to launch node"),
            _db_temp_dir: None,
        }
    }

    /// Creates a [`TestNode`] from a pre-existing database directory.
    ///
    /// Copies the database to a temp directory so each test gets its own mutable copy.
    /// The database is opened with [`SyncMode::UtterlyNoSync`] for test performance.
    pub async fn new_from_db(db_path: &Path) -> Self {
        Self::new_from_db_with_config(db_path, test_config()).await
    }

    /// Creates a [`TestNode`] from a pre-existing database directory with a custom config.
    ///
    /// Copies the database to a temp directory so each test gets its own mutable copy.
    /// The database is opened with [`SyncMode::UtterlyNoSync`] for test performance.
    pub async fn new_from_db_with_config(db_path: &Path, config: Config) -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");

        copy_db_dir(db_path, temp_dir.path()).expect("failed to copy database");

        let db = katana_db::Db::open_no_sync(temp_dir.path()).expect("failed to open database");
        let provider = DbProviderFactory::new(db.clone());

        Self {
            node: Node::build_with_provider(db, provider, config)
                .expect("failed to build node")
                .launch()
                .await
                .expect("failed to launch node"),
            _db_temp_dir: Some(temp_dir),
        }
    }

    /// Creates a [`TestNode`] with a pre-migrated `spawn-and-move` database snapshot.
    pub async fn new_with_spawn_and_move_db() -> Self {
        let db_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/db/spawn_and_move");
        Self::new_from_db(&db_path).await
    }

    /// Creates a [`TestNode`] with a pre-migrated `simple` database snapshot.
    pub async fn new_with_simple_db() -> Self {
        let db_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/db/simple");
        Self::new_from_db(&db_path).await
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
            _db_temp_dir: None,
        }
    }
}

impl<P> TestNode<P>
where
    P: ProviderFactory + Clone,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    /// Returns the address of the node's RPC server.
    pub fn rpc_addr(&self) -> &SocketAddr {
        self.node.rpc().addr()
    }

    pub fn backend(&self) -> &Arc<Backend<P>> {
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

    /// Returns the address of the node's gRPC server (if enabled).
    #[cfg(feature = "grpc")]
    pub fn grpc_addr(&self) -> Option<&SocketAddr> {
        self.node.grpc().map(|h| h.addr())
    }

    /// Migrates the `spawn-and-move` example contracts from the dojo repository.
    ///
    /// This method requires `git`, `asdf`, and `sozo` to be available in PATH.
    /// The scarb version is managed by asdf using the `.tool-versions` file
    /// in the dojo repository.
    pub async fn migrate_spawn_and_move(&self) -> Result<(), MigrateError> {
        self.migrate_example("spawn-and-move").await
    }

    /// Migrates the `simple` example contracts from the dojo repository.
    ///
    /// This method requires `git`, `asdf`, and `sozo` to be available in PATH.
    /// The scarb version is managed by asdf using the `.tool-versions` file
    /// in the dojo repository.
    pub async fn migrate_simple(&self) -> Result<(), MigrateError> {
        self.migrate_example("simple").await
    }

    /// Migrates contracts from a dojo example project.
    ///
    /// Clones the dojo repository, builds contracts with `scarb`, and deploys
    /// them with `sozo migrate`.
    ///
    /// This method requires `git`, `asdf`, and `sozo` to be available in PATH.
    /// The scarb version is managed by asdf using the `.tool-versions` file
    /// in the dojo repository.
    async fn migrate_example(&self, example: &str) -> Result<(), MigrateError> {
        let rpc_url = format!("http://{}", self.rpc_addr());

        let (address, account) = self
            .backend()
            .chain_spec
            .genesis()
            .accounts()
            .next()
            .expect("must have at least one genesis account");
        let private_key = account.private_key().ok_or(MigrateError::MissingPrivateKey)?;

        let address_hex = address.to_string();
        let private_key_hex = format!("{private_key:#x}");
        let example_path = format!("dojo/examples/{example}");

        tokio::task::spawn_blocking(move || {
            let temp_dir = tempfile::tempdir()?;

            // Clone dojo repository at v1.7.0
            run_git_clone(temp_dir.path())?;

            let project_dir = temp_dir.path().join(&example_path);

            // Build contracts using asdf to ensure correct scarb version
            run_scarb_build(&project_dir)?;

            // Deploy contracts to the katana node
            run_sozo_migrate(&project_dir, &rpc_url, &address_hex, &private_key_hex)?;

            Ok(())
        })
        .await?
    }
}

fn run_git_clone(temp_dir: &Path) -> Result<(), MigrateError> {
    let output = Command::new("git")
        .args(["clone", "--depth", "1", "--branch", "v1.7.0", "https://github.com/dojoengine/dojo"])
        .current_dir(temp_dir)
        .output()
        .map_err(|e| MigrateError::GitClone(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MigrateError::GitClone(stderr.to_string()));
    }
    Ok(())
}

fn run_scarb_build(project_dir: &Path) -> Result<(), MigrateError> {
    let output = Command::new("scarb")
        .arg("build")
        .current_dir(project_dir)
        .output()
        .map_err(|e| MigrateError::ScarbBuild(e.to_string()))?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}\n{stderr}");

        let lines: Vec<&str> = combined.lines().collect();
        let last_50: String =
            lines.iter().rev().take(50).rev().cloned().collect::<Vec<_>>().join("\n");

        return Err(MigrateError::ScarbBuild(last_50));
    }
    Ok(())
}

fn run_sozo_migrate(
    project_dir: &Path,
    rpc_url: &str,
    address: &str,
    private_key: &str,
) -> Result<(), MigrateError> {
    let status = Command::new("sozo")
        .args([
            "migrate",
            "--rpc-url",
            rpc_url,
            "--account-address",
            address,
            "--private-key",
            private_key,
        ])
        .current_dir(project_dir)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|e| MigrateError::SozoMigrate(e.to_string()))?;

    if !status.success() {
        return Err(MigrateError::SozoMigrate(format!(
            "sozo migrate exited with status: {status}"
        )));
    }
    Ok(())
}

/// Copies all files from `src` to `dst` (flat copy, no subdirectories).
///
/// The MDBX lock file (`mdbx.lck`) is intentionally skipped because it contains
/// platform-specific data (pthread mutexes, process IDs) that is not portable across
/// systems. MDBX creates a fresh lock file when the database is opened.
fn copy_db_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && entry.file_name() != "mdbx.lck" {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

pub fn test_config() -> Config {
    let sequencing = SequencingConfig::default();
    let dev = DevConfig { fee: false, account_validation: true, fixed_gas_prices: None };

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

    let grpc = Some(GrpcConfig {
        addr: DEFAULT_GRPC_ADDR,
        port: 0, // Use port 0 for auto-assignment
        timeout: Some(Duration::from_secs(30)),
    });

    Config { sequencing, rpc, dev, chain: ChainSpec::Dev(chain).into(), grpc, ..Default::default() }
}
