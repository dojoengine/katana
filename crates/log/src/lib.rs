use std::str::FromStr;

use opentelemetry::trace::TracerProvider;
use opentelemetry::KeyValue;
use opentelemetry_gcloud_trace::errors::GcloudTraceError;
use opentelemetry_gcloud_trace::{GcpCloudTraceExporterBuilder, SdkTracer};
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler, SdkTracerProvider};
use opentelemetry_sdk::{resource, Resource};
use tracing::subscriber::SetGlobalDefaultError;
use tracing::{Level, Subscriber};
use tracing_log::log::SetLoggerError;
use tracing_log::LogTracer;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{filter, EnvFilter, Registry};

mod fmt;

pub use fmt::LogFormat;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to initialize log tracer: {0}")]
    LogTracerInit(#[from] SetLoggerError),

    #[error("failed to parse environment filter: {0}")]
    EnvFilterParse(#[from] filter::ParseError),

    #[error("failed to set global dispatcher: {0}")]
    SetGlobalDefault(#[from] SetGlobalDefaultError),

    #[error("google cloud trace error: {0}")]
    GcloudTrace(#[from] GcloudTraceError),
}

pub async fn init(format: LogFormat, dev_log: bool) -> Result<(), Error> {
    const DEFAULT_LOG_FILTER: &str = "cairo_native::compiler=off,pipeline=debug,stage=debug,info,\
                                      tasks=debug,executor=trace,forking::backend=trace,\
                                      blockifier=off,jsonrpsee_server=off,hyper=off,\
                                      messaging=debug,node=error,explorer=info";

    let filter = if dev_log {
        format!("{DEFAULT_LOG_FILTER},server=debug")
    } else {
        DEFAULT_LOG_FILTER.to_string()
    };

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // LogTracer::init()?;
    init_tracing_subscriber().await;

    // // If the user has set the `RUST_LOG` environment variable, then we prioritize it.
    // // Otherwise, we use the default log filter.
    // // TODO: change env var to `KATANA_LOG`.
    // let filter = EnvFilter::try_from_default_env().or(EnvFilter::try_new(&filter))?;
    // let builder = tracing_subscriber::fmt::Subscriber::builder().with_env_filter(filter);

    // let subscriber: Box<dyn Subscriber + Send + Sync> = match format {
    //     LogFormat::Full => {
    //         let a = builder.finish();
    //         Box::new(a)
    //     }
    //     LogFormat::Json => Box::new(builder.json().finish()),
    // };

    // // tracing_subscriber::registry().with(layer).try_init();
    // Ok(tracing::subscriber::set_global_default(subscriber)?)

    Ok(())
}

async fn init_tracing_subscriber() {
    let gcp_tracer = init_gcp_tracer("katana").await.unwrap();
    let telemetry = tracing_opentelemetry::layer().with_tracer(gcp_tracer);
    let filter = tracing_subscriber::filter::LevelFilter::from_level(Level::INFO);

    tracing_subscriber::registry()
        // The global level filter prevents the exporter network stack
        // from reentering the globally installed OpenTelemetryLayer with
        // its own spans while exporting, as the libraries should not use
        // tracing levels below DEBUG. If the OpenTelemetry layer needs to
        // trace spans and events with higher verbosity levels, consider using
        // per-layer filtering to target the telemetry layer specifically,
        // e.g. by target matching.
        .with(filter)
        .with(telemetry)
        .with(tracing_subscriber::fmt::layer())
        .init();
}

/// Initialize Google Cloud Trace exporter
async fn init_gcp_tracer(service_name: &str) -> anyhow::Result<SdkTracer> {
    let resource = Resource::builder().with_service_name(service_name.to_string()).build();

    // Default will attempt to find project ID from environment variables in the following order:
    // - GCP_PROJECT
    // - PROJECT_ID
    // - GCP_PROJECT_ID

    // default it is using batch span processor
    let trace_exporter =
        GcpCloudTraceExporterBuilder::for_default_project_id().await?.with_resource(resource);

    // const ENV_KEY: &str = "GOOGLE_APPLICATION_CREDENTIALS"; set google application credentials
    let tracer_provider = trace_exporter.create_provider().await?;
    let tracer = trace_exporter.install(&tracer_provider).await?;

    // // Create a tracing layer with the configured tracer
    // let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
    // Use the tracing subscriber `Registry`, or any other subscriber
    // that impls `LookupSpan`
    // let subscriber = Registry::default().with(telemetry);

    // tracing::subscriber::set_global_default(subscriber).unwrap();
    opentelemetry::global::set_tracer_provider(tracer_provider.clone());

    // // Trace executed code
    // tracing::subscriber::with_default(subscriber, || {
    //     // Spans will be sent to the configured OpenTelemetry exporter
    //     let root = span!(tracing::Level::TRACE, "my_app", work_units = 2);
    //     let _enter = root.enter();

    //     let child_span = span!(
    //         tracing::Level::TRACE,
    //         "my_child",
    //         work_units = 2,
    //         "http.client_ip" = "42.42.42.42"
    //     );
    //     child_span.in_scope(|| {
    //         info!(
    //             "Do printing, nothing more here. Please check your Google Cloud Trace dashboard."
    //         );
    //     });

    //     error!("This event will be logged in the root span.");
    // });

    // hold this somewhere
    // tracer_provider.shutdown()?;

    Ok(tracer)
}

// fn init_otlp_tracer(service_name: &str, endpoint: &str, _sample_rate: f64) -> anyhow::Result<()>
// {     use opentelemetry_otlp::WithExportConfig;
//     use opentelemetry_sdk::Resource;
//     use std::time::Duration;

//     let resource = Resource::builder().with_service_name(service_name).build();

//     let exporter = opentelemetry_otlp::new_exporter()
//         .tonic()
//         .with_endpoint(endpoint)
//         .with_timeout(Duration::from_secs(3));

//     let provider = SdkTracerProvider::builder()
//     .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
//         // .with_batch_exporter(exporter)
//         .with_id_generator(RandomIdGenerator::default())
//         .build();

//     let provider = TracerProvider::builder()
//         .with_config(
//             opentelemetry_sdk::trace::config()
//                 .with_resource(resource)
//                 .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn),
//         )
//         .build();

//     opentelemetry::global::set_tracer_provider(provider);

//     Ok(())
// }
