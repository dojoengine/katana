use katana_tracing::{LogFormat, TracingBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Example 1: Basic configuration without telemetry
    TracingBuilder::new()
        .with_log_format(LogFormat::Json)
        .with_default_filter()?
        .configure()
        .build()
        .await?;

    tracing::info!("Basic logging initialized");

    // Note: In a real application, you can only initialize tracing once.
    // The examples below show different configuration options.

    // Example 2: With OTLP telemetry
    // TracingBuilder::new()
    //     .with_log_format(LogFormat::Full)
    //     .with_env_filter_or_default()?
    //     .with_service_name("my-katana-node")
    //     .configure()
    //     .with_telemetry()
    //     .otlp()
    //     .with_endpoint("http://localhost:4317")
    //     .build()
    //     .await?;

    // Example 3: With Google Cloud telemetry
    // TracingBuilder::new()
    //     .with_log_format(LogFormat::Json)
    //     .with_filter("debug")?
    //     .with_service_name("katana-prod")
    //     .configure()
    //     .with_telemetry()
    //     .gcloud()
    //     .with_project_id("my-project")
    //     .build()
    //     .await?;

    // Example 4: Using environment filter
    // TracingBuilder::new()
    //     .with_env_filter()?  // Uses RUST_LOG environment variable
    //     .configure()
    //     .build()
    //     .await?;

    // Example 5: Custom filter with telemetry
    // TracingBuilder::new()
    //     .with_filter("katana=debug,tower=info")?
    //     .with_service_name("custom-service")
    //     .configure()
    //     .with_telemetry()
    //     .otlp()
    //     .build()  // Uses default OTLP endpoint
    //     .await?;

    Ok(())
}
