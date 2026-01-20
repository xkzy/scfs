use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// System-wide metrics collection
#[derive(Debug, Clone)]
pub struct Metrics {
    // Disk metrics
    pub disk_reads: Arc<AtomicU64>,
    pub disk_writes: Arc<AtomicU64>,
    pub disk_read_bytes: Arc<AtomicU64>,
    pub disk_write_bytes: Arc<AtomicU64>,
    pub disk_errors: Arc<AtomicU64>,

    // Extent metrics
    pub extents_healthy: Arc<AtomicU64>,
    pub extents_degraded: Arc<AtomicU64>,
    pub extents_unrecoverable: Arc<AtomicU64>,

    // Rebuild metrics
    pub rebuilds_attempted: Arc<AtomicU64>,
    pub rebuilds_successful: Arc<AtomicU64>,
    pub rebuilds_failed: Arc<AtomicU64>,
    pub rebuild_bytes_written: Arc<AtomicU64>,

    // Scrub metrics
    pub scrubs_completed: Arc<AtomicU64>,
    pub scrub_issues_found: Arc<AtomicU64>,
    pub scrub_repairs_attempted: Arc<AtomicU64>,
    pub scrub_repairs_successful: Arc<AtomicU64>,

    // Cache metrics
    pub cache_hits: Arc<AtomicU64>,
    pub cache_misses: Arc<AtomicU64>,
    
    // Phase 15: Concurrency metrics
    pub lock_acquisitions: Arc<AtomicU64>,
    pub lock_contentions: Arc<AtomicU64>,
    pub group_commits: Arc<AtomicU64>,
    pub group_commit_ops: Arc<AtomicU64>,
    pub io_queue_length: Arc<AtomicU64>,
    pub io_ops_completed: Arc<AtomicU64>,
}

impl Metrics {
    pub fn new() -> Self {
        Metrics {
            disk_reads: Arc::new(AtomicU64::new(0)),
            disk_writes: Arc::new(AtomicU64::new(0)),
            disk_read_bytes: Arc::new(AtomicU64::new(0)),
            disk_write_bytes: Arc::new(AtomicU64::new(0)),
            disk_errors: Arc::new(AtomicU64::new(0)),

            extents_healthy: Arc::new(AtomicU64::new(0)),
            extents_degraded: Arc::new(AtomicU64::new(0)),
            extents_unrecoverable: Arc::new(AtomicU64::new(0)),

            rebuilds_attempted: Arc::new(AtomicU64::new(0)),
            rebuilds_successful: Arc::new(AtomicU64::new(0)),
            rebuilds_failed: Arc::new(AtomicU64::new(0)),
            rebuild_bytes_written: Arc::new(AtomicU64::new(0)),

            scrubs_completed: Arc::new(AtomicU64::new(0)),
            scrub_issues_found: Arc::new(AtomicU64::new(0)),
            scrub_repairs_attempted: Arc::new(AtomicU64::new(0)),
            scrub_repairs_successful: Arc::new(AtomicU64::new(0)),

            cache_hits: Arc::new(AtomicU64::new(0)),
            cache_misses: Arc::new(AtomicU64::new(0)),
            
            // Phase 15: Concurrency metrics
            lock_acquisitions: Arc::new(AtomicU64::new(0)),
            lock_contentions: Arc::new(AtomicU64::new(0)),
            group_commits: Arc::new(AtomicU64::new(0)),
            group_commit_ops: Arc::new(AtomicU64::new(0)),
            io_queue_length: Arc::new(AtomicU64::new(0)),
            io_ops_completed: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn record_disk_read(&self, bytes: u64) {
        self.disk_reads.fetch_add(1, Ordering::Relaxed);
        self.disk_read_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_disk_write(&self, bytes: u64) {
        self.disk_writes.fetch_add(1, Ordering::Relaxed);
        self.disk_write_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_disk_error(&self) {
        self.disk_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_rebuild_start(&self) {
        self.rebuilds_attempted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_rebuild_success(&self, bytes: u64) {
        self.rebuilds_successful.fetch_add(1, Ordering::Relaxed);
        self.rebuild_bytes_written.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_rebuild_failure(&self) {
        self.rebuilds_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_scrub_completed(&self, issues: u64, repairs: u64, successful: u64) {
        self.scrubs_completed.fetch_add(1, Ordering::Relaxed);
        self.scrub_issues_found.fetch_add(issues, Ordering::Relaxed);
        self.scrub_repairs_attempted.fetch_add(repairs, Ordering::Relaxed);
        self.scrub_repairs_successful.fetch_add(successful, Ordering::Relaxed);
    }

    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }
    
    // Phase 15: Concurrency metric recording
    
    pub fn record_lock_acquisition(&self) {
        self.lock_acquisitions.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn record_lock_contention(&self) {
        self.lock_contentions.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn record_group_commit(&self, ops_count: u64) {
        self.group_commits.fetch_add(1, Ordering::Relaxed);
        self.group_commit_ops.fetch_add(ops_count, Ordering::Relaxed);
    }
    
    pub fn update_io_queue_length(&self, length: u64) {
        self.io_queue_length.store(length, Ordering::Relaxed);
    }
    
    pub fn record_io_op_completed(&self) {
        self.io_ops_completed.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Get average operations per group commit
    pub fn avg_group_commit_ops(&self) -> f64 {
        let commits = self.group_commits.load(Ordering::Relaxed);
        if commits == 0 {
            return 0.0;
        }
        let ops = self.group_commit_ops.load(Ordering::Relaxed);
        ops as f64 / commits as f64
    }
    
    /// Get lock contention ratio (contentions / acquisitions)
    pub fn lock_contention_ratio(&self) -> f64 {
        let acquisitions = self.lock_acquisitions.load(Ordering::Relaxed);
        if acquisitions == 0 {
            return 0.0;
        }
        let contentions = self.lock_contentions.load(Ordering::Relaxed);
        contentions as f64 / acquisitions as f64
    }

    /// Get snapshot of all metrics
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            disk_reads: self.disk_reads.load(Ordering::Relaxed),
            disk_writes: self.disk_writes.load(Ordering::Relaxed),
            disk_read_bytes: self.disk_read_bytes.load(Ordering::Relaxed),
            disk_write_bytes: self.disk_write_bytes.load(Ordering::Relaxed),
            disk_errors: self.disk_errors.load(Ordering::Relaxed),
            extents_healthy: self.extents_healthy.load(Ordering::Relaxed),
            extents_degraded: self.extents_degraded.load(Ordering::Relaxed),
            extents_unrecoverable: self.extents_unrecoverable.load(Ordering::Relaxed),
            rebuilds_attempted: self.rebuilds_attempted.load(Ordering::Relaxed),
            rebuilds_successful: self.rebuilds_successful.load(Ordering::Relaxed),
            rebuilds_failed: self.rebuilds_failed.load(Ordering::Relaxed),
            rebuild_bytes_written: self.rebuild_bytes_written.load(Ordering::Relaxed),
            scrubs_completed: self.scrubs_completed.load(Ordering::Relaxed),
            scrub_issues_found: self.scrub_issues_found.load(Ordering::Relaxed),
            scrub_repairs_attempted: self.scrub_repairs_attempted.load(Ordering::Relaxed),
            scrub_repairs_successful: self.scrub_repairs_successful.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            lock_acquisitions: self.lock_acquisitions.load(Ordering::Relaxed),
            lock_contentions: self.lock_contentions.load(Ordering::Relaxed),
            group_commits: self.group_commits.load(Ordering::Relaxed),
            group_commit_ops: self.group_commit_ops.load(Ordering::Relaxed),
            io_queue_length: self.io_queue_length.load(Ordering::Relaxed),
            io_ops_completed: self.io_ops_completed.load(Ordering::Relaxed),
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Point-in-time snapshot of metrics
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub disk_reads: u64,
    pub disk_writes: u64,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub disk_errors: u64,
    pub extents_healthy: u64,
    pub extents_degraded: u64,
    pub extents_unrecoverable: u64,
    pub rebuilds_attempted: u64,
    pub rebuilds_successful: u64,
    pub rebuilds_failed: u64,
    pub rebuild_bytes_written: u64,
    pub scrubs_completed: u64,
    pub scrub_issues_found: u64,
    pub scrub_repairs_attempted: u64,
    pub scrub_repairs_successful: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    // Phase 15: Concurrency metrics
    pub lock_acquisitions: u64,
    pub lock_contentions: u64,
    pub group_commits: u64,
    pub group_commit_ops: u64,
    pub io_queue_length: u64,
    pub io_ops_completed: u64,
}

impl MetricsSnapshot {
    pub fn total_iops(&self) -> u64 {
        self.disk_reads + self.disk_writes
    }

    pub fn total_bytes_transferred(&self) -> u64 {
        self.disk_read_bytes + self.disk_write_bytes
    }

    pub fn rebuild_success_rate(&self) -> f64 {
        if self.rebuilds_attempted == 0 {
            100.0
        } else {
            100.0 * self.rebuilds_successful as f64 / self.rebuilds_attempted as f64
        }
    }

    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            100.0 * self.cache_hits as f64 / total as f64
        }
    }
}

impl std::fmt::Display for MetricsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            r#"Metrics Snapshot:
  Disk I/O:
    Reads:  {} ({} bytes)
    Writes: {} ({} bytes)
    Errors: {}
  Extents:
    Healthy:      {}
    Degraded:     {}
    Unrecoverable: {}
  Rebuilds:
    Attempted:    {} (success rate: {:.1}%)
    Successful:   {}
    Failed:       {}
    Bytes written: {}
  Scrubs:
    Completed:    {}
    Issues found: {}
    Repairs:      {} attempted, {} successful
  Cache:
    Hits:   {} (hit rate: {:.1}%)
    Misses: {}
"#,
            self.disk_reads,
            self.disk_read_bytes,
            self.disk_writes,
            self.disk_write_bytes,
            self.disk_errors,
            self.extents_healthy,
            self.extents_degraded,
            self.extents_unrecoverable,
            self.rebuilds_attempted,
            self.rebuild_success_rate(),
            self.rebuilds_successful,
            self.rebuilds_failed,
            self.rebuild_bytes_written,
            self.scrubs_completed,
            self.scrub_issues_found,
            self.scrub_repairs_attempted,
            self.scrub_repairs_successful,
            self.cache_hits,
            self.cache_hit_rate(),
            self.cache_misses,
        )
    }
}
