use serde::{Serialize, Deserialize};

/// Filesystem configuration with sensible defaults
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub storage: StorageConfig,
    pub performance: PerformanceConfig,
    pub reliability: ReliabilityConfig,
    pub monitoring: MonitoringConfig,
    pub security: SecurityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Default extent size in bytes
    pub default_extent_size: usize,
    /// Maximum file size in bytes
    pub max_file_size: u64,
    /// Maximum extents per file
    pub max_extents_per_file: usize,
    /// Enable compression
    pub enable_compression: bool,
    /// Enable deduplication
    pub enable_deduplication: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    /// Enable write batching
    pub enable_write_batching: bool,
    /// Write batch size
    pub write_batch_size: usize,
    /// Maximum parallel writes
    pub max_parallel_writes: usize,
    /// Enable metadata caching
    pub enable_metadata_cache: bool,
    /// Metadata cache size
    pub metadata_cache_size: usize,
    /// Enable read-ahead
    pub enable_read_ahead: bool,
    /// Read-ahead buffer size
    pub read_ahead_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReliabilityConfig {
    /// Enable automatic rebuilds
    pub enable_auto_rebuild: bool,
    /// Rebuild concurrency
    pub rebuild_concurrency: usize,
    /// Enable automatic scrubbing
    pub enable_auto_scrub: bool,
    /// Scrub interval in hours
    pub scrub_interval_hours: u32,
    /// Repair on scrub
    pub repair_on_scrub: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Enable metrics collection
    pub enable_metrics: bool,
    /// Metrics batch size
    pub metrics_batch_size: usize,
    /// Enable logging
    pub enable_logging: bool,
    /// Log level
    pub log_level: String,
    /// Maximum log events
    pub max_log_events: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Enable audit logging
    pub enable_audit_logging: bool,
    /// FUSE allow_other
    pub fuse_allow_other: bool,
    /// Enforce access control
    pub enforce_access_control: bool,
    /// Maximum open files
    pub max_open_files: usize,
}

impl Config {
    /// Default configuration for production
    pub fn production() -> Self {
        Config {
            storage: StorageConfig {
                default_extent_size: 4 * 1024 * 1024,  // 4MB
                max_file_size: 1024 * 1024 * 1024 * 1024, // 1TB
                max_extents_per_file: 256,
                enable_compression: false,
                enable_deduplication: false,
            },
            performance: PerformanceConfig {
                enable_write_batching: true,
                write_batch_size: 10,
                max_parallel_writes: 16,
                enable_metadata_cache: true,
                metadata_cache_size: 1000,
                enable_read_ahead: true,
                read_ahead_size: 64 * 1024,
            },
            reliability: ReliabilityConfig {
                enable_auto_rebuild: true,
                rebuild_concurrency: 4,
                enable_auto_scrub: true,
                scrub_interval_hours: 24,
                repair_on_scrub: true,
            },
            monitoring: MonitoringConfig {
                enable_metrics: true,
                metrics_batch_size: 100,
                enable_logging: true,
                log_level: "info".to_string(),
                max_log_events: 10000,
            },
            security: SecurityConfig {
                enable_audit_logging: true,
                fuse_allow_other: false,
                enforce_access_control: true,
                max_open_files: 4096,
            },
        }
    }

    /// Development configuration
    pub fn development() -> Self {
        let mut config = Self::production();
        config.performance.enable_write_batching = false;
        config.reliability.enable_auto_scrub = false;
        config.monitoring.log_level = "debug".to_string();
        config.security.enforce_access_control = false;
        config
    }

    /// Testing configuration
    pub fn testing() -> Self {
        let mut config = Self::development();
        config.storage.default_extent_size = 1024; // 1KB for testing
        config.monitoring.max_log_events = 1000;
        config
    }

    /// High-performance configuration
    pub fn high_performance() -> Self {
        let mut config = Self::production();
        config.performance.write_batch_size = 50;
        config.performance.max_parallel_writes = 64;
        config.performance.metadata_cache_size = 10000;
        config.performance.read_ahead_size = 256 * 1024;
        config.reliability.rebuild_concurrency = 8;
        config
    }

    /// Load from JSON file
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        serde_json::from_str(json).map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))
    }

    /// Save to JSON file
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Merge with another config (other overwrites self)
    pub fn merge(&self, other: &Config) -> Config {
        let mut merged = self.clone();
        
        // Merge each section
        merged.storage = other.storage.clone();
        merged.performance = other.performance.clone();
        merged.reliability = other.reliability.clone();
        merged.monitoring = other.monitoring.clone();
        merged.security = other.security.clone();

        merged
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.storage.default_extent_size == 0 {
            errors.push("default_extent_size must be > 0".to_string());
        }

        if self.storage.max_extents_per_file == 0 {
            errors.push("max_extents_per_file must be > 0".to_string());
        }

        if self.performance.write_batch_size == 0 {
            errors.push("write_batch_size must be > 0".to_string());
        }

        if self.performance.max_parallel_writes == 0 {
            errors.push("max_parallel_writes must be > 0".to_string());
        }

        if self.performance.metadata_cache_size == 0 {
            errors.push("metadata_cache_size must be > 0".to_string());
        }

        if self.security.max_open_files == 0 {
            errors.push("max_open_files must be > 0".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::production()
    }
}

/// Configuration builder for fluent API
pub struct ConfigBuilder {
    config: Config,
}

impl ConfigBuilder {
    pub fn new() -> Self {
        ConfigBuilder {
            config: Config::production(),
        }
    }

    pub fn from_preset(preset: &str) -> Self {
        let config = match preset {
            "development" => Config::development(),
            "testing" => Config::testing(),
            "high_performance" => Config::high_performance(),
            _ => Config::production(),
        };

        ConfigBuilder { config }
    }

    pub fn extent_size(mut self, size: usize) -> Self {
        self.config.storage.default_extent_size = size;
        self
    }

    pub fn enable_write_batching(mut self, enable: bool) -> Self {
        self.config.performance.enable_write_batching = enable;
        self
    }

    pub fn enable_auto_scrub(mut self, enable: bool) -> Self {
        self.config.reliability.enable_auto_scrub = enable;
        self
    }

    pub fn log_level(mut self, level: impl Into<String>) -> Self {
        self.config.monitoring.log_level = level.into();
        self
    }

    pub fn build(self) -> anyhow::Result<Config> {
        match self.config.validate() {
            Ok(()) => Ok(self.config),
            Err(errors) => Err(anyhow::anyhow!("Configuration validation failed: {}", errors.join("; ")))
        }
    }
}

