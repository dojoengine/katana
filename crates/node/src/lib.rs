#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod full;

pub mod config;
pub mod exit;
#[cfg(feature = "vrf")]
pub mod sidecar;

use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use config::rpc::RpcModuleKind;
use config::Config;
use http::header::CONTENT_TYPE;
use http::Method;
use jsonrpsee::RpcModule;
use katana_chain_spec::{ChainSpec, SettlementLayer};
use katana_core::backend::Backend;
use katana_core::env::BlockContextGenerator;
use katana_core::service::block_producer::BlockProducer;
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{ExecutionFlags, ExecutorFactory};
use katana_gas_price_oracle::{FixedPriceOracle, GasPriceOracle};
use katana_gateway_server::{GatewayServer, GatewayServerHandle};
#[cfg(feature = "grpc")]
use katana_grpc::{GrpcServer, GrpcServerHandle};
use katana_metrics::exporters::prometheus::{Prometheus, PrometheusRecorder};
use katana_metrics::sys::DiskReporter;
use katana_metrics::{MetricsServer, MetricsServerHandle, Report};
use katana_pool::ordering::FiFo;
use katana_pool::TxPool;
use katana_primitives::block::{BlockHashOrNumber, GasPrices};
use katana_primitives::cairo::ShortString;
use katana_primitives::env::VersionedConstantsOverrides;
use katana_provider::{
    DbProviderFactory, ForkProviderFactory, ProviderFactory, ProviderRO, ProviderRW,
};
#[cfg(feature = "cartridge")]
use katana_rpc_api::cartridge::CartridgeApiServer;
use katana_rpc_api::dev::DevApiServer;
#[cfg(feature = "paymaster")]
use katana_rpc_api::paymaster::PaymasterApiServer;
use katana_rpc_api::starknet::{StarknetApiServer, StarknetTraceApiServer, StarknetWriteApiServer};
#[cfg(feature = "explorer")]
use katana_rpc_api::starknet_ext::StarknetApiExtServer;
#[cfg(feature = "tee")]
use katana_rpc_api::tee::TeeApiServer;
use katana_rpc_client::starknet::Client as StarknetClient;
#[cfg(feature = "cartridge")]
use katana_rpc_server::cartridge::{CartridgeApi, CartridgeConfig};
use katana_rpc_server::cors::Cors;
use katana_rpc_server::dev::DevApi;
#[cfg(feature = "paymaster")]
use katana_rpc_server::paymaster::PaymasterProxy;
#[cfg(feature = "cartridge")]
use katana_rpc_server::starknet::CartridgePaymasterConfig;
use katana_rpc_server::starknet::{StarknetApi, StarknetApiConfig};
#[cfg(feature = "tee")]
use katana_rpc_server::tee::TeeApi;
use katana_rpc_server::{RpcServer, RpcServerHandle};
use katana_rpc_types::GetBlockWithTxHashesResponse;
use katana_stage::Sequencing;
use katana_tasks::TaskManager;
use num_traits::ToPrimitive;
use tracing::info;

use crate::exit::NodeStoppedFuture;

/// A node instance.
///
/// The struct contains the handle to all the components of the node.
#[must_use = "Node does nothing unless launched."]
#[derive(Debug)]
pub struct Node<P>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    db: katana_db::Db,
    provider: P,
    config: Arc<Config>,
    pool: TxPool,
    rpc_server: RpcServer,
    #[cfg(feature = "grpc")]
    grpc_server: Option<GrpcServer>,
    task_manager: TaskManager,
    backend: Arc<Backend<BlockifierFactory, P>>,
    block_producer: BlockProducer<BlockifierFactory, P>,
    gateway_server: Option<GatewayServer<TxPool, P>>,
    metrics_server: Option<MetricsServer<Prometheus>>,
}

impl<P> Node<P>
where
    P: ProviderFactory + Clone,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    /// Build the node components from the given [`Config`].
    ///
    /// This returns a [`Node`] instance which can be launched with the all the necessary components
    /// configured.
    pub fn build_with_provider(db: katana_db::Db, provider: P, config: Config) -> Result<Node<P>> {
        if config.metrics.is_some() {
            // Metrics recorder must be initialized before calling any of the metrics macros, in
            // order for it to be registered.
            let _ = PrometheusRecorder::install("katana")?;
        }

        // -- build task manager

        let task_manager = TaskManager::current();
        let task_spawner = task_manager.task_spawner();

        // --- build executor factory

        // Create versioned constants overrides from config
        let overrides = Some(VersionedConstantsOverrides {
            invoke_tx_max_n_steps: Some(config.execution.invocation_max_steps),
            validate_max_n_steps: Some(config.execution.validation_max_steps),
            max_recursion_depth: Some(config.execution.max_recursion_depth),
        });

        let execution_flags = ExecutionFlags::new()
            .with_account_validation(config.dev.account_validation)
            .with_fee(config.dev.fee);

        let executor_factory = {
            #[allow(unused_mut)]
            let mut class_cache = ClassCache::builder();

            #[cfg(feature = "native")]
            {
                info!(enabled = config.execution.compile_native, "Cairo native compilation");
                class_cache = class_cache.compile_native(config.execution.compile_native);
            }

            let global_class_cache = class_cache.build_global()?;

            let factory = BlockifierFactory::new(
                overrides,
                execution_flags.clone(),
                config.sequencing.block_limits(),
                global_class_cache,
                config.chain.clone(),
            );

            Arc::new(factory)
        };

        // --- build l1 gas oracle

        // Check if the user specify a fixed gas price in the dev config.
        let gas_oracle = if let Some(prices) = &config.dev.fixed_gas_prices {
            GasPriceOracle::fixed(
                prices.l2_gas_prices.clone(),
                prices.l1_gas_prices.clone(),
                prices.l1_data_gas_prices.clone(),
            )
        } else if let Some(settlement) = config.chain.settlement() {
            match settlement {
                SettlementLayer::Starknet { rpc_url, .. } => {
                    GasPriceOracle::sampled_starknet(rpc_url.clone())
                }
                SettlementLayer::Ethereum { rpc_url, .. } => {
                    GasPriceOracle::sampled_ethereum(rpc_url.clone())
                }
                SettlementLayer::Sovereign { .. } => {
                    GasPriceOracle::Fixed(FixedPriceOracle::default())
                }
            }
        } else {
            GasPriceOracle::Fixed(FixedPriceOracle::default())
        };

        // Get cfg_env before moving executor_factory into Backend
        let versioned_constant_overrides = executor_factory.overrides().cloned();

        // --- build backend

        let block_context_generator = BlockContextGenerator::default().into();
        let backend = Arc::new(Backend {
            gas_oracle: gas_oracle.clone(),
            storage: provider.clone(),
            executor_factory,
            block_context_generator,
            chain_spec: config.chain.clone(),
        });

        backend.init_genesis(config.forking.is_some()).context("failed to initialize genesis")?;

        // --- build block producer

        let block_producer =
            if config.sequencing.block_time.is_some() || config.sequencing.no_mining {
                if let Some(interval) = config.sequencing.block_time {
                    BlockProducer::interval(Arc::clone(&backend), interval)
                } else {
                    BlockProducer::on_demand(Arc::clone(&backend))
                }
            } else {
                BlockProducer::instant(Arc::clone(&backend))
            };

        // --- build transaction pool

        let validator = block_producer.validator();
        let pool = TxPool::new(validator.clone(), FiFo::new());

        // --- build rpc server

        let mut rpc_modules = RpcModule::new(());

        let cors = Cors::new()
        .allow_origins(config.rpc.cors_origins.clone())
        // Allow `POST` when accessing the resource
        .allow_methods([Method::POST, Method::GET])
        .allow_headers([CONTENT_TYPE, "argent-client".parse().unwrap(), "argent-version".parse().unwrap()]);

        #[cfg(feature = "paymaster")]
        if let Some(cfg) = &config.paymaster {
            let proxy = PaymasterProxy::new(cfg.url.clone(), cfg.api_key.clone())?;
            rpc_modules.merge(proxy.into_rpc())?;
        };

        #[cfg(feature = "cartridge")]
        let cartridge_paymaster = if let Some(cfg) = &config.paymaster {
            if let Some(cartridge_api_cfg) = &cfg.cartridge_api {
                anyhow::ensure!(
                    config.rpc.apis.contains(&RpcModuleKind::Cartridge),
                    "Cartridge API should be enabled when paymaster is set"
                );

                #[cfg(feature = "vrf")]
                let vrf = if let Some(vrf) = &config.vrf {
                    let derived = crate::sidecar::derive_vrf_accounts(vrf, &config, &backend)?;
                    Some(katana_rpc_server::cartridge::CartridgeVrfConfig {
                        url: vrf.url.clone(),
                        account_address: derived.vrf_account_address,
                        account_private_key: derived.source_private_key,
                    })
                } else {
                    None
                };

                #[cfg(not(feature = "vrf"))]
                let vrf = None;

                let api = CartridgeApi::new(
                    backend.clone(),
                    block_producer.clone(),
                    pool.clone(),
                    task_spawner.clone(),
                    CartridgeConfig {
                        paymaster_url: cfg.url.clone(),
                        paymaster_api_key: cfg.api_key.clone(),
                        api_url: cartridge_api_cfg.cartridge_api_url.clone(),
                        paymaster_address: cartridge_api_cfg.controller_deployer_address,
                        paymaster_private_key: cartridge_api_cfg.controller_deployer_private_key,
                        vrf,
                    },
                )?;

                rpc_modules.merge(CartridgeApiServer::into_rpc(api))?;

                Some(CartridgePaymasterConfig {
                    cartridge_api_url: cartridge_api_cfg.cartridge_api_url.clone(),
                    paymaster_address: cartridge_api_cfg.controller_deployer_address,
                    paymaster_private_key: cartridge_api_cfg.controller_deployer_private_key,
                })
            } else {
                None
            }
        } else {
            None
        };

        // --- build starknet api

        let starknet_api_cfg = StarknetApiConfig {
            max_event_page_size: config.rpc.max_event_page_size,
            max_proof_keys: config.rpc.max_proof_keys,
            max_call_gas: config.rpc.max_call_gas,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
            simulation_flags: execution_flags,
            versioned_constant_overrides,
            #[cfg(feature = "cartridge")]
            paymaster: cartridge_paymaster,
        };

        let chain_spec = backend.chain_spec.clone();

        let starknet_api = StarknetApi::new(
            chain_spec.clone(),
            pool.clone(),
            task_spawner.clone(),
            block_producer.clone(),
            gas_oracle.clone(),
            starknet_api_cfg,
            provider.clone(),
        );

        if config.rpc.apis.contains(&RpcModuleKind::Starknet) {
            #[cfg(feature = "explorer")]
            if config.rpc.explorer {
                rpc_modules.merge(StarknetApiExtServer::into_rpc(starknet_api.clone()))?;
            }

            rpc_modules.merge(StarknetApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetWriteApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetTraceApiServer::into_rpc(starknet_api.clone()))?;
        }

        if config.rpc.apis.contains(&RpcModuleKind::Dev) {
            let api = DevApi::new(backend.clone(), block_producer.clone());
            rpc_modules.merge(DevApiServer::into_rpc(api))?;
        }

        // --- build tee api (if configured)
        #[cfg(feature = "tee")]
        if config.rpc.apis.contains(&RpcModuleKind::Tee) {
            if let Some(ref tee_config) = config.tee {
                use katana_tee::{TeeProvider, TeeProviderType};

                let tee_provider: Arc<dyn TeeProvider> = match tee_config.provider_type {
                    TeeProviderType::SevSnp => {
                        #[cfg(feature = "tee-snp")]
                        {
                            Arc::new(
                                katana_tee::SevSnpProvider::new()
                                    .context("Failed to initialize SEV-SNP provider")?,
                            )
                        }
                        #[cfg(not(feature = "tee-snp"))]
                        {
                            anyhow::bail!(
                                "SEV-SNP TEE provider requires the 'tee-snp' feature to be enabled"
                            );
                        }
                    }
                };

                let api = TeeApi::new(provider.clone(), tee_provider);
                rpc_modules.merge(TeeApiServer::into_rpc(api))?;

                info!(target: "node", provider = ?tee_config.provider_type, "TEE API enabled");
            }
        }

        #[allow(unused_mut)]
        let mut rpc_server =
            RpcServer::new().metrics(true).health_check(true).cors(cors).module(rpc_modules)?;

        #[cfg(feature = "explorer")]
        {
            rpc_server = rpc_server.explorer(config.rpc.explorer);
        }

        if let Some(timeout) = config.rpc.timeout {
            rpc_server = rpc_server.timeout(timeout);
        };

        if let Some(max_connections) = config.rpc.max_connections {
            rpc_server = rpc_server.max_connections(max_connections);
        }

        if let Some(max_request_body_size) = config.rpc.max_request_body_size {
            rpc_server = rpc_server.max_request_body_size(max_request_body_size);
        }

        if let Some(max_response_body_size) = config.rpc.max_response_body_size {
            rpc_server = rpc_server.max_response_body_size(max_response_body_size);
        }

        // --- build gRPC server (optional)

        #[cfg(feature = "grpc")]
        let grpc_server = if let Some(grpc_config) = &config.grpc {
            use katana_grpc::{
                StarknetServer, StarknetService, StarknetTraceServer, StarknetWriteServer,
            };

            let mut server = GrpcServer::new();

            if let Some(timeout) = grpc_config.timeout {
                server = server.timeout(timeout);
            }

            let svc = StarknetService::new(starknet_api.clone());

            server = server
                .service(StarknetServer::new(svc.clone()))
                .service(StarknetTraceServer::new(svc.clone()))
                .service(StarknetWriteServer::new(svc.clone()));

            Some(server)
        } else {
            None
        };

        // --- build feeder gateway server (optional)

        let gateway_server = if let Some(gw_config) = &config.gateway {
            let mut server = GatewayServer::new(starknet_api)
                .health_check(true)
                .metered(config.metrics.is_some());

            if let Some(timeout) = gw_config.timeout {
                server = server.timeout(timeout);
            }

            Some(server)
        } else {
            None
        };

        // --- build metrics server (optional)

        let metrics_server = if config.metrics.is_some() {
            let db_metrics = Box::new(db.clone()) as Box<dyn Report>;
            let disk_metrics = Box::new(DiskReporter::new(db.path())?) as Box<dyn Report>;
            let reports: Vec<Box<dyn Report>> = vec![db_metrics, disk_metrics];

            let exporter = PrometheusRecorder::current().expect("qed; should exist at this point");
            let server = MetricsServer::new(exporter).with_process_metrics().reports(reports);

            Some(server)
        } else {
            None
        };

        Ok(Node {
            db,
            provider,
            pool,
            backend,
            rpc_server,
            #[cfg(feature = "grpc")]
            grpc_server,
            gateway_server,
            block_producer,
            metrics_server,
            config: Arc::new(config),
            task_manager,
        })
    }
}

impl Node<DbProviderFactory> {
    pub fn build(config: Config) -> Result<Self> {
        let (provider, db) = if let Some(path) = &config.db.dir {
            info!(target: "node", path = %path.display(), "Initializing database.");
            let db = katana_db::Db::new(path)?;
            let factory = DbProviderFactory::new(db.clone());
            (factory, db)
        } else {
            info!(target: "node", "Initializing in-memory database.");
            let factory = DbProviderFactory::new_in_memory();
            let db = factory.db().clone();
            (factory, db)
        };

        Self::build_with_provider(db, provider, config)
    }
}

impl Node<ForkProviderFactory> {
    pub async fn build_forked(mut config: Config) -> Result<Self> {
        // NOTE: because the chain spec will be cloned for the BlockifierFactory (see below),
        // this mutation must be performed before the chain spec is cloned. Otherwise
        // this will panic.
        let chain_spec = Arc::get_mut(&mut config.chain).expect("get mut Arc");

        let cfg = config.forking.as_ref().unwrap();

        let ChainSpec::Dev(chain_spec) = chain_spec else {
            return Err(anyhow::anyhow!("Forking is only supported in dev mode for now"));
        };

        info!(target: "node", "Initializing in-memory database.");
        let db = katana_db::Db::in_memory()?;

        let client = StarknetClient::new(cfg.url.clone());
        let chain_id = client.chain_id().await.context("failed to fetch forked network id")?;

        // If the fork block number is not specified, we use the latest accepted block on the forked
        // network.
        let block_id = if let Some(id) = cfg.block {
            id
        } else {
            let res = client.block_number().await?;
            BlockHashOrNumber::Num(res.block_number)
        };

        // if the id is not in ASCII encoding, we display the chain id as is in hex.
        match ShortString::try_from(chain_id) {
            Ok(id) => {
                info!(chain = %id, block = %block_id, "Forking chain.");
            }

            Err(_) => {
                let id = format!("{chain_id:#x}");
                info!(chain = %id, block = %block_id, "Forking chain.");
            }
        };

        let block = client
            .get_block_with_tx_hashes(block_id.into())
            .await
            .context("failed to fetch forked block")?;

        let GetBlockWithTxHashesResponse::Block(forked_block) = block else {
            bail!("forking a pending block is not allowed")
        };

        let block_num = forked_block.block_number;
        let genesis_block_num = block_num + 1;

        chain_spec.id = chain_id.into();

        // adjust the genesis to match the forked block
        chain_spec.genesis.timestamp = forked_block.timestamp;
        chain_spec.genesis.number = genesis_block_num;
        chain_spec.genesis.state_root = Default::default();
        chain_spec.genesis.parent_hash = forked_block.parent_hash;
        chain_spec.genesis.sequencer_address = forked_block.sequencer_address;

        // TODO: remove gas price from genesis
        let eth_l1_gas_price =
            forked_block.l1_gas_price.price_in_wei.to_u128().expect("should fit in u128");
        let strk_l1_gas_price =
            forked_block.l1_gas_price.price_in_fri.to_u128().expect("should fit in u128");
        chain_spec.genesis.gas_prices =
            unsafe { GasPrices::new_unchecked(eth_l1_gas_price, strk_l1_gas_price) };

        // TODO: convert this to block number instead of BlockHashOrNumber so that it is easier to
        // check if the requested block is within the supported range or not.
        let provider_factory = ForkProviderFactory::new(db.clone(), block_num, client.clone());

        // update the genesis block with the forked block's data
        // we dont update the `l1_gas_price` bcs its already done when we set the `gas_prices` in
        // genesis. this flow is kinda flawed, we should probably refactor it out of the
        // genesis.
        let mut block = chain_spec.block();

        let eth_l1_data_gas_price =
            forked_block.l1_data_gas_price.price_in_wei.to_u128().expect("should fit in u128");
        let strk_l1_data_gas_price =
            forked_block.l1_data_gas_price.price_in_fri.to_u128().expect("should fit in u128");

        block.header.l1_data_gas_prices =
            unsafe { GasPrices::new_unchecked(eth_l1_data_gas_price, strk_l1_data_gas_price) };

        block.header.l1_da_mode = forked_block.l1_da_mode;

        Self::build_with_provider(db, provider_factory, config)
    }
}

impl<P> Node<P>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    /// Start the node.
    ///
    /// This method will start all the node process, running them until the node is stopped.
    pub async fn launch(self) -> Result<LaunchedNode<P>> {
        let chain = self.backend.chain_spec.id();
        info!(%chain, "Starting node.");

        // --- start the metrics server (if configured)

        let metrics_handle = if let Some(ref server) = self.metrics_server {
            // safe to unwrap here because metrics_server can only be Some if the metrics config
            // exists
            let cfg = self.config.metrics.as_ref().expect("qed; must exist");
            let addr = cfg.socket_addr();
            Some(server.start(addr)?)
        } else {
            None
        };

        let pool = self.pool.clone();
        let backend = self.backend.clone();
        let block_producer = self.block_producer.clone();

        // --- build and run sequencing task

        let sequencing = Sequencing::new(
            pool.clone(),
            backend.clone(),
            self.task_manager.task_spawner(),
            block_producer.clone(),
            self.config.messaging.clone(),
        );

        self.task_manager
            .task_spawner()
            .build_task()
            .graceful_shutdown()
            .name("Sequencing")
            .spawn(sequencing.into_future());

        // --- start the rpc server

        let rpc_handle = self.rpc_server.start(self.config.rpc.socket_addr()).await?;

        // --- start the feeder gateway server (if configured)

        let gateway_handle = match &self.gateway_server {
            Some(server) => {
                let config = self.config().gateway.as_ref().expect("qed; must exist");
                Some(server.start(config.socket_addr()).await?)
            }
            None => None,
        };

        // --- start the gRPC server (if configured)

        #[cfg(feature = "grpc")]
        let grpc_handle = if let Some(server) = &self.grpc_server {
            let config = self
                .config()
                .grpc
                .as_ref()
                .expect("qed; config must exist if grpc server is configured");

            Some(server.start(config.socket_addr()).await?)
        } else {
            None
        };

        // --- start the gas oracle worker task

        if let Some(worker) = self.backend.gas_oracle.run_worker() {
            self.task_manager
                .task_spawner()
                .build_task()
                .graceful_shutdown()
                .name("gas oracle")
                .spawn(worker);
        }

        info!(target: "node", "Gas price oracle worker started.");

        Ok(LaunchedNode {
            node: self,
            rpc: rpc_handle,
            gateway: gateway_handle,
            #[cfg(feature = "grpc")]
            grpc: grpc_handle,
            metrics: metrics_handle,
        })
    }

    /// Returns a reference to the node's database environment (if any).
    pub fn provider(&self) -> &P {
        &self.provider
    }

    pub fn backend(&self) -> &Arc<Backend<BlockifierFactory, P>> {
        &self.backend
    }

    /// Returns a reference to the node's transaction pool.
    pub fn pool(&self) -> &TxPool {
        &self.pool
    }

    /// Returns a reference to the node's JSON-RPC server.
    pub fn rpc(&self) -> &RpcServer {
        &self.rpc_server
    }

    /// Returns a reference to the node's database.
    pub fn db(&self) -> &katana_db::Db {
        &self.db
    }

    /// Returns a reference to the node's configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns a reference to the node's block producer.
    pub fn block_producer(&self) -> &BlockProducer<BlockifierFactory, P> {
        &self.block_producer
    }
}

/// A handle to the launched node.
#[derive(Debug)]
pub struct LaunchedNode<P>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    node: Node<P>,
    /// Handle to the rpc server.
    rpc: RpcServerHandle,
    /// Handle to the gateway server (if enabled).
    gateway: Option<GatewayServerHandle>,
    /// Handle to the gRPC server (if enabled).
    #[cfg(feature = "grpc")]
    grpc: Option<GrpcServerHandle>,
    /// Handle to the metrics server (if enabled).
    metrics: Option<MetricsServerHandle>,
}

impl<P> LaunchedNode<P>
where
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    /// Returns a reference to the [`Node`] handle.
    pub fn node(&self) -> &Node<P> {
        &self.node
    }

    /// Returns a reference to the rpc server handle.
    pub fn rpc(&self) -> &RpcServerHandle {
        &self.rpc
    }

    /// Returns a reference to the gateway server handle (if enabled).
    pub fn gateway(&self) -> Option<&GatewayServerHandle> {
        self.gateway.as_ref()
    }

    /// Returns a reference to the metrics server handle (if enabled).
    pub fn metrics(&self) -> Option<&MetricsServerHandle> {
        self.metrics.as_ref()
    }

    /// Returns a reference to the gRPC server handle (if enabled).
    #[cfg(feature = "grpc")]
    pub fn grpc(&self) -> Option<&GrpcServerHandle> {
        self.grpc.as_ref()
    }

    /// Stops the node.
    ///
    /// This will instruct the node to stop and wait until it has actually stop.
    pub async fn stop(self) -> Result<()> {
        // TODO: wait for the rpc server to stop instead of just stopping it.
        self.rpc.stop()?;

        // Stop feeder gateway server if it's running
        if let Some(handle) = self.gateway {
            handle.stop()?;
        }

        // Stop gRPC server if it's running
        #[cfg(feature = "grpc")]
        if let Some(handle) = self.grpc {
            handle.stop()?;
        }

        // Stop metrics server if it's running
        if let Some(mut handle) = self.metrics {
            handle.stop()?;
        }

        self.node.task_manager.shutdown().await;
        Ok(())
    }

    /// Returns a future which resolves only when the node has stopped.
    pub fn stopped(&self) -> NodeStoppedFuture<'_> {
        NodeStoppedFuture::new(self)
    }
}
