/// Simple performance benchmarking utilities
use std::time::Instant;

pub struct Benchmark {
    pub name: String,
    pub start: Instant,
}

impl Benchmark {
    pub fn start(name: &str) -> Self {
        Benchmark {
            name: name.to_string(),
            start: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn report(&self) {
        let elapsed = self.elapsed_ms();
        println!("{}: {} ms", self.name, elapsed);
    }
}

impl Drop for Benchmark {
    fn drop(&mut self) {
        self.report();
    }
}

/// Track performance metrics
pub struct PerfStats {
    pub operation: String,
    pub count: u64,
    pub total_bytes: u64,
    pub total_ms: u64,
}

impl PerfStats {
    pub fn new(operation: &str) -> Self {
        PerfStats {
            operation: operation.to_string(),
            count: 0,
            total_bytes: 0,
            total_ms: 0,
        }
    }

    pub fn throughput_mbps(&self) -> f64 {
        if self.total_ms == 0 {
            0.0
        } else {
            (self.total_bytes as f64 / 1_000_000.0) / (self.total_ms as f64 / 1000.0)
        }
    }

    pub fn ops_per_sec(&self) -> f64 {
        if self.total_ms == 0 {
            0.0
        } else {
            (self.count as f64 * 1000.0) / self.total_ms as f64
        }
    }

    pub fn report(&self) {
        println!(
            "{}: {} ops, {} bytes, {} ms ({:.2} MB/s, {:.0} ops/s)",
            self.operation,
            self.count,
            self.total_bytes,
            self.total_ms,
            self.throughput_mbps(),
            self.ops_per_sec()
        );
    }
}
