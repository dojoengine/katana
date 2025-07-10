use anyhow::Result;
use serde_json::Value;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Performance metrics collector
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    pub start_time: Instant,
    pub total_transactions: u64,
    pub successful_transactions: u64,
    pub failed_transactions: u64,
    pub total_latency: Duration,
    pub min_latency: Option<Duration>,
    pub max_latency: Option<Duration>,
    pub latencies: Vec<Duration>,
}

impl PerformanceMetrics {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            total_transactions: 0,
            successful_transactions: 0,
            failed_transactions: 0,
            total_latency: Duration::ZERO,
            min_latency: None,
            max_latency: None,
            latencies: Vec::new(),
        }
    }

    pub fn record_transaction(&mut self, latency: Duration, success: bool) {
        self.total_transactions += 1;
        
        if success {
            self.successful_transactions += 1;
        } else {
            self.failed_transactions += 1;
        }

        self.total_latency += latency;
        self.latencies.push(latency);

        // Update min/max latency
        if self.min_latency.is_none() || latency < self.min_latency.unwrap() {
            self.min_latency = Some(latency);
        }
        if self.max_latency.is_none() || latency > self.max_latency.unwrap() {
            self.max_latency = Some(latency);
        }
    }

    pub fn tps(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.successful_transactions as f64 / elapsed
        } else {
            0.0
        }
    }

    pub fn success_rate(&self) -> f64 {
        if self.total_transactions > 0 {
            self.successful_transactions as f64 / self.total_transactions as f64 * 100.0
        } else {
            0.0
        }
    }

    pub fn average_latency(&self) -> Duration {
        if self.total_transactions > 0 {
            self.total_latency / self.total_transactions as u32
        } else {
            Duration::ZERO
        }
    }

    pub fn percentile_latency(&self, percentile: f64) -> Duration {
        if self.latencies.is_empty() {
            return Duration::ZERO;
        }

        let mut sorted_latencies = self.latencies.clone();
        sorted_latencies.sort();

        let index = (percentile / 100.0 * sorted_latencies.len() as f64) as usize;
        let index = index.min(sorted_latencies.len() - 1);
        
        sorted_latencies[index]
    }

    pub fn print_summary(&self) {
        let elapsed = self.start_time.elapsed();
        
        info!("=== Performance Summary ===");
        info!("Test Duration: {:.2}s", elapsed.as_secs_f64());
        info!("Total Transactions: {}", self.total_transactions);
        info!("Successful: {} ({:.1}%)", self.successful_transactions, self.success_rate());
        info!("Failed: {}", self.failed_transactions);
        info!("Transactions per Second: {:.2}", self.tps());
        info!("Average Latency: {:.2}ms", self.average_latency().as_millis());
        
        if let Some(min) = self.min_latency {
            info!("Min Latency: {:.2}ms", min.as_millis());
        }
        if let Some(max) = self.max_latency {
            info!("Max Latency: {:.2}ms", max.as_millis());
        }
        
        if !self.latencies.is_empty() {
            info!("50th Percentile: {:.2}ms", self.percentile_latency(50.0).as_millis());
            info!("95th Percentile: {:.2}ms", self.percentile_latency(95.0).as_millis());
            info!("99th Percentile: {:.2}ms", self.percentile_latency(99.0).as_millis());
        }
    }
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Utility functions for transaction validation
pub fn validate_transaction_response(response: &Value) -> Result<String> {
    if let Some(error) = response.get("error") {
        return Err(anyhow::anyhow!("Transaction error: {}", error));
    }

    let tx_hash = response
        .get("result")
        .and_then(|r| r.get("transaction_hash"))
        .and_then(|h| h.as_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid transaction response format"))?;

    Ok(tx_hash.to_string())
}

/// Rate limiter for controlling transaction submission rate
pub struct RateLimiter {
    interval: Duration,
    last_request: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_second: f64) -> Self {
        let interval = Duration::from_secs_f64(1.0 / requests_per_second);
        Self {
            interval,
            last_request: Instant::now() - interval, // Allow immediate first request
        }
    }

    pub async fn wait(&mut self) {
        let elapsed = self.last_request.elapsed();
        if elapsed < self.interval {
            let wait_time = self.interval - elapsed;
            tokio::time::sleep(wait_time).await;
        }
        self.last_request = Instant::now();
    }
}

/// Resource monitor for tracking system resources during load test
#[derive(Debug, Clone)]
pub struct ResourceMonitor {
    pub cpu_usage: Vec<f64>,
    pub memory_usage: Vec<u64>,
    pub timestamps: Vec<Instant>,
}

impl ResourceMonitor {
    pub fn new() -> Self {
        Self {
            cpu_usage: Vec::new(),
            memory_usage: Vec::new(),
            timestamps: Vec::new(),
        }
    }

    pub fn record_sample(&mut self, cpu: f64, memory: u64) {
        self.cpu_usage.push(cpu);
        self.memory_usage.push(memory);
        self.timestamps.push(Instant::now());
    }

    pub fn average_cpu(&self) -> f64 {
        if self.cpu_usage.is_empty() {
            0.0
        } else {
            self.cpu_usage.iter().sum::<f64>() / self.cpu_usage.len() as f64
        }
    }

    pub fn average_memory(&self) -> u64 {
        if self.memory_usage.is_empty() {
            0
        } else {
            self.memory_usage.iter().sum::<u64>() / self.memory_usage.len() as u64
        }
    }
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration validator
pub fn validate_config(rpc_url: &str) -> Result<()> {
    // Basic URL validation
    if !rpc_url.starts_with("http://") && !rpc_url.starts_with("https://") {
        return Err(anyhow::anyhow!("Invalid RPC URL format"));
    }

    // Additional validation can be added here
    debug!("Configuration validated successfully");
    Ok(())
}

/// Pretty print JSON for debugging
pub fn pretty_print_json(value: &Value) {
    if let Ok(pretty) = serde_json::to_string_pretty(value) {
        debug!("JSON: {}", pretty);
    }
}

/// Convert hex string to felt safely
pub fn safe_hex_to_felt(hex_str: &str) -> Result<starknet::core::types::Felt> {
    let cleaned = if hex_str.starts_with("0x") {
        &hex_str[2..]
    } else {
        hex_str
    };
    
    starknet::core::types::Felt::from_hex(hex_str)
        .map_err(|e| anyhow::anyhow!("Invalid hex string '{}': {}", hex_str, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_performance_metrics() {
        let mut metrics = PerformanceMetrics::new();
        
        // Record some transactions
        metrics.record_transaction(Duration::from_millis(100), true);
        metrics.record_transaction(Duration::from_millis(200), true);
        metrics.record_transaction(Duration::from_millis(150), false);
        
        assert_eq!(metrics.total_transactions, 3);
        assert_eq!(metrics.successful_transactions, 2);
        assert_eq!(metrics.failed_transactions, 1);
        assert_eq!(metrics.success_rate(), 66.66666666666667);
        
        // Check latency calculations
        assert_eq!(metrics.average_latency(), Duration::from_millis(150));
        assert_eq!(metrics.min_latency, Some(Duration::from_millis(100)));
        assert_eq!(metrics.max_latency, Some(Duration::from_millis(200)));
    }

    #[tokio::test]
    async fn test_rate_limiter() {
        let mut limiter = RateLimiter::new(2.0); // 2 requests per second
        
        let start = Instant::now();
        limiter.wait().await;
        limiter.wait().await;
        let elapsed = start.elapsed();
        
        // Should take at least 500ms for the second request
        assert!(elapsed >= Duration::from_millis(450));
    }

    #[test]
    fn test_config_validation() {
        assert!(validate_config("http://localhost:5050").is_ok());
        assert!(validate_config("https://api.example.com").is_ok());
        assert!(validate_config("invalid-url").is_err());
        assert!(validate_config("ftp://example.com").is_err());
    }
}
