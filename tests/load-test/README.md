# Katana Load Testing Tool

A comprehensive load testing tool for the Katana sequencer built with the Goose load testing framework.

## Features

- **Multiple Test Scenarios**: Constant load, ramp-up, burst testing, and custom scenarios
- **Real Transaction Load**: Simulates actual StarkNet transactions (ERC20 transfers, account deployments)
- **Comprehensive Metrics**: TPS, latency percentiles, success rates, and resource monitoring
- **Configurable Parameters**: Users, duration, transaction types, and load patterns
- **JSON Output**: Export results for analysis and reporting

## Installation

```bash
# Add to workspace Cargo.toml
cargo build -p katana-load-test
```

## Usage

### Basic Examples

```bash
# Constant load test - 50 TPS for 2 minutes
./target/debug/katana-load-test constant --tps 50 --duration 120 --users 25

# Ramp-up test - 1 to 100 TPS over 5 minutes, hold for 2 minutes
./target/debug/katana-load-test ramp-up --start-tps 1 --max-tps 100 --ramp-duration 300 --hold-duration 120

# Burst test - baseline 20 TPS with 200 TPS bursts
./target/debug/katana-load-test burst --baseline-tps 20 --burst-tps 200 --burst-duration 30 --total-duration 600

# Custom scenario with specific parameters
./target/debug/katana-load-test custom --users 100 --spawn-rate 5 --duration 300
```

### Configuration Options

#### Global Options
- `--rpc-url`: Katana RPC endpoint (default: http://localhost:5050)
- `--private-key`: Private key for signing transactions (env: KATANA_PRIVATE_KEY)
- `--account-address`: Account address (env: KATANA_ACCOUNT)
- `--debug`: Enable debug logging
- `--output`: JSON output file for results

#### Test-Specific Options

**Constant Load**
- `--tps`: Target transactions per second
- `--duration`: Test duration in seconds
- `--users`: Number of concurrent users

**Ramp-Up Load**
- `--start-tps`: Starting TPS
- `--max-tps`: Maximum TPS
- `--ramp-duration`: Ramp-up duration
- `--hold-duration`: Hold duration at max TPS

**Burst Load**
- `--baseline-tps`: Normal TPS
- `--burst-tps`: Burst TPS
- `--burst-duration`: Duration of each burst
- `--total-duration`: Total test duration

## Test Scenarios

### 1. Constant Load
Maintains steady transaction rate throughout the test duration.

**Use Case**: Baseline performance measurement, capacity planning.

### 2. Ramp-Up Load
Gradually increases transaction rate from start to max, then holds.

**Use Case**: Finding breaking points, stress testing.

### 3. Burst Load
Alternates between baseline and burst traffic patterns.

**Use Case**: Testing resilience to traffic spikes, queue handling.

### 4. Custom Scenario
Flexible configuration for specific testing needs.

**Use Case**: Reproducing specific load patterns, complex scenarios.

## Transaction Types

The tool simulates various StarkNet transaction types:

- **ERC20 Transfers**: Token transfers between accounts
- **Account Deployments**: New account creation
- **Contract Calls**: General contract interactions

Transaction parameters are randomized to simulate realistic load patterns.

## Metrics and Reporting

### Real-time Metrics
- Transactions per second (TPS)
- Response time percentiles (50th, 95th, 99th)
- Success/failure rates
- Error categorization

### Output Example
```
=== Load Test Summary ===
Total requests: 3000
Total users: 50
Success rate: 98.50%
Average response time: 45.23ms
95th percentile: 120.45ms
99th percentile: 250.78ms
Requests per second: 49.85
```

### JSON Export
```json
{
  "total_requests": 3000,
  "success_rate": 98.5,
  "avg_response_time": 45.23,
  "p95_response_time": 120.45,
  "p99_response_time": 250.78,
  "requests_per_second": 49.85,
  "errors": {
    "timeout": 15,
    "invalid_nonce": 30
  }
}
```

## Performance Considerations

### Optimal Configuration
- **Users**: Start with 2-5x target TPS, adjust based on latency
- **Spawn Rate**: Gradual ramp-up (2-10 users/second) for stable results
- **Duration**: Minimum 60 seconds for reliable metrics

### Resource Requirements
- **CPU**: ~0.1 core per 100 TPS
- **Memory**: ~50MB base + 1MB per 100 concurrent users
- **Network**: ~10KB per transaction

### Katana Configuration
For optimal load testing performance:

```toml
# Recommended Katana settings
[sequencing]
block_time = 1000  # 1 second blocks
max_transactions_per_block = 1000

[rpc]
max_connections = 1000
```

## Troubleshooting

### Common Issues

**Connection Refused**
```bash
# Ensure Katana is running
katana --host 0.0.0.0 --port 5050
```

**Nonce Errors**
- Use separate accounts for high-concurrency tests
- Enable nonce management in scenarios

**Low TPS**
- Increase user count
- Reduce transaction complexity
- Check Katana block time settings

### Debug Mode
```bash
./target/debug/katana-load-test --debug constant --tps 10 --duration 30
```

## Advanced Usage

### Environment Variables
```bash
export KATANA_PRIVATE_KEY="0x1800000000300000180000000000030000000000003006001800006600"
export KATANA_ACCOUNT="0x517ececd29116499f4a1b64b094da79ba08dfd54a3edaa316134c41f8160973"
```

### CI/CD Integration
```bash
# Automated performance regression testing
./target/debug/katana-load-test constant --tps 100 --duration 60 --output results.json
python analyze_results.py results.json
```

### Distributed Testing
For higher load, run multiple instances:
```bash
# Terminal 1
./target/debug/katana-load-test constant --tps 50 --duration 300

# Terminal 2
./target/debug/katana-load-test constant --tps 50 --duration 300
```

## Development

### Adding New Transaction Types
1. Extend `TransactionType` enum in `transactions.rs`
2. Implement transaction building logic
3. Add to scenario weight distribution

### Custom Scenarios
Implement new scenarios in `scenarios.rs` following the Goose pattern:
```rust
pub async fn custom_scenario(user: &mut GooseUser) -> TransactionResult {
    // Your custom logic here
}
```

## Contributing

1. Add new features with comprehensive tests
2. Update documentation for new scenarios
3. Ensure backward compatibility
4. Follow Katana coding standards

## License

Apache License 2.0 - see LICENSE file for details.
