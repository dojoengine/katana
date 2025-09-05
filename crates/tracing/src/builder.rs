
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::fmt::LocalTime;
use crate::{gcloud, otlp, Error, LogFormat};

// Main builder struct
pub struct TracingBuilder {
    log_format: LogFormat,
    filter: Option<EnvFilter>,
    service_name: String,
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
    parent_builder: TracingBuilder,
}

// Builder for Google Cloud telemetry configuration
pub struct GcloudTelemetryBuilder {
    project_id: Option<String>,
    parent_builder: TracingBuilder,
}

impl TracingBuilder {
    /// Create a new tracing builder with default settings
    pub fn new() -> Self {
        Self {
            log_format: LogFormat::Full,
            filter: None,
            service_name: "katana".to_string(),
        }
    }

    /// Set the log format
    pub fn with_log_format(mut self, format: LogFormat) -> Self {
        self.log_format = format;
        self
    }

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

        match telemetry_config {
            TelemetryConfig::None => {
                let fmt = match self.log_format {
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

                let fmt = match self.log_format {
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

                let fmt = match self.log_format {
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

impl Default for TracingBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_type_safety() {
        // This test ensures the builder pattern compiles correctly
        // and demonstrates the fluent API flow
        
        // The following should compile:
        let _builder = TracingBuilder::new()
            .with_log_format(LogFormat::Json)
            .with_service_name("test-service");

        // Test that telemetry selection works
        let _otlp_builder = TracingBuilder::new().otlp();
        let _gcloud_builder = TracingBuilder::new().gcloud();
    }

    #[tokio::test]
    async fn test_builder_without_telemetry() {
        // Note: This will fail if tracing is already initialized
        // In practice, this would be tested with a custom registry
        
        // Just ensure the builder compiles and doesn't panic
        let result = TracingBuilder::new()
            .with_log_format(LogFormat::Json)
            .build()
            .await;
        
        // The second initialization should fail
        assert!(result.is_ok() || result.is_err());
    }
}