#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod config;
pub mod exit;

use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use config::rpc::RpcModuleKind;
use config::Config;
use http::header::CONTENT_TYPE;
use http::Method;
use jsonrpsee::core::middleware::layer::Either;
use jsonrpsee::RpcModule;
use katana_chain_spec::{settlement_check, ChainSpec, SettlementLayer};
use katana_core::backend::Backend;
use katana_core::env::BlockContextGenerator;
use katana_core::service::block_producer::{BlockProducer, MinedBlockOutcome};
use katana_db::migration;
use katana_executor::blockifier::cache::ClassCache;
use katana_executor::blockifier::BlockifierFactory;
use katana_executor::{ExecutionFlags, ExecutorFactory};
use katana_gas_price_oracle::{FixedPriceOracle, GasPriceOracle};
use katana_gateway_server::{GatewayServer, GatewayServerHandle};
#[cfg(feature = "grpc")]
use katana_grpc::{GrpcServer, GrpcServerHandle};
use katana_messaging::service::{MessagingService, MessagingServiceHandle};
use katana_metrics::exporters::prometheus::{Prometheus, PrometheusRecorder};
use katana_metrics::sys::DiskReporter;
use katana_metrics::{MetricsServer, MetricsServerHandle, Report};
use katana_pool::ordering::FiFo;
use katana_pool::TxPool;
use katana_primitives::block::{BlockHashOrNumber, GasPrices};
use katana_primitives::cairo::ShortString;
use katana_primitives::env::VersionedConstantsOverrides;
use katana_primitives::Felt;
use katana_provider::api::messaging::{
    MessagingCheckpointProvider, MessagingL1ToL2IndexProvider, MessagingL1ToL2IndexWriter,
};
use katana_provider::{
    DbProviderFactory, ForkProviderFactory, MutableProvider, ProviderFactory, ProviderRO,
    ProviderRW,
};
use katana_rpc_api::cartridge::CartridgeApiServer;
use katana_rpc_api::dev::DevApiServer;
use katana_rpc_api::katana::KatanaApiServer;
use katana_rpc_api::node::NodeApiServer;
use katana_rpc_api::paymaster::PaymasterApiServer;
use katana_rpc_api::starknet::{StarknetApiServer, StarknetSubscriptionApiServer};
#[cfg(feature = "explorer")]
use katana_rpc_api::starknet_ext::StarknetApiExtServer;
#[cfg(any(feature = "tee-snp", feature = "tee-mock"))]
use katana_rpc_api::tee::TeeApiServer;
use katana_rpc_server::cartridge::{CartridgeApi, CartridgeConfig};
use katana_rpc_server::dev::DevApi;
use katana_rpc_server::middleware::cartridge::{ControllerDeploymentLayer, VrfLayer};
use katana_rpc_server::middleware::cors::Cors;
use katana_rpc_server::middleware::logger::RpcLoggerLayer;
use katana_rpc_server::middleware::metrics::RpcServerMetricsLayer;
use katana_rpc_server::node::NodeApi;
use katana_rpc_server::paymaster::PaymasterProxy;
use katana_rpc_server::starknet::{RpcCache, StarknetApi, StarknetApiConfig};
#[cfg(any(feature = "tee-snp", feature = "tee-mock"))]
use katana_rpc_server::tee::TeeApi;
use katana_rpc_server::{RpcServer, RpcServerHandle, RpcServiceBuilder};
use katana_rpc_types::node::NodeInfo;
use katana_rpc_types::GetBlockWithTxHashesResponse;
use katana_settlement::{
    ChainConfig, ProverConfig, SettlementPipeline, SettlementService, SettlementServiceHandle,
    StarknetSettlementChain, TeeBackend, TeeProver,
};
use katana_stage::Sequencing;
use katana_starknet::rpc::StarknetRpcClient as StarknetClient;
use katana_tasks::TaskManager;
use num_traits::ToPrimitive;
use starknet::signers::SigningKey;
use tokio::sync::broadcast;
use tower::layer::util::{Identity, Stack};
use tracing::info;

use crate::exit::NodeStoppedFuture;

/// The concrete type of the RPC middleware stack used by the node.
type NodeRpcMiddleware<PF> = Stack<
    Either<VrfLayer, Identity>,
    Stack<
        Either<ControllerDeploymentLayer<TxPool, BlockProducer<PF>, PF>, Identity>,
        Stack<RpcLoggerLayer, Stack<RpcServerMetricsLayer, Identity>>,
    >,
>;

pub type NodeRpcServer<PF> = RpcServer<NodeRpcMiddleware<PF>>;

/// A node instance.
///
/// The struct contains the handle to all the components of the node.
#[must_use = "Node does nothing unless launched."]
#[derive(Debug)]
pub struct Node<P>
where
    P: ProviderFactory + Clone,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    db: katana_db::Db,
    provider: P,
    config: Arc<Config>,
    pool: TxPool,
    rpc_server: NodeRpcServer<P>,
    #[cfg(feature = "grpc")]
    grpc_server: Option<GrpcServer>,
    task_manager: TaskManager,
    backend: Arc<Backend<P>>,
    block_producer: BlockProducer<P>,
    block_notify: broadcast::Sender<MinedBlockOutcome>,
    gateway_server: Option<GatewayServer<TxPool, BlockProducer<P>, P>>,
    metrics_server: Option<MetricsServer<Prometheus>>,
    messaging_service: Option<MessagingService<P>>,
    settlement_service: Option<SettlementService<P, MinedBlockOutcome>>,
}

impl<P> Node<P>
where
    P: ProviderFactory + Clone + Send + Sync + 'static,
    <P as ProviderFactory>::Provider: ProviderRO + MessagingL1ToL2IndexProvider,
    <P as ProviderFactory>::ProviderMut:
        ProviderRW + MessagingCheckpointProvider + MessagingL1ToL2IndexWriter + MutableProvider,
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

        let is_l3 = match config.chain.as_ref() {
            ChainSpec::Dev(_) => false,
            ChainSpec::FullNode(_) => false,
            ChainSpec::Rollup(cs) => matches!(cs.settlement, SettlementLayer::Starknet { .. }),
        };

        // Create versioned constants overrides from config
        let overrides = Some(VersionedConstantsOverrides {
            invoke_tx_max_n_steps: Some(config.execution.invocation_max_steps),
            validate_max_n_steps: Some(config.execution.validation_max_steps),
            max_recursion_depth: Some(config.execution.max_recursion_depth),
            is_l3,
        });

        let execution_flags = ExecutionFlags::new()
            .with_account_validation(config.dev.account_validation)
            .with_fee(config.dev.fee);

        let class_cache = {
            #[allow(unused_mut)]
            let mut builder = ClassCache::builder();

            #[cfg(feature = "native")]
            {
                info!(enabled = config.execution.compile_native, "Cairo native");
                builder = builder.compile_native(config.execution.compile_native);
            }

            builder.build()?
        };

        let executor_factory = {
            let factory = BlockifierFactory::new(
                overrides,
                execution_flags.clone(),
                config.sequencing.block_limits(),
                class_cache.clone(),
                config.chain.clone(),
            );

            Arc::new(factory) as Arc<dyn ExecutorFactory>
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
            no_state_trie: config.sequencing.no_state_trie,
        });

        let skip_dev_genesis =
            config.forking.as_ref().is_some_and(|forking| !forking.init_dev_genesis);

        backend.init_genesis(skip_dev_genesis).context("failed to initialize genesis")?;

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

        info!(target: "rpc", apis = %config.rpc.apis, "Enabled JSON-RPC APIs.");

        let mut rpc_modules = RpcModule::new(());

        // Allow `POST` when accessing the resource
        let cors = Cors::new()
            .allow_origins(config.rpc.cors_origins.clone())
            .allow_methods([Method::POST, Method::GET])
            .allow_headers([
                CONTENT_TYPE,
                "argent-client".parse().unwrap(),
                "argent-version".parse().unwrap(),
            ]);

        if let Some(cfg) = &config.paymaster {
            let proxy = PaymasterProxy::new(cfg.url.clone(), cfg.api_key.clone())?;
            rpc_modules.merge(proxy.into_rpc())?;
        };

        // --- build block notification channel

        let (block_notify, _) = broadcast::channel::<MinedBlockOutcome>(64);

        // --- build starknet api

        let starknet_api_cfg = StarknetApiConfig {
            max_event_page_size: config.rpc.max_event_page_size,
            max_proof_keys: config.rpc.max_proof_keys,
            max_call_gas: config.rpc.max_call_gas,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
            simulation_flags: execution_flags,
            versioned_constant_overrides,
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
            RpcCache::new(),
            class_cache.clone(),
            block_notify.clone(),
        );

        if config.rpc.apis.contains(&RpcModuleKind::Starknet) {
            rpc_modules.merge(StarknetApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetSubscriptionApiServer::into_rpc(starknet_api.clone()))?;

            #[cfg(feature = "explorer")]
            if config.rpc.explorer {
                rpc_modules.merge(StarknetApiExtServer::into_rpc(starknet_api.clone()))?;
            }
        }

        if config.rpc.apis.contains(&RpcModuleKind::Starknet) {
            rpc_modules.merge(KatanaApiServer::into_rpc(starknet_api.clone()))?;
        }

        if config.rpc.apis.contains(&RpcModuleKind::Dev) {
            let api = DevApi::new(backend.clone(), block_producer.clone(), pool.clone());
            rpc_modules.merge(DevApiServer::into_rpc(api))?;
        }

        if config.rpc.apis.contains(&RpcModuleKind::TxPool) {
            let api = katana_rpc_server::txpool::TxPoolApi::new(pool.clone());
            rpc_modules.merge(katana_rpc_api::txpool::TxPoolApiServer::into_rpc(api))?;
        }

        if config.rpc.apis.contains(&RpcModuleKind::Node) {
            let info = NodeInfo::from_parts(&config.build_info, backend.chain_spec.as_ref());
            rpc_modules.merge(NodeApiServer::into_rpc(NodeApi::new(info)))?;
        }

        // --- build cartridge api (plus middleware)

        let (controller_deployment_layer, vrf_layer) = if let Some(cfg) = &config.paymaster {
            if let Some(cartridge_api_cfg) = &cfg.cartridge_api {
                use anyhow::ensure;

                ensure!(
                    config.rpc.apis.contains(&RpcModuleKind::Cartridge),
                    "Cartridge API should be enabled when paymaster is set"
                );

                let cartridge_api_client =
                    cartridge::CartridgeApiClient::new(cartridge_api_cfg.cartridge_api_url.clone());

                let cartridge_api_config = CartridgeConfig {
                    paymaster_url: cfg.url.clone(),
                    paymaster_api_key: cfg.api_key.clone(),
                    api_url: cartridge_api_cfg.cartridge_api_url.clone(),
                };

                let cartrige_api = CartridgeApi::new(
                    backend.clone(),
                    block_producer.clone(),
                    pool.clone(),
                    task_spawner.clone(),
                    cartridge_api_config,
                )?;

                rpc_modules.merge(CartridgeApiServer::into_rpc(cartrige_api))?;

                let controller_deployment_layer = ControllerDeploymentLayer::new(
                    starknet_api.clone(),
                    cartridge_api_client,
                    cartridge_api_cfg.controller_deployer_address,
                    SigningKey::from_secret_scalar(
                        cartridge_api_cfg.controller_deployer_private_key,
                    ),
                );

                let vrf_layer = if let Some(vrf) = &cartridge_api_cfg.vrf {
                    use katana_rpc_server::cartridge::{VrfService, VrfServiceConfig};
                    use url::Url;

                    let rpc_url = Url::parse(&format!("http://{}", config.rpc.socket_addr()))
                        .expect("valid rpc url");

                    let vrf_service = VrfService::new(VrfServiceConfig {
                        rpc_url,
                        service_url: vrf.url.clone(),
                        vrf_contract: vrf.vrf_account,
                    });

                    Some(VrfLayer::new(vrf_service, backend.chain_spec.id()))
                } else {
                    None
                };

                (Some(controller_deployment_layer), vrf_layer)
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        // --- build tee attester + api (if configured)

        // The attester is shared between the TEE RPC API and the settlement service so both
        // attest through the same hardware (or mock) instance.
        let attester: Option<Arc<dyn katana_tee::Attester>> = if let Some(ref tee_config) =
            config.tee
        {
            #[cfg(not(any(feature = "tee-snp", feature = "tee-mock")))]
            {
                let _ = tee_config;
                anyhow::bail!(
                    "TEE configuration provided but no TEE attester feature is compiled in \
                     (enable 'tee-snp' or 'tee-mock')"
                );
            }

            #[cfg(any(feature = "tee-snp", feature = "tee-mock"))]
            {
                use katana_tee::{Attester, AttesterKind};

                let attester: Arc<dyn Attester> = match tee_config.attester {
                    AttesterKind::SevSnp => {
                        #[cfg(feature = "tee-snp")]
                        {
                            Arc::new(
                                katana_tee::SevSnpAttester::new()
                                    .context("Failed to initialize SEV-SNP attester")?,
                            )
                        }
                        #[cfg(not(feature = "tee-snp"))]
                        {
                            anyhow::bail!(
                                "SEV-SNP TEE attester requires the 'tee-snp' feature to be enabled"
                            );
                        }
                    }
                    #[cfg(feature = "tee-mock")]
                    AttesterKind::Mock => Arc::new(katana_tee::MockAttester::new()),
                    // The `Mock` variant on `AttesterKind` is gated on
                    // `feature = "mock"` in `katana-tee`, but cargo features
                    // unify across the workspace — so the variant can be visible
                    // here even when this crate's own `tee-mock` feature is off.
                    #[allow(unreachable_patterns)]
                    _ => anyhow::bail!("Mock TEE attester requires the 'tee-mock' feature"),
                };

                Some(attester)
            }
        } else {
            None
        };

        #[cfg(any(feature = "tee-snp", feature = "tee-mock"))]
        if let (Some(tee_config), Some(attester)) = (&config.tee, &attester) {
            let api = TeeApi::new(
                provider.clone(),
                attester.clone(),
                tee_config.fork_block_number,
                &backend.chain_spec,
            );
            rpc_modules.merge(TeeApiServer::into_rpc(api))?;

            info!(target: "node", attester = ?tee_config.attester, "TEE API enabled");
        }

        // --- build rpc middleware

        let rpc_middleware = RpcServiceBuilder::new()
            .layer(RpcServerMetricsLayer::new(&rpc_modules))
            .layer(RpcLoggerLayer::new());

        let rpc_middleware =
            rpc_middleware.option_layer(controller_deployment_layer).option_layer(vrf_layer);

        #[allow(unused_mut)]
        let mut rpc_server = RpcServer::new()
            .cors(cors)
            .metrics(true)
            .health_check(true)
            .rpc_middleware(rpc_middleware)
            .module(rpc_modules)?;

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

        // --- build messaging server

        let messaging_service = config.messaging.as_ref().map(|cfg| {
            MessagingService::new(
                backend.chain_spec.id(),
                pool.clone(),
                provider.clone(),
                cfg.settlement.clone(),
            )
            .interval(cfg.interval)
            .from_block(cfg.from_block)
            .confirmation_depth(cfg.confirmation_depth)
        });

        // --- build settlement service (if configured)

        let settlement_service = match &config.settlement {
            Some(cfg) => {
                // The two axes of the settlement setup — which chain to settle to and how to
                // prove state transitions — are selected here; the pipeline pairing statically
                // checks the proving backend's payload matches the chain's core contract. Only
                // the Starknet (Piltover) + TEE combination exists today.
                let ChainConfig::Starknet {
                    id,
                    rpc_url,
                    core_contract,
                    account_address,
                    account_private_key,
                } = &cfg.chain;

                let chain = StarknetSettlementChain::new(
                    rpc_url.clone(),
                    *id,
                    *core_contract,
                    *account_address,
                    *account_private_key,
                );

                // Each proving system declares its own requirements here.
                let service = match &cfg.prover {
                    ProverConfig::Tee { tee_registry, prover_key } => {
                        let Some(attester) = &attester else {
                            anyhow::bail!(
                                "TEE settlement is configured but TEE attestation is not; enable \
                                 a TEE attester with --tee"
                            );
                        };
                        let tee_config =
                            config.tee.as_ref().expect("qed; attester implies tee config");

                        // Fork mode attests with the sharding report schema, which carries no
                        // message data — Piltover's appchain `TeeInput` path can't validate it.
                        if tee_config.fork_block_number.is_some() {
                            anyhow::bail!("settlement of forked chains is not supported");
                        }

                        // A mock attester's quotes can never yield a real SP1 proof (the SP1
                        // program verifies the AMD signature), so the prover is forced by the
                        // attester kind.
                        let prover =
                            if matches!(tee_config.attester, katana_tee::AttesterKind::SevSnp) {
                                let prover_key = prover_key.clone().context(
                                    "SP1 prover key is required for SEV-SNP attestation proving",
                                )?;
                                TeeProver::Sp1 {
                                    settlement_rpc: rpc_url.clone(),
                                    tee_registry: *tee_registry,
                                    prover_key,
                                }
                            } else {
                                TeeProver::Mock
                            };

                        let tee_backend = TeeBackend::new(
                            provider.clone(),
                            attester.clone(),
                            &backend.chain_spec,
                            prover,
                        );

                        SettlementService::new(
                            provider.clone(),
                            SettlementPipeline::new(tee_backend, chain),
                            block_notify.clone(),
                            cfg.clone(),
                        )
                    }
                };

                Some(service)
            }

            None => None,
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
            block_notify,
            metrics_server,
            messaging_service,
            settlement_service,
            task_manager,
            config: Arc::new(config),
        })
    }
}

impl Node<DbProviderFactory> {
    pub fn build(config: Config) -> Result<Self> {
        let (provider, db) = if let Some(path) = &config.db.dir {
            info!(target: "node", path = %path.display(), "Initializing database.");
            let db = katana_db::Db::new(path)?;

            if config.db.migrate {
                migration::Migration::new_v9(&db).run()?;
            }

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

        // Store fork block number in TEE config so report_data includes it
        if let Some(ref mut tee_config) = config.tee {
            tee_config.fork_block_number = Some(block_num);
        }

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
    P: ProviderFactory + Clone + Send + Sync + 'static,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut:
        ProviderRW + MessagingCheckpointProvider + MessagingL1ToL2IndexWriter + MutableProvider,
{
    /// Start the node.
    ///
    /// This method will start all the node process, running them until the node is stopped.
    pub async fn launch(self) -> Result<LaunchedNode<P>> {
        let chain = self.backend.chain_spec.id();
        info!(%chain, "Starting node.");

        // --- validate the on-chain settlement contract for rollup chains

        if let ChainSpec::Rollup(spec) = self.config.chain.as_ref() {
            if let SettlementLayer::Starknet { rpc_url, core_contract, proof_kind, .. } =
                &spec.settlement
            {
                let provider =
                    settlement_check::SettlementChainProvider::new(rpc_url.clone(), Felt::ZERO);

                settlement_check::validate_starknet_settlement(
                    spec.id.id(),
                    spec.fee_contracts.strk.into(),
                    *core_contract,
                    &provider,
                    *proof_kind,
                )
                .await
                .context("settlement core contract validation failed")?;
            }
        }

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
        let block_producer = self.block_producer.clone();

        // --- build and run sequencing task

        let sequencing = Sequencing::new(
            pool.clone(),
            self.task_manager.task_spawner(),
            block_producer.clone(),
            self.block_notify.clone(),
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

        // --- start the messaging server (skipped when messaging is disabled)

        let messaging_handle = self.messaging_service.as_ref().map(|s| s.start()).transpose()?;

        // --- start the settlement service (skipped when settlement is not configured)
        //
        // Launched after the settlement contract validation above, so the service only ever
        // trusts a cursor read from a contract with the expected program info and config hash.

        let settlement_handle = match &self.settlement_service {
            Some(service) => Some(service.start().await?),
            None => None,
        };

        Ok(LaunchedNode {
            node: self,
            rpc: rpc_handle,
            gateway: gateway_handle,
            #[cfg(feature = "grpc")]
            grpc: grpc_handle,
            metrics: metrics_handle,
            messaging: messaging_handle,
            settlement: settlement_handle,
        })
    }
}

impl<P> Node<P>
where
    P: ProviderFactory + Clone,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    /// Returns a reference to the node's database environment (if any).
    pub fn provider(&self) -> &P {
        &self.provider
    }

    pub fn backend(&self) -> &Arc<Backend<P>> {
        &self.backend
    }

    /// Returns a reference to the node's transaction pool.
    pub fn pool(&self) -> &TxPool {
        &self.pool
    }

    /// Returns a reference to the node's JSON-RPC server.
    pub fn rpc(&self) -> &NodeRpcServer<P> {
        &self.rpc_server
    }

    /// Returns a reference to the node's messaging server, if messaging is enabled.
    pub fn messaging_server(&self) -> Option<&MessagingService<P>> {
        self.messaging_service.as_ref()
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
    pub fn block_producer(&self) -> &BlockProducer<P> {
        &self.block_producer
    }
}

/// A handle to the launched node.
#[derive(Debug)]
pub struct LaunchedNode<P>
where
    P: ProviderFactory + Clone,
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
    /// Handle to the messaging server (if running).
    messaging: Option<MessagingServiceHandle>,
    /// Handle to the settlement service (if running).
    settlement: Option<SettlementServiceHandle>,
}

impl<P> LaunchedNode<P>
where
    P: ProviderFactory + Clone,
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

    /// Returns a reference to the messaging server handle (if running).
    pub fn messaging(&self) -> Option<&MessagingServiceHandle> {
        self.messaging.as_ref()
    }

    /// Returns a reference to the settlement service handle (if running).
    pub fn settlement(&self) -> Option<&SettlementServiceHandle> {
        self.settlement.as_ref()
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

        // Stop messaging server if running. Signal and then await so the final
        // checkpoint write completes before we tear down the provider.
        if let Some(mut handle) = self.messaging {
            handle.stop();
            handle.stopped().await;
        }

        // Stop settlement service if running. Signal and then await so an in-flight
        // `update_state` submission is not torn down mid-watch.
        if let Some(mut handle) = self.settlement {
            handle.stop();
            handle.stopped().await;
        }

        self.node.task_manager.shutdown().await;
        Ok(())
    }

    /// Returns a future which resolves only when the node has stopped.
    pub fn stopped(&self) -> NodeStoppedFuture<'_> {
        NodeStoppedFuture::new(self)
    }
}
