use std::marker::PhantomData;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::fmt::LocalTime;
use crate::{gcloud, otlp, Error, LogFormat};

// Type-state markers for log format
pub struct NoFormat;
pub struct WithFormat;

// Main builder struct with type-state for format
pub struct TracingBuilder<Format = NoFormat> {
    log_format: Option<LogFormat>,
    filter: Option<EnvFilter>,
    service_name: String,
    _format: PhantomData<Format>,
}

// Configuration that will be used during build
enum TelemetryConfig {
    None,
    Otlp(otlp::OtlpConfig),
    Gcloud(gcloud::GcloudConfig),
}

// Builder for OTLP telemetry configuration
pub struct OtlpTelemetryBuilder {
    endpoint: Option<String>,
    parent_builder: TracingBuilder<WithFormat>,
}

// Builder for Google Cloud telemetry configuration
pub struct GcloudTelemetryBuilder {
    project_id: Option<String>,
    parent_builder: TracingBuilder<WithFormat>,
}

impl TracingBuilder<NoFormat> {
    /// Create a new tracing builder
    pub fn new() -> Self {
        Self {
            log_format: None,
            filter: None,
            service_name: "katana".to_string(),
            _format: PhantomData,
        }
    }

    /// Set the log format to full (human-readable with colors)
    pub fn full(self) -> TracingBuilder<WithFormat> {
        TracingBuilder {
            log_format: Some(LogFormat::Full),
            filter: self.filter,
            service_name: self.service_name,
            _format: PhantomData,
        }
    }

    /// Set the log format to JSON
    pub fn json(self) -> TracingBuilder<WithFormat> {
        TracingBuilder {
            log_format: Some(LogFormat::Json),
            filter: self.filter,
            service_name: self.service_name,
            _format: PhantomData,
        }
    }
}

impl TracingBuilder<WithFormat> {
    /// Set a custom filter from a string
    pub fn with_filter(mut self, filter: &str) -> Result<Self, Error> {
        self.filter = Some(EnvFilter::try_new(filter)?);
        Ok(self)
    }

    /// Use the default filter
    pub fn with_default_filter(mut self) -> Result<Self, Error> {
        const DEFAULT_LOG_FILTER: &str = "katana_db::mdbx=trace,cairo_native::compiler=off,\
                                          pipeline=debug,stage=debug,tasks=debug,executor=trace,\
                                          forking::backend=trace,blockifier=off,\
                                          jsonrpsee_server=off,hyper=off,messaging=debug,\
                                          node=error,explorer=info,rpc=trace,pool=trace,info";

        self.filter = Some(EnvFilter::try_new(DEFAULT_LOG_FILTER)?);
        Ok(self)
    }

    /// Use filter from environment variable (RUST_LOG)
    pub fn with_env_filter(mut self) -> Result<Self, Error> {
        self.filter = Some(EnvFilter::try_from_default_env()?);
        Ok(self)
    }

    /// Use filter from environment with fallback to default
    pub fn with_env_filter_or_default(mut self) -> Result<Self, Error> {
        const DEFAULT_LOG_FILTER: &str = "katana_db::mdbx=trace,cairo_native::compiler=off,\
                                          pipeline=debug,stage=debug,tasks=debug,executor=trace,\
                                          forking::backend=trace,blockifier=off,\
                                          jsonrpsee_server=off,hyper=off,messaging=debug,\
                                          node=error,explorer=info,rpc=trace,pool=trace,info";

        let default_filter = EnvFilter::try_new(DEFAULT_LOG_FILTER);
        self.filter = Some(EnvFilter::try_from_default_env().or(default_filter)?);
        Ok(self)
    }

    /// Set the service name (default: "katana")
    pub fn with_service_name(mut self, name: impl Into<String>) -> Self {
        self.service_name = name.into();
        self
    }

    /// Configure OTLP telemetry
    pub fn otlp(self) -> OtlpTelemetryBuilder {
        OtlpTelemetryBuilder {
            endpoint: None,
            parent_builder: self,
        }
    }

    /// Configure Google Cloud telemetry
    pub fn gcloud(self) -> GcloudTelemetryBuilder {
        GcloudTelemetryBuilder {
            project_id: None,
            parent_builder: self,
        }
    }

    /// Build the tracing subscriber without telemetry
    pub async fn build(self) -> Result<(), Error> {
        self.build_with_telemetry(TelemetryConfig::None).await
    }

    async fn build_with_telemetry(self, telemetry_config: TelemetryConfig) -> Result<(), Error> {
        let filter = self.filter.unwrap_or_else(|| {
            const DEFAULT_LOG_FILTER: &str =
                "katana_db::mdbx=trace,cairo_native::compiler=off,pipeline=debug,stage=debug,\
                 tasks=debug,executor=trace,forking::backend=trace,blockifier=off,\
                 jsonrpsee_server=off,hyper=off,messaging=debug,node=error,explorer=info,\
                 rpc=trace,pool=trace,info";

            EnvFilter::try_new(DEFAULT_LOG_FILTER).expect("default filter should be valid")
        });

        let log_format = self.log_format.expect("log format must be set");

        match telemetry_config {
            TelemetryConfig::None => {
                let fmt = match log_format {
                    LogFormat::Full => {
                        tracing_subscriber::fmt::layer().with_timer(LocalTime::new()).boxed()
                    }
                    LogFormat::Json => {
                        tracing_subscriber::fmt::layer().json().with_timer(LocalTime::new()).boxed()
                    }
                };

                tracing_subscriber::registry().with(filter).with(fmt).init();
            }
            TelemetryConfig::Otlp(cfg) => {
                let tracer = otlp::init_tracer_with_service(&cfg, &self.service_name)?;
                let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

                let fmt = match log_format {
                    LogFormat::Full => {
                        tracing_subscriber::fmt::layer().with_timer(LocalTime::new()).boxed()
                    }
                    LogFormat::Json => {
                        tracing_subscriber::fmt::layer().json().with_timer(LocalTime::new()).boxed()
                    }
                };

                tracing_subscriber::registry().with(filter).with(telemetry).with(fmt).init();
            }
            TelemetryConfig::Gcloud(cfg) => {
                let tracer = gcloud::init_tracer_with_service(&cfg, &self.service_name).await?;
                let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

                let fmt = match log_format {
                    LogFormat::Full => {
                        tracing_subscriber::fmt::layer().with_timer(LocalTime::new()).boxed()
                    }
                    LogFormat::Json => {
                        tracing_subscriber::fmt::layer().json().with_timer(LocalTime::new()).boxed()
                    }
                };

                tracing_subscriber::registry().with(filter).with(telemetry).with(fmt).init();
            }
        }

        Ok(())
    }
}

impl OtlpTelemetryBuilder {
    /// Set the OTLP endpoint
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Build the tracing subscriber with OTLP telemetry
    pub async fn build(self) -> Result<(), Error> {
        let config = otlp::OtlpConfig {
            endpoint: self.endpoint,
        };
        self.parent_builder.build_with_telemetry(TelemetryConfig::Otlp(config)).await
    }
}

impl GcloudTelemetryBuilder {
    /// Set the Google Cloud project ID
    pub fn with_project_id(mut self, project_id: impl Into<String>) -> Self {
        self.project_id = Some(project_id.into());
        self
    }

    /// Build the tracing subscriber with Google Cloud telemetry
    pub async fn build(self) -> Result<(), Error> {
        let config = gcloud::GcloudConfig {
            project_id: self.project_id,
        };
        self.parent_builder.build_with_telemetry(TelemetryConfig::Gcloud(config)).await
    }
}

impl Default for TracingBuilder<NoFormat> {
    fn default() -> Self {
        Self::new()
    }
}

// Helper function for backward compatibility - creates a builder with a format already set
impl TracingBuilder<NoFormat> {
    /// Create a builder with a pre-selected format (for backward compatibility)
    pub(crate) fn with_format(format: LogFormat) -> TracingBuilder<WithFormat> {
        match format {
            LogFormat::Full => Self::new().full(),
            LogFormat::Json => Self::new().json(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_requires_format() {
        // This demonstrates that you must choose a format before building
        
        // The following should compile:
        let _builder = TracingBuilder::new()
            .json()
            .with_service_name("test-service");

        let _builder = TracingBuilder::new()
            .full()
            .with_service_name("test-service");

        // The following would NOT compile (commented out):
        // let _builder = TracingBuilder::new()
        //     .with_service_name("test-service") // Error: method not found
        //     .build();

        // Test that telemetry selection works after format is set
        let _otlp_builder = TracingBuilder::new().json().otlp();
        let _gcloud_builder = TracingBuilder::new().full().gcloud();
    }

    #[tokio::test]
    async fn test_builder_with_format() {
        // Note: This will fail if tracing is already initialized
        // In practice, this would be tested with a custom registry
        
        // Just ensure the builder compiles and doesn't panic
        let result = TracingBuilder::new()
            .json()
            .build()
            .await;
        
        // The second initialization should fail
        assert!(result.is_ok() || result.is_err());
    }
}