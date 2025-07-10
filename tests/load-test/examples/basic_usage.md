# Katana Load Testing - Basic Usage Examples

## Quick Start

```bash
# Build the load testing tool
cargo build -p katana-load-test

# Start Katana in another terminal
katana --host 0.0.0.0 --port 5050
```

## Viewing Results & Metrics

### 1. Real-time Console Output
By default, Goose displays live metrics in the terminal:

```bash
# Basic load test with live console metrics
./target/debug/katana-load-test constant --tps 20 --duration 60 --users 10
```

**Sample Output:**
```
=== PER TRANSACTION METRICS ===
Name                     |   # times run |        # fails |  trans/s |  fail/s
send_transaction         |           489 |         2 (0%) |     8.15 |    0.03
check_tx_status          |           156 |         0 (0%) |     2.60 |    0.00

Name                     |    Avg (ms) |        Min |         Max |     Median
send_transaction         |      245.3  |         89 |       1,234 |        210
check_tx_status          |       45.2  |         12 |         156 |         38
```

### 2. HTML Reports with Interactive Charts
Generate comprehensive HTML reports with graphs and detailed breakdowns:

```bash
# Generate HTML report
./target/debug/katana-load-test constant \
  --tps 50 --duration 120 --users 25 \
  --html-report katana_performance_report.html

# Open the report in your browser
open katana_performance_report.html
```

**HTML Report Features:**
- Interactive response time charts
- Transaction distribution graphs
- Error rate visualization
- Percentile breakdowns
- Timeline of load test execution

### 3. Real-time Dashboard & Control
Enable real-time monitoring and control via WebSocket:

```bash
# Start with dashboard enabled
./target/debug/katana-load-test constant \
  --tps 30 --duration 300 --users 15 \
  --dashboard

# Dashboard available at: ws://localhost:5117
# Telnet control available at: telnet localhost 5116
```

**Dashboard Features:**
- Live TPS monitoring
- Real-time error rates
- Response time graphs
- Start/stop/pause controls

### 4. Manual Control Mode
Start the load test without auto-starting, control via telnet/WebSocket:

```bash
# Start in manual mode
./target/debug/katana-load-test constant \
  --tps 100 --duration 600 --users 50 \
  --no-autostart --dashboard

# Control via telnet
telnet localhost 5116
> start
> stop
> config
> metrics
```

### 5. JSON Export for Analysis
Export raw metrics data for custom analysis:

```bash
# Export to JSON file
./target/debug/katana-load-test ramp-up \
  --start-tps 1 --max-tps 200 --ramp-duration 300 \
  --output katana_metrics.json \
  --html-report katana_report.html

# Analyze with custom tools
python analyze_katana_performance.py katana_metrics.json
```

## Sample Test Scenarios

### Finding Maximum TPS
```bash
# Ramp up to find breaking point
./target/debug/katana-load-test ramp-up \
  --start-tps 1 --max-tps 500 \
  --ramp-duration 600 --hold-duration 120 \
  --html-report max_tps_test.html
```

### Stress Testing with Bursts
```bash
# Test resilience to traffic spikes
./target/debug/katana-load-test burst \
  --baseline-tps 50 --burst-tps 300 \
  --burst-duration 30 --total-duration 600 \
  --dashboard --html-report burst_test.html
```

### Sustained Load Testing
```bash
# Long-running constant load
./target/debug/katana-load-test constant \
  --tps 100 --duration 3600 --users 50 \
  --html-report sustained_load.html \
  --output sustained_metrics.json
```

### Monitoring During Test

**Terminal 1: Load Test**
```bash
./target/debug/katana-load-test constant --tps 75 --duration 300 --dashboard
```

**Terminal 2: Real-time Telnet Control**
```bash
telnet localhost 5116
> metrics    # View current metrics
> config     # Show configuration
> pause      # Pause the test
> resume     # Resume the test
```

**Browser: WebSocket Dashboard**
```
Open: ws://localhost:5117 in a WebSocket client
Real-time charts and controls available
```

## Interpreting Results

### Key Metrics to Monitor
- **TPS (Transactions Per Second)**: Actual throughput achieved
- **Response Time Percentiles**: 50th, 95th, 99th percentiles
- **Error Rate**: Failed transactions percentage
- **Success Rate**: Successful transactions percentage

### Sample Analysis
```
=== Load Test Summary ===
Total requests: 4,523
Success rate: 98.2%
Average response time: 156.7ms
95th percentile: 445ms
99th percentile: 1.2s
Requests per second: 75.4

# Analysis:
# - Achieved 75.4 TPS vs target 75 TPS ✓
# - 98.2% success rate is excellent ✓
# - 95th percentile under 500ms is good ✓
# - 99th percentile over 1s indicates some slow transactions ⚠️
```

### Performance Tuning

**If TPS is too low:**
- Increase user count
- Reduce wait times between requests
- Check Katana block time settings

**If error rate is high:**
- Reduce load intensity
- Check nonce management
- Verify transaction validity

**If latency is high:**
- Monitor Katana resource usage
- Check network connectivity
- Reduce transaction complexity

## Integration with CI/CD

```bash
#!/bin/bash
# performance_test.sh

# Start Katana
katana --host 0.0.0.0 --port 5050 &
KATANA_PID=$!

# Wait for startup
sleep 5

# Run performance test
./target/debug/katana-load-test constant \
  --tps 50 --duration 120 --users 25 \
  --output ci_metrics.json

# Analyze results
python validate_performance.py ci_metrics.json

# Cleanup
kill $KATANA_PID
```

This comprehensive monitoring and reporting setup provides deep insights into Katana's performance characteristics under various load conditions.
