use katana_tracing::TracingBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Example 1: Basic configuration without telemetry
    TracingBuilder::new()
        .json()  // Must choose format first
        .with_default_filter()?
        .build()
        .await?;

    tracing::info!("Basic logging initialized");

    // Note: In a real application, you can only initialize tracing once.
    // The examples below show different configuration options.

    // Example 2: With OTLP telemetry
    // TracingBuilder::new()
    //     .full()  // Choose format first
    //     .with_env_filter_or_default()?
    //     .with_service_name("my-katana-node")
    //     .otlp()
    //     .with_endpoint("http://localhost:4317")
    //     .build()
    //     .await?;

    // Example 3: With Google Cloud telemetry
    // TracingBuilder::new()
    //     .json()  // Choose format first
    //     .with_filter("debug")?
    //     .with_service_name("katana-prod")
    //     .gcloud()
    //     .with_project_id("my-project")
    //     .build()
    //     .await?;

    // Example 4: Using environment filter
    // TracingBuilder::new()
    //     .full()  // Must choose format
    //     .with_env_filter()?  // Uses RUST_LOG environment variable
    //     .build()
    //     .await?;

    // Example 5: Custom filter with OTLP telemetry (using default endpoint)
    // TracingBuilder::new()
    //     .json()
    //     .with_filter("katana=debug,tower=info")?
    //     .with_service_name("custom-service")
    //     .otlp()
    //     .build()  // Uses default OTLP endpoint
    //     .await?;

    Ok(())
}
