use std::fmt::Write;
use std::sync::Arc;
use crate::metrics::Metrics;

/// Prometheus-compatible metrics exporter
pub struct PrometheusExporter {
    metrics: Arc<Metrics>,
}

impl PrometheusExporter {
    pub fn new(metrics: Arc<Metrics>) -> Self {
        PrometheusExporter { metrics }
    }

    /// Generate Prometheus metrics in text format
    pub fn export(&self) -> String {
        let snapshot = self.metrics.snapshot();
        let mut output = String::new();

        // HELP and TYPE comments for Prometheus
        writeln!(output, "# HELP dynamicfs_disk_reads_total Total disk read operations").unwrap();
        writeln!(output, "# TYPE dynamicfs_disk_reads_total counter").unwrap();
        writeln!(output, "dynamicfs_disk_reads_total {}", snapshot.disk_reads).unwrap();

        writeln!(output, "# HELP dynamicfs_disk_writes_total Total disk write operations").unwrap();
        writeln!(output, "# TYPE dynamicfs_disk_writes_total counter").unwrap();
        writeln!(output, "dynamicfs_disk_writes_total {}", snapshot.disk_writes).unwrap();

        writeln!(output, "# HELP dynamicfs_disk_read_bytes Total bytes read from disk").unwrap();
        writeln!(output, "# TYPE dynamicfs_disk_read_bytes counter").unwrap();
        writeln!(output, "dynamicfs_disk_read_bytes {}", snapshot.disk_read_bytes).unwrap();

        writeln!(output, "# HELP dynamicfs_disk_write_bytes Total bytes written to disk").unwrap();
        writeln!(output, "# TYPE dynamicfs_disk_write_bytes counter").unwrap();
        writeln!(output, "dynamicfs_disk_write_bytes {}", snapshot.disk_write_bytes).unwrap();

        writeln!(output, "# HELP dynamicfs_disk_errors_total Total disk errors").unwrap();
        writeln!(output, "# TYPE dynamicfs_disk_errors_total counter").unwrap();
        writeln!(output, "dynamicfs_disk_errors_total {}", snapshot.disk_errors).unwrap();

        writeln!(output, "# HELP dynamicfs_extents_healthy Number of healthy extents").unwrap();
        writeln!(output, "# TYPE dynamicfs_extents_healthy gauge").unwrap();
        writeln!(output, "dynamicfs_extents_healthy {}", snapshot.extents_healthy).unwrap();

        writeln!(output, "# HELP dynamicfs_extents_degraded Number of degraded extents").unwrap();
        writeln!(output, "# TYPE dynamicfs_extents_degraded gauge").unwrap();
        writeln!(output, "dynamicfs_extents_degraded {}", snapshot.extents_degraded).unwrap();

        writeln!(output, "# HELP dynamicfs_extents_unrecoverable Number of unrecoverable extents").unwrap();
        writeln!(output, "# TYPE dynamicfs_extents_unrecoverable gauge").unwrap();
        writeln!(output, "dynamicfs_extents_unrecoverable {}", snapshot.extents_unrecoverable).unwrap();

        writeln!(output, "# HELP dynamicfs_rebuilds_attempted Total rebuild attempts").unwrap();
        writeln!(output, "# TYPE dynamicfs_rebuilds_attempted counter").unwrap();
        writeln!(output, "dynamicfs_rebuilds_attempted {}", snapshot.rebuilds_attempted).unwrap();

        writeln!(output, "# HELP dynamicfs_rebuilds_successful Successful rebuilds").unwrap();
        writeln!(output, "# TYPE dynamicfs_rebuilds_successful counter").unwrap();
        writeln!(output, "dynamicfs_rebuilds_successful {}", snapshot.rebuilds_successful).unwrap();

        writeln!(output, "# HELP dynamicfs_rebuilds_failed Failed rebuilds").unwrap();
        writeln!(output, "# TYPE dynamicfs_rebuilds_failed counter").unwrap();
        writeln!(output, "dynamicfs_rebuilds_failed {}", snapshot.rebuilds_failed).unwrap();

        writeln!(output, "# HELP dynamicfs_rebuild_bytes_written Bytes written during rebuilds").unwrap();
        writeln!(output, "# TYPE dynamicfs_rebuild_bytes_written counter").unwrap();
        writeln!(output, "dynamicfs_rebuild_bytes_written {}", snapshot.rebuild_bytes_written).unwrap();

        writeln!(output, "# HELP dynamicfs_scrubs_completed Total completed scrubs").unwrap();
        writeln!(output, "# TYPE dynamicfs_scrubs_completed counter").unwrap();
        writeln!(output, "dynamicfs_scrubs_completed {}", snapshot.scrubs_completed).unwrap();

        writeln!(output, "# HELP dynamicfs_scrub_issues_found Total issues found by scrub").unwrap();
        writeln!(output, "# TYPE dynamicfs_scrub_issues_found counter").unwrap();
        writeln!(output, "dynamicfs_scrub_issues_found {}", snapshot.scrub_issues_found).unwrap();

        writeln!(output, "# HELP dynamicfs_scrub_repairs_attempted Total repair attempts").unwrap();
        writeln!(output, "# TYPE dynamicfs_scrub_repairs_attempted counter").unwrap();
        writeln!(output, "dynamicfs_scrub_repairs_attempted {}", snapshot.scrub_repairs_attempted).unwrap();

        writeln!(output, "# HELP dynamicfs_scrub_repairs_successful Successful repairs").unwrap();
        writeln!(output, "# TYPE dynamicfs_scrub_repairs_successful counter").unwrap();
        writeln!(output, "dynamicfs_scrub_repairs_successful {}", snapshot.scrub_repairs_successful).unwrap();

        writeln!(output, "# HELP dynamicfs_cache_hits Total cache hits").unwrap();
        writeln!(output, "# TYPE dynamicfs_cache_hits counter").unwrap();
        writeln!(output, "dynamicfs_cache_hits {}", snapshot.cache_hits).unwrap();

        writeln!(output, "# HELP dynamicfs_cache_misses Total cache misses").unwrap();
        writeln!(output, "# TYPE dynamicfs_cache_misses counter").unwrap();
        writeln!(output, "dynamicfs_cache_misses {}", snapshot.cache_misses).unwrap();

        // Derived metrics
        writeln!(output, "# HELP dynamicfs_disk_iops_total Total I/O operations per second").unwrap();
        writeln!(output, "# TYPE dynamicfs_disk_iops_total gauge").unwrap();
        writeln!(output, "dynamicfs_disk_iops_total {}", snapshot.total_iops()).unwrap();

        writeln!(output, "# HELP dynamicfs_cache_hit_rate Cache hit rate percentage").unwrap();
        writeln!(output, "# TYPE dynamicfs_cache_hit_rate gauge").unwrap();
        writeln!(output, "dynamicfs_cache_hit_rate {:.2}", snapshot.cache_hit_rate()).unwrap();

        writeln!(output, "# HELP dynamicfs_rebuild_success_rate Rebuild success rate percentage").unwrap();
        writeln!(output, "# TYPE dynamicfs_rebuild_success_rate gauge").unwrap();
        writeln!(output, "dynamicfs_rebuild_success_rate {:.2}", snapshot.rebuild_success_rate()).unwrap();

        output
    }

    /// Generate JSON metrics for structured logging
    pub fn export_json(&self) -> String {
        let snapshot = self.metrics.snapshot();
        
        format!(
            r#"{{
  "timestamp": "{}",
  "metrics": {{
    "disk": {{
      "reads": {},
      "writes": {},
      "read_bytes": {},
      "write_bytes": {},
      "errors": {},
      "total_iops": {}
    }},
    "extents": {{
      "healthy": {},
      "degraded": {},
      "unrecoverable": {}
    }},
    "rebuilds": {{
      "attempted": {},
      "successful": {},
      "failed": {},
      "bytes_written": {},
      "success_rate": {:.2}
    }},
    "scrubs": {{
      "completed": {},
      "issues_found": {},
      "repairs_attempted": {},
      "repairs_successful": {}
    }},
    "cache": {{
      "hits": {},
      "misses": {},
      "hit_rate": {:.2}
    }}
  }}
}}"#,
            chrono::Utc::now().to_rfc3339(),
            snapshot.disk_reads,
            snapshot.disk_writes,
            snapshot.disk_read_bytes,
            snapshot.disk_write_bytes,
            snapshot.disk_errors,
            snapshot.total_iops(),
            snapshot.extents_healthy,
            snapshot.extents_degraded,
            snapshot.extents_unrecoverable,
            snapshot.rebuilds_attempted,
            snapshot.rebuilds_successful,
            snapshot.rebuilds_failed,
            snapshot.rebuild_bytes_written,
            snapshot.rebuild_success_rate(),
            snapshot.scrubs_completed,
            snapshot.scrub_issues_found,
            snapshot.scrub_repairs_attempted,
            snapshot.scrub_repairs_successful,
            snapshot.cache_hits,
            snapshot.cache_misses,
            snapshot.cache_hit_rate(),
        )
    }
}

/// Health check status
#[derive(Debug, Clone)]
pub struct HealthCheckStatus {
    pub status: HealthStatus,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Critical,
}

impl HealthCheckStatus {
    pub fn to_json(&self) -> String {
        format!(
            r#"{{
  "status": "{}",
  "message": "{}",
  "timestamp": "{}"
}}"#,
            match self.status {
                HealthStatus::Healthy => "healthy",
                HealthStatus::Degraded => "degraded",
                HealthStatus::Critical => "critical",
            },
            self.message,
            self.timestamp,
        )
    }

    pub fn http_status_code(&self) -> u16 {
        match self.status {
            HealthStatus::Healthy => 200,
            HealthStatus::Degraded => 202,
            HealthStatus::Critical => 503,
        }
    }
}

/// Health checker for the filesystem
pub struct HealthChecker {
    metrics: Arc<Metrics>,
}

impl HealthChecker {
    pub fn new(metrics: Arc<Metrics>) -> Self {
        HealthChecker { metrics }
    }

    /// Perform comprehensive health check
    pub fn check(&self) -> HealthCheckStatus {
        let snapshot = self.metrics.snapshot();

        // Check for critical issues
        if snapshot.extents_unrecoverable > 0 {
            return HealthCheckStatus {
                status: HealthStatus::Critical,
                message: format!(
                    "CRITICAL: {} unrecoverable extents detected",
                    snapshot.extents_unrecoverable
                ),
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
        }

        // Check for degraded state
        if snapshot.disk_errors > 10 || snapshot.extents_degraded > 5 {
            return HealthCheckStatus {
                status: HealthStatus::Degraded,
                message: format!(
                    "DEGRADED: {} disk errors, {} degraded extents",
                    snapshot.disk_errors, snapshot.extents_degraded
                ),
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
        }

        // Check rebuild success rate
        if snapshot.rebuilds_attempted > 0 && snapshot.rebuild_success_rate() < 50.0 {
            return HealthCheckStatus {
                status: HealthStatus::Degraded,
                message: format!(
                    "DEGRADED: Low rebuild success rate: {:.1}%",
                    snapshot.rebuild_success_rate()
                ),
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
        }

        // Everything looks good
        HealthCheckStatus {
            status: HealthStatus::Healthy,
            message: format!(
                "HEALTHY: {} extents, {} disk I/O ops, cache hit rate {:.1}%",
                snapshot.extents_healthy + snapshot.extents_degraded,
                snapshot.total_iops(),
                snapshot.cache_hit_rate()
            ),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Quick readiness check
    pub fn is_ready(&self) -> bool {
        let check = self.check();
        check.status != HealthStatus::Critical
    }
}

