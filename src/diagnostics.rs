use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// System diagnostic information
#[derive(Debug, Clone)]
pub struct Diagnostics {
    pub timestamp: DateTime<Utc>,
    pub health: DiagnosticHealth,
    pub performance: PerformanceDiagnostics,
    pub reliability: ReliabilityDiagnostics,
    pub capacity: CapacityDiagnostics,
    pub issues: Vec<DiagnosticIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticHealth {
    Healthy,
    Warning,
    Critical,
}

#[derive(Debug, Clone)]
pub struct PerformanceDiagnostics {
    pub read_latency_ms: f64,
    pub write_latency_ms: f64,
    pub throughput_mbps: f64,
    pub cache_efficiency: f64,
}

#[derive(Debug, Clone)]
pub struct ReliabilityDiagnostics {
    pub disk_error_rate: f64,
    pub rebuild_success_rate: f64,
    pub scrub_issues_found: u64,
    pub mean_time_to_failure_hours: u64,
}

#[derive(Debug, Clone)]
pub struct CapacityDiagnostics {
    pub used_percent: f64,
    pub estimated_remaining_hours: u64,
    pub hot_tier_usage: f64,
    pub cold_tier_usage: f64,
}

#[derive(Debug, Clone)]
pub struct DiagnosticIssue {
    pub severity: IssueSeverity,
    pub category: IssueCategory,
    pub title: String,
    pub description: String,
    pub recommendation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IssueSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueCategory {
    Performance,
    Reliability,
    Capacity,
    Security,
    Configuration,
}

/// Diagnostic analyzer
pub struct DiagnosticAnalyzer {
    performance_baselines: HashMap<String, f64>,
}

impl DiagnosticAnalyzer {
    pub fn new() -> Self {
        let mut baselines = HashMap::new();
        
        // Set baseline expectations
        baselines.insert("read_latency_ms".to_string(), 10.0);
        baselines.insert("write_latency_ms".to_string(), 20.0);
        baselines.insert("throughput_mbps".to_string(), 100.0);
        baselines.insert("cache_efficiency".to_string(), 80.0);
        baselines.insert("disk_error_rate".to_string(), 0.1);
        baselines.insert("capacity_threshold".to_string(), 80.0);

        DiagnosticAnalyzer {
            performance_baselines: baselines,
        }
    }

    /// Analyze system state and generate diagnostics
    pub fn analyze(
        &self,
        read_latency: f64,
        write_latency: f64,
        throughput: f64,
        cache_efficiency: f64,
        disk_error_rate: f64,
        rebuild_success_rate: f64,
        scrub_issues: u64,
        capacity_used: f64,
    ) -> Diagnostics {
        let mut issues = Vec::new();

        // Check performance
        if read_latency > self.performance_baselines["read_latency_ms"] * 2.0 {
            issues.push(DiagnosticIssue {
                severity: IssueSeverity::Warning,
                category: IssueCategory::Performance,
                title: "High read latency".to_string(),
                description: format!("Read latency is {} ms, expected < {} ms",
                    read_latency as i64,
                    (self.performance_baselines["read_latency_ms"] * 2.0) as i64
                ),
                recommendation: "Check disk utilization, consider enabling caching".to_string(),
            });
        }

        if write_latency > self.performance_baselines["write_latency_ms"] * 3.0 {
            issues.push(DiagnosticIssue {
                severity: IssueSeverity::Warning,
                category: IssueCategory::Performance,
                title: "High write latency".to_string(),
                description: format!("Write latency is {} ms", write_latency as i64),
                recommendation: "Check disk I/O and fsync performance".to_string(),
            });
        }

        if cache_efficiency < 50.0 {
            issues.push(DiagnosticIssue {
                severity: IssueSeverity::Info,
                category: IssueCategory::Performance,
                title: "Low cache efficiency".to_string(),
                description: format!("Cache hit rate is {:.1}%", cache_efficiency),
                recommendation: "Increase cache size or verify workload pattern".to_string(),
            });
        }

        // Check reliability
        if disk_error_rate > self.performance_baselines["disk_error_rate"] * 10.0 {
            issues.push(DiagnosticIssue {
                severity: IssueSeverity::Critical,
                category: IssueCategory::Reliability,
                title: "High disk error rate".to_string(),
                description: format!("Error rate is {:.2}%", disk_error_rate),
                recommendation: "Inspect disk health, replace if necessary".to_string(),
            });
        }

        if rebuild_success_rate < 50.0 {
            issues.push(DiagnosticIssue {
                severity: IssueSeverity::Error,
                category: IssueCategory::Reliability,
                title: "Low rebuild success rate".to_string(),
                description: format!("Only {:.1}% of rebuilds successful", rebuild_success_rate),
                recommendation: "Check disk availability and network connectivity".to_string(),
            });
        }

        if scrub_issues > 100 {
            issues.push(DiagnosticIssue {
                severity: IssueSeverity::Warning,
                category: IssueCategory::Reliability,
                title: "Many scrub issues detected".to_string(),
                description: format!("{} integrity issues found", scrub_issues),
                recommendation: "Run repair operation: `scrub --repair`".to_string(),
            });
        }

        // Check capacity
        if capacity_used > self.performance_baselines["capacity_threshold"] {
            issues.push(DiagnosticIssue {
                severity: IssueSeverity::Warning,
                category: IssueCategory::Capacity,
                title: "High capacity usage".to_string(),
                description: format!("{:.1}% of capacity used", capacity_used),
                recommendation: "Plan to add more disks or enable tiering to cold storage".to_string(),
            });
        }

        if capacity_used > 95.0 {
            issues.push(DiagnosticIssue {
                severity: IssueSeverity::Critical,
                category: IssueCategory::Capacity,
                title: "Critically high capacity usage".to_string(),
                description: format!("{:.1}% of capacity used - running out of space", capacity_used),
                recommendation: "Add more disks immediately or delete data".to_string(),
            });
        }

        // Determine overall health
        let health = if issues.iter().any(|i| i.severity == IssueSeverity::Critical) {
            DiagnosticHealth::Critical
        } else if issues.iter().any(|i| i.severity == IssueSeverity::Error) {
            DiagnosticHealth::Warning
        } else if issues.iter().any(|i| i.severity == IssueSeverity::Warning) {
            DiagnosticHealth::Warning
        } else {
            DiagnosticHealth::Healthy
        };

        Diagnostics {
            timestamp: Utc::now(),
            health,
            performance: PerformanceDiagnostics {
                read_latency_ms: read_latency,
                write_latency_ms: write_latency,
                throughput_mbps: throughput,
                cache_efficiency: cache_efficiency,
            },
            reliability: ReliabilityDiagnostics {
                disk_error_rate,
                rebuild_success_rate,
                scrub_issues_found: scrub_issues,
                mean_time_to_failure_hours: Self::estimate_mttf(disk_error_rate),
            },
            capacity: CapacityDiagnostics {
                used_percent: capacity_used,
                estimated_remaining_hours: Self::estimate_full_time(capacity_used),
                hot_tier_usage: 0.0,
                cold_tier_usage: 0.0,
            },
            issues,
        }
    }

    fn estimate_mttf(error_rate: f64) -> u64 {
        if error_rate < 0.001 {
            100000 // > 11 years
        } else if error_rate < 0.01 {
            50000
        } else if error_rate < 0.1 {
            5000
        } else {
            500
        }
    }

    fn estimate_full_time(used_percent: f64) -> u64 {
        let growth_rate = 0.1; // 10% per day
        let remaining_percent = 100.0 - used_percent;
        if growth_rate > 0.0 {
            (remaining_percent / growth_rate) as u64 * 24 // convert days to hours
        } else {
            u64::MAX
        }
    }
}

/// Runbook for common issues
pub struct Runbook;

impl Runbook {
    pub fn get_resolution(issue: &DiagnosticIssue) -> Vec<String> {
        match (issue.severity, issue.category) {
            (IssueSeverity::Critical, IssueCategory::Capacity) => vec![
                "1. Check current capacity: `status`".to_string(),
                "2. Add a new disk: `add-disk /pool /disk-path`".to_string(),
                "3. Enable tiering: Configure tiering policy".to_string(),
                "4. If urgent, delete old data and run scrub: `scrub --repair`".to_string(),
            ],
            (IssueSeverity::Critical, IssueCategory::Reliability) => vec![
                "1. Run diagnostics: `status`".to_string(),
                "2. Identify failed disks: `probe-disks`".to_string(),
                "3. Mark disk as failed: `set-disk-health /disk failed`".to_string(),
                "4. Replace disk and rebuild: System will auto-rebuild on mount".to_string(),
            ],
            (IssueSeverity::Warning, IssueCategory::Performance) => vec![
                "1. Check disk utilization: `metrics`".to_string(),
                "2. Review cache settings: Check configuration".to_string(),
                "3. Enable write batching if disabled: Update config".to_string(),
                "4. Monitor for 24 hours, then re-run diagnostics".to_string(),
            ],
            _ => vec![
                "1. Review the issue description above".to_string(),
                "2. Follow the recommendation provided".to_string(),
                "3. Run scrub to verify fix: `scrub`".to_string(),
            ],
        }
    }

    pub fn quick_recovery_steps() -> Vec<String> {
        vec![
            "QUICK RECOVERY STEPS:".to_string(),
            "".to_string(),
            "1. Mount filesystem (auto-rebuild on mount)".to_string(),
            "   $ dynamicfs mount /pool /mnt".to_string(),
            "".to_string(),
            "2. Check health".to_string(),
            "   $ dynamicfs status /pool".to_string(),
            "".to_string(),
            "3. Run integrity check".to_string(),
            "   $ dynamicfs scrub /pool".to_string(),
            "".to_string(),
            "4. Repair if issues found".to_string(),
            "   $ dynamicfs scrub /pool --repair".to_string(),
            "".to_string(),
            "5. Verify metrics".to_string(),
            "   $ dynamicfs metrics /pool".to_string(),
        ]
    }
}

/// Diagnostics formatter for human-readable output
pub struct DiagnosticsFormatter;

impl DiagnosticsFormatter {
    pub fn format_text(diag: &Diagnostics) -> String {
        let mut output = format!(
            r#"===== DIAGNOSTICS REPORT =====
Timestamp: {}
Overall Health: {:?}

PERFORMANCE:
  Read Latency: {:.2} ms
  Write Latency: {:.2} ms
  Throughput: {:.2} MB/s
  Cache Efficiency: {:.1}%

RELIABILITY:
  Disk Error Rate: {:.2}%
  Rebuild Success Rate: {:.1}%
  Scrub Issues: {}
  Est. MTTF: {} hours

CAPACITY:
  Used: {:.1}%
  Est. Full Time: {} hours
  Hot Tier: {:.1}%
  Cold Tier: {:.1}%

ISSUES FOUND: {}
"#,
            diag.timestamp.to_rfc3339(),
            diag.health,
            diag.performance.read_latency_ms,
            diag.performance.write_latency_ms,
            diag.performance.throughput_mbps,
            diag.performance.cache_efficiency,
            diag.reliability.disk_error_rate,
            diag.reliability.rebuild_success_rate,
            diag.reliability.scrub_issues_found,
            diag.reliability.mean_time_to_failure_hours,
            diag.capacity.used_percent,
            diag.capacity.estimated_remaining_hours,
            diag.capacity.hot_tier_usage,
            diag.capacity.cold_tier_usage,
            diag.issues.len()
        );

        for (idx, issue) in diag.issues.iter().enumerate() {
            output.push_str(&format!(
                "\n  Issue {}: {:?} - {}\n  {}\n  Recommendation: {}\n",
                idx + 1,
                issue.severity,
                issue.title,
                issue.description,
                issue.recommendation
            ));
        }

        output
    }

    pub fn format_json(diag: &Diagnostics) -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "timestamp": diag.timestamp.to_rfc3339(),
            "health": format!("{:?}", diag.health),
            "performance": {
                "read_latency_ms": diag.performance.read_latency_ms,
                "write_latency_ms": diag.performance.write_latency_ms,
                "throughput_mbps": diag.performance.throughput_mbps,
                "cache_efficiency": diag.performance.cache_efficiency,
            },
            "reliability": {
                "disk_error_rate": diag.reliability.disk_error_rate,
                "rebuild_success_rate": diag.reliability.rebuild_success_rate,
                "scrub_issues_found": diag.reliability.scrub_issues_found,
                "mttf_hours": diag.reliability.mean_time_to_failure_hours,
            },
            "capacity": {
                "used_percent": diag.capacity.used_percent,
                "remaining_hours": diag.capacity.estimated_remaining_hours,
            },
            "issues": diag.issues.len(),
        })).unwrap_or_else(|_| "{}".to_string())
    }
}

