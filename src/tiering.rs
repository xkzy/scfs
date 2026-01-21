use uuid::Uuid;
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Serialize, Deserialize};
use crate::extent::{Extent, AccessClassification};

/// Storage tier definition
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StorageTier {
    Hot,      // Fast, expensive, local NVMe
    Warm,     // Medium speed, local HDD
    Cold,     // Slow, cheap, archive storage
}

impl StorageTier {
    pub fn description(&self) -> &'static str {
        match self {
            StorageTier::Hot => "Fast local NVMe storage for active data",
            StorageTier::Warm => "Medium-speed local HDD storage for warm data",
            StorageTier::Cold => "Slow archive storage for cold/historical data",
        }
    }

    pub fn latency_ms(&self) -> u32 {
        match self {
            StorageTier::Hot => 1,    // < 1ms
            StorageTier::Warm => 10,  // ~10ms
            StorageTier::Cold => 100, // ~100ms
        }
    }

    pub fn cost_per_gb(&self) -> f64 {
        match self {
            StorageTier::Hot => 0.10,  // $0.10/GB
            StorageTier::Warm => 0.02, // $0.02/GB
            StorageTier::Cold => 0.005, // $0.005/GB
        }
    }
}

/// Tiering policy definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TieringPolicy {
    pub name: String,
    pub description: String,
    pub hot_threshold_days: u32,
    pub cold_threshold_days: u32,
    pub max_hot_percent: u32,    // Max % of capacity in hot tier
    pub max_warm_percent: u32,   // Max % of capacity in warm tier
    pub enabled: bool,
}

impl TieringPolicy {
    /// Default aggressive tiering (good for cost)
    pub fn aggressive() -> Self {
        TieringPolicy {
            name: "aggressive".to_string(),
            description: "Aggressive tiering: minimize cost".to_string(),
            hot_threshold_days: 1,     // Only 1 day in hot
            cold_threshold_days: 7,    // 7 days until cold
            max_hot_percent: 20,
            max_warm_percent: 40,
            enabled: true,
        }
    }

    /// Default balanced tiering
    pub fn balanced() -> Self {
        TieringPolicy {
            name: "balanced".to_string(),
            description: "Balanced tiering: cost vs performance".to_string(),
            hot_threshold_days: 7,     // 1 week in hot
            cold_threshold_days: 30,   // 1 month until cold
            max_hot_percent: 40,
            max_warm_percent: 50,
            enabled: true,
        }
    }

    /// Default performance-focused tiering
    pub fn performance() -> Self {
        TieringPolicy {
            name: "performance".to_string(),
            description: "Performance tiering: maximize speed".to_string(),
            hot_threshold_days: 30,    // 1 month in hot
            cold_threshold_days: 90,   // 3 months until cold
            max_hot_percent: 70,
            max_warm_percent: 80,
            enabled: true,
        }
    }

    /// Determine target tier for extent based on access patterns
    pub fn target_tier(&self, extent: &Extent) -> StorageTier {
        let now = current_timestamp() as i64;
        let days_since_access = (now - extent.access_stats.last_read) / 86400;

        // Use classification if available, otherwise use access recency
        match extent.access_stats.classification {
            AccessClassification::Hot => StorageTier::Hot,
            AccessClassification::Warm => {
                if days_since_access < self.cold_threshold_days as i64 {
                    StorageTier::Warm
                } else {
                    StorageTier::Cold
                }
            }
            AccessClassification::Cold => {
                if days_since_access < self.cold_threshold_days as i64 {
                    StorageTier::Warm
                } else {
                    StorageTier::Cold
                }
            }
        }
    }
}

/// Tiering recommendation with reasoning
#[derive(Debug, Clone)]
pub struct TieringRecommendation {
    pub extent_uuid: Uuid,
    pub current_tier: StorageTier,
    pub recommended_tier: StorageTier,
    pub reason: String,
    pub priority: u32, // 1-10, higher = more urgent
}

/// Tiering analyzer for policy-driven decisions
pub struct TieringAnalyzer {
    pub policy: TieringPolicy,
    pub current_tier_distribution: TierDistribution,
}

#[derive(Debug, Clone, Default)]
pub struct TierDistribution {
    pub hot_extents: u64,
    pub warm_extents: u64,
    pub cold_extents: u64,
    pub hot_bytes: u64,
    pub warm_bytes: u64,
    pub cold_bytes: u64,
}

impl TierDistribution {
    pub fn total_extents(&self) -> u64 {
        self.hot_extents + self.warm_extents + self.cold_extents
    }

    pub fn total_bytes(&self) -> u64 {
        self.hot_bytes + self.warm_bytes + self.cold_bytes
    }

    pub fn hot_percent(&self) -> f64 {
        let total = self.total_bytes();
        if total == 0 {
            0.0
        } else {
            (self.hot_bytes as f64 / total as f64) * 100.0
        }
    }

    pub fn warm_percent(&self) -> f64 {
        let total = self.total_bytes();
        if total == 0 {
            0.0
        } else {
            (self.warm_bytes as f64 / total as f64) * 100.0
        }
    }

    pub fn cost_estimate(&self) -> f64 {
        let hot_cost = self.hot_bytes as f64 / 1_000_000_000.0 * StorageTier::Hot.cost_per_gb();
        let warm_cost = self.warm_bytes as f64 / 1_000_000_000.0 * StorageTier::Warm.cost_per_gb();
        let cold_cost = self.cold_bytes as f64 / 1_000_000_000.0 * StorageTier::Cold.cost_per_gb();
        hot_cost + warm_cost + cold_cost
    }
}

impl TieringAnalyzer {
    pub fn new(policy: TieringPolicy) -> Self {
        TieringAnalyzer {
            policy,
            current_tier_distribution: TierDistribution::default(),
        }
    }

    /// Analyze extents and generate tiering recommendations
    pub fn analyze_extents(&mut self, extents: &[Extent]) -> Vec<TieringRecommendation> {
        let mut recommendations = Vec::new();
        
        // Reset distribution tracking
        self.current_tier_distribution = TierDistribution::default();

        // Build current distribution and recommendations
        for extent in extents {
            let target_tier = self.policy.target_tier(extent);
            let current_tier = self.get_current_tier(extent);

            // Track current distribution
            match current_tier {
                StorageTier::Hot => {
                    self.current_tier_distribution.hot_extents += 1;
                    self.current_tier_distribution.hot_bytes += extent.size as u64;
                }
                StorageTier::Warm => {
                    self.current_tier_distribution.warm_extents += 1;
                    self.current_tier_distribution.warm_bytes += extent.size as u64;
                }
                StorageTier::Cold => {
                    self.current_tier_distribution.cold_extents += 1;
                    self.current_tier_distribution.cold_bytes += extent.size as u64;
                }
            }

            if current_tier != target_tier {
                let reason = format!(
                    "Classification: {:?}, Moves {} -> {}",
                    extent.access_stats.classification, current_tier, target_tier
                );

                let priority = if extent.access_stats.read_count > 100 {
                    10 // High priority for hot data
                } else if extent.access_stats.read_count == 0 {
                    3  // Low priority for never-accessed data
                } else {
                    6  // Medium priority for accessed data
                };

                recommendations.push(TieringRecommendation {
                    extent_uuid: extent.uuid,
                    current_tier,
                    recommended_tier: target_tier,
                    reason,
                    priority,
                });
            }
        }

        // Sort by priority (highest first)
        recommendations.sort_by(|a, b| b.priority.cmp(&a.priority));
        recommendations
    }

    /// Check if tiering policy is respected
    pub fn check_policy_compliance(&self) -> Vec<PolicyViolation> {
        let mut violations = Vec::new();

        if self.current_tier_distribution.hot_percent() as u32 > self.policy.max_hot_percent {
            violations.push(PolicyViolation {
                tier: StorageTier::Hot,
                current_percent: self.current_tier_distribution.hot_percent() as u32,
                max_percent: self.policy.max_hot_percent,
                message: format!(
                    "Hot tier at {}%, exceeds limit of {}%",
                    self.current_tier_distribution.hot_percent() as u32,
                    self.policy.max_hot_percent
                ),
            });
        }

        if self.current_tier_distribution.warm_percent() as u32 > self.policy.max_warm_percent {
            violations.push(PolicyViolation {
                tier: StorageTier::Warm,
                current_percent: self.current_tier_distribution.warm_percent() as u32,
                max_percent: self.policy.max_warm_percent,
                message: format!(
                    "Warm tier at {}%, exceeds limit of {}%",
                    self.current_tier_distribution.warm_percent() as u32,
                    self.policy.max_warm_percent
                ),
            });
        }

        violations
    }

    fn get_current_tier(&self, _extent: &Extent) -> StorageTier {
        // In real implementation, would check actual placement
        // For now, assume everything starts in warm
        StorageTier::Warm
    }
}

#[derive(Debug, Clone)]
pub struct PolicyViolation {
    pub tier: StorageTier,
    pub current_percent: u32,
    pub max_percent: u32,
    pub message: String,
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl std::fmt::Display for StorageTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageTier::Hot => write!(f, "Hot"),
            StorageTier::Warm => write!(f, "Warm"),
            StorageTier::Cold => write!(f, "Cold"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extent::{AccessStats, RedundancyPolicy, FragmentLocation};

    fn create_test_extent(size: usize, classification: AccessClassification) -> Extent {
        Extent {
            uuid: Uuid::new_v4(),
            size,
            checksum: [0; 32],
            redundancy: RedundancyPolicy::Replication { copies: 3 },
            fragment_locations: vec![
                FragmentLocation {
                    disk_uuid: Uuid::new_v4(),
                    fragment_index: 0,
                    on_device: None,
                },
            ],
            previous_policy: None,
            policy_transitions: Vec::new(),
            last_policy_change: None,
            access_stats: AccessStats {
                read_count: 10,
                write_count: 1,
                last_read: current_timestamp() as i64,
                last_write: current_timestamp() as i64,
                created_at: (current_timestamp() - 86400 * 30) as i64, // 30 days old
                classification,
                hmm_classifier: None,
            },
            rebuild_in_progress: false,
            rebuild_progress: None,
            generation: 0,
        }
    }

    #[test]
    fn test_tiering_policies() {
        let aggressive = TieringPolicy::aggressive();
        assert!(aggressive.hot_threshold_days < 7);

        let balanced = TieringPolicy::balanced();
        assert_eq!(balanced.hot_threshold_days, 7);

        let performance = TieringPolicy::performance();
        assert!(performance.hot_threshold_days > 7);
    }

    #[test]
    fn test_tiering_recommendation() {
        let policy = TieringPolicy::balanced();
        let extent = create_test_extent(1024, AccessClassification::Hot);
        
        let target = policy.target_tier(&extent);
        assert_eq!(target, StorageTier::Hot);
    }

    #[test]
    fn test_tiering_analyzer() {
        let policy = TieringPolicy::balanced();
        let mut analyzer = TieringAnalyzer::new(policy);

        let hot_extent = create_test_extent(1024, AccessClassification::Hot);
        let cold_extent = create_test_extent(512, AccessClassification::Cold);
        let extents = vec![hot_extent, cold_extent];

        let recommendations = analyzer.analyze_extents(&extents);
        
        // Should have recommendations for movement
        assert!(!recommendations.is_empty());
        
        // Check distribution tracking
        assert!(analyzer.current_tier_distribution.total_extents() > 0);
    }

    #[test]
    fn test_tier_distribution() {
        let mut dist = TierDistribution::default();
        dist.hot_bytes = 100;
        dist.warm_bytes = 100;
        dist.cold_bytes = 100;

        assert_eq!(dist.total_bytes(), 300);
        assert_eq!(dist.hot_percent() as u32, 33);
    }

    #[test]
    fn test_policy_violation_detection() {
        let mut policy = TieringPolicy::aggressive();
        policy.max_hot_percent = 10;

        let mut analyzer = TieringAnalyzer::new(policy);
        analyzer.current_tier_distribution.hot_bytes = 100;
        analyzer.current_tier_distribution.warm_bytes = 100;

        let violations = analyzer.check_policy_compliance();
        // 50% hot > 10% limit, so should detect violation
        assert!(!violations.is_empty());
    }
}
