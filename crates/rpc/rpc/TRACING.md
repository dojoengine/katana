# Distributed Tracing in Katana RPC

This document describes the distributed tracing implementation in the Katana RPC server.

## Overview

The Katana RPC server supports distributed tracing through OpenTelemetry, with built-in support for:
- Google Cloud Trace
- OpenTelemetry Protocol (OTLP) exporters
- W3C Trace Context propagation

## Architecture

The tracing implementation consists of several components:

### TracingConfig
Configuration structure that defines:
- `service_name`: The name of the service in traces
- `exporter`: The exporter type (GoogleCloudTrace, OTLP, or None)
- `sample_rate`: Sampling rate from 0.0 to 1.0

### TracingLayer
A Tower middleware layer that:
- Extracts trace context from incoming HTTP headers
- Creates spans for HTTP requests
- Propagates context to downstream services

### RpcTracingLayer
A specialized layer for RPC-specific tracing that ensures proper context propagation through the RPC call stack.

## Implementation Details

### Trace Context Extraction
The middleware extracts trace context from HTTP headers using the W3C Trace Context standard (`traceparent` header).

### Span Creation
Each RPC request creates a span with:
- Operation name: `HTTP {method} {uri}`
- Span kind: Server
- Attributes:
  - `http.method`
  - `http.target`
  - `http.scheme`
  - `rpc.system`: "jsonrpc"
  - `rpc.service`: "katana"

### Context Propagation
The trace context is propagated through the request lifecycle using:
1. OpenTelemetry context management
2. Tracing crate integration via `tracing-opentelemetry`

## Usage

### Programmatic Usage

```rust
use katana_rpc::{RpcServer, tracing::{TracingConfig, TracingExporter}};

let tracing_config = TracingConfig {
    service_name: "my-katana-node".to_string(),
    exporter: TracingExporter::GoogleCloudTrace {
        project_id: "my-project".to_string(),
    },
    sample_rate: 0.1,
};

let server = RpcServer::new()
    .tracing(tracing_config)
    .module(rpc_module)?;

let handle = server.start(addr).await?;
```

### CLI Usage

```bash
katana --tracing \
       --tracing-exporter google-cloud-trace \
       --tracing-gcp-project-id YOUR_PROJECT_ID \
       --tracing-sample-rate 0.1
```

## Exporters

### Google Cloud Trace
Requires:
- GCP project with Cloud Trace API enabled
- Authentication via Application Default Credentials or service account

### OTLP (OpenTelemetry Protocol)
Requires:
- OTLP-compatible backend (e.g., Jaeger, Tempo, etc.)
- Endpoint URL configuration

## Performance Considerations

- Tracing adds minimal overhead when properly configured
- Use sampling in production environments
- Typical sampling rates: 0.01-0.1 for high-traffic systems
- Spans are created asynchronously to minimize latency impact

## Future Enhancements

Potential improvements:
- Additional span attributes for StarkNet-specific data
- Custom sampling strategies based on request type
- Integration with metrics collection
- Support for additional exporters (Zipkin, AWS X-Ray)