# Distributed Tracing Example for Katana RPC

This example demonstrates how to enable distributed tracing in Katana with Google Cloud Trace support.

## Prerequisites

1. Google Cloud Project with Cloud Trace API enabled
2. Google Cloud credentials configured (via `gcloud auth application-default login` or service account key)

## Basic Usage

### Enable tracing with Google Cloud Trace

```bash
katana --tracing \
       --tracing-exporter google-cloud-trace \
       --tracing-gcp-project-id YOUR_PROJECT_ID
```

### Enable tracing with OTLP (OpenTelemetry Protocol)

```bash
katana --tracing \
       --tracing-exporter otlp \
       --tracing-otlp-endpoint http://localhost:4317
```

### Customize tracing configuration

```bash
katana --tracing \
       --tracing-service-name my-katana-node \
       --tracing-exporter google-cloud-trace \
       --tracing-gcp-project-id YOUR_PROJECT_ID \
       --tracing-sample-rate 0.1
```

## Configuration Options

- `--tracing`: Enable distributed tracing
- `--tracing-service-name`: Service name for traces (default: "katana-rpc")
- `--tracing-exporter`: Exporter type: `google-cloud-trace`, `otlp`, or `none` (default: "none")
- `--tracing-gcp-project-id`: GCP project ID (required for Google Cloud Trace)
- `--tracing-otlp-endpoint`: OTLP endpoint URL (required for OTLP exporter)
- `--tracing-sample-rate`: Sampling rate from 0.0 to 1.0 (default: 1.0)

## Viewing Traces

### Google Cloud Trace

1. Navigate to the [Google Cloud Console](https://console.cloud.google.com)
2. Go to "Trace" in the navigation menu
3. You should see traces from your Katana node with the service name you configured

### Local OTLP with Jaeger

1. Run Jaeger locally:
   ```bash
   docker run -d --name jaeger \
     -e COLLECTOR_OTLP_ENABLED=true \
     -p 16686:16686 \
     -p 4317:4317 \
     -p 4318:4318 \
     jaegertracing/all-in-one:latest
   ```

2. Start Katana with OTLP:
   ```bash
   katana --tracing \
          --tracing-exporter otlp \
          --tracing-otlp-endpoint http://localhost:4317
   ```

3. View traces at http://localhost:16686

## Trace Context Propagation

Katana automatically extracts trace context from incoming HTTP headers using the W3C Trace Context standard. This allows you to link traces from your client applications to Katana's internal operations.

Example with curl:
```bash
curl -H "traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01" \
     -X POST http://localhost:5050 \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"starknet_chainId","params":[],"id":1}'
```

## Integration with Application Code

When making RPC calls from your application, use an OpenTelemetry-instrumented HTTP client to automatically propagate trace context:

```python
from opentelemetry import trace
from opentelemetry.exporter.cloud_trace import CloudTraceSpanExporter
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
import requests

# Setup tracing
tracer_provider = TracerProvider()
cloud_trace_exporter = CloudTraceSpanExporter(project_id="YOUR_PROJECT_ID")
tracer_provider.add_span_processor(BatchSpanProcessor(cloud_trace_exporter))
trace.set_tracer_provider(tracer_provider)

# Make RPC call with automatic trace propagation
tracer = trace.get_tracer(__name__)
with tracer.start_as_current_span("call_katana_rpc"):
    response = requests.post(
        "http://localhost:5050",
        json={
            "jsonrpc": "2.0",
            "method": "starknet_chainId",
            "params": [],
            "id": 1
        }
    )
```

## Performance Considerations

- Use sampling (`--tracing-sample-rate`) in production to reduce overhead
- Typical sampling rates: 0.01 (1%) to 0.1 (10%) for high-traffic environments
- Tracing adds minimal overhead when sampling is configured appropriately