use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::defrag::{DefragConfig, DefragIntensity, DefragmentationEngine};
use crate::metrics::Metrics;
use crate::storage::StorageEngine;
use crate::tiering::StorageTier;
use crate::trim::{TrimConfig, TrimEngine, TrimIntensity};

/// Space reclamation policy presets
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReclamationPolicy {
    /// Maximize space reclamation, defrag all extents
    Aggressive,
    /// Defrag hot tier, regular TRIM
    Balanced,
    /// Only TRIM, no defrag on writes
    Conservative,
    /// Minimal TRIM, no defrag, prioritize performance
    Performance,
    /// Custom policy defined by user
    Custom,
}

impl ReclamationPolicy {
    pub fn description(&self) -> &str {
        match self {
            ReclamationPolicy::Aggressive => {
                "Maximize space: Heavy defrag + aggressive TRIM"
            }
            ReclamationPolicy::Balanced => {
                "Balanced: Defrag hot tier + regular TRIM"
            }
            ReclamationPolicy::Conservative => {
                "Space-focused: TRIM only, minimal defrag"
            }
            ReclamationPolicy::Performance => {
                "Performance-focused: Minimal maintenance"
            }
            ReclamationPolicy::Custom => {
                "User-defined custom policy"
            }
        }
    }

    pub fn defrag_config(&self) -> DefragConfig {
        match self {
            ReclamationPolicy::Aggressive => DefragConfig {
                enabled: true,
                intensity: DefragIntensity::High,
                fragmentation_threshold: 0.15,
                min_extent_fragments: 2,
                prioritize_hot_extents: false, // Defrag everything
                pause_on_high_load: false,
                max_concurrent_operations: 4,
            },
            ReclamationPolicy::Balanced => DefragConfig {
                enabled: true,
                intensity: DefragIntensity::Medium,
                fragmentation_threshold: 0.30,
                min_extent_fragments: 3,
                prioritize_hot_extents: true,
                pause_on_high_load: true,
                max_concurrent_operations: 2,
            },
            ReclamationPolicy::Conservative => DefragConfig {
                enabled: true,
                intensity: DefragIntensity::Low,
                fragmentation_threshold: 0.50,
                min_extent_fragments: 5,
                prioritize_hot_extents: false,
                pause_on_high_load: true,
                max_concurrent_operations: 1,
            },
            ReclamationPolicy::Performance => DefragConfig {
                enabled: false,
                intensity: DefragIntensity::Low,
                fragmentation_threshold: 0.80,
                min_extent_fragments: 10,
                prioritize_hot_extents: true,
                pause_on_high_load: true,
                max_concurrent_operations: 1,
            },
            ReclamationPolicy::Custom => DefragConfig::default(),
        }
    }

    pub fn trim_config(&self) -> TrimConfig {
        match self {
            ReclamationPolicy::Aggressive => TrimConfig {
                enabled: true,
                intensity: TrimIntensity::Aggressive,
                batch_size_mb: 500,
                secure_erase: false,
                discard_granularity: 4096,
            },
            ReclamationPolicy::Balanced => TrimConfig {
                enabled: true,
                intensity: TrimIntensity::Balanced,
                batch_size_mb: 200,
                secure_erase: false,
                discard_granularity: 4096,
            },
            ReclamationPolicy::Conservative => TrimConfig {
                enabled: true,
                intensity: TrimIntensity::Balanced,
                batch_size_mb: 100,
                secure_erase: false,
                discard_granularity: 4096,
            },
            ReclamationPolicy::Performance => TrimConfig {
                enabled: true,
                intensity: TrimIntensity::Conservative,
                batch_size_mb: 50,
                secure_erase: false,
                discard_granularity: 4096,
            },
            ReclamationPolicy::Custom => TrimConfig::default(),
        }
    }
}

/// Per-tier reclamation policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierPolicy {
    pub tier: StorageTier,
    pub defrag_enabled: bool,
    pub defrag_intensity: DefragIntensity,
    pub trim_intensity: TrimIntensity,
    pub capacity_threshold_percent: u8,
    pub fragmentation_threshold: f64,
}

impl TierPolicy {
    pub fn for_tier(tier: StorageTier, base_policy: ReclamationPolicy) -> Self {
        let defrag_cfg = base_policy.defrag_config();
        let trim_cfg = base_policy.trim_config();

        match tier {
            StorageTier::Hot => TierPolicy {
                tier,
                defrag_enabled: true,
                defrag_intensity: DefragIntensity::Medium,
                trim_intensity: TrimIntensity::Conservative, // Minimize disruption
                capacity_threshold_percent: 90,
                fragmentation_threshold: 0.25,
            },
            StorageTier::Warm => TierPolicy {
                tier,
                defrag_enabled: defrag_cfg.enabled,
                defrag_intensity: defrag_cfg.intensity,
                trim_intensity: trim_cfg.intensity,
                capacity_threshold_percent: 85,
                fragmentation_threshold: defrag_cfg.fragmentation_threshold,
            },
            StorageTier::Cold => TierPolicy {
                tier,
                defrag_enabled: false, // Cold data rarely accessed
                defrag_intensity: DefragIntensity::Low,
                trim_intensity: TrimIntensity::Aggressive, // Reclaim space aggressively
                capacity_threshold_percent: 80,
                fragmentation_threshold: 0.60,
            },
        }
    }
}

/// Reclamation triggers
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReclamationTrigger {
    /// Trigger on capacity threshold
    Capacity,
    /// Trigger on fragmentation level
    Fragmentation,
    /// Time-based trigger (scheduled maintenance)
    Scheduled,
    /// Manual trigger
    Manual,
}

/// Reclamation event for tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReclamationEvent {
    pub trigger: ReclamationTrigger,
    pub timestamp: i64,
    pub space_reclaimed_bytes: u64,
    pub extents_defragmented: u64,
    pub duration_secs: u64,
}

/// Policy engine configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEngineConfig {
    pub enabled: bool,
    pub policy: ReclamationPolicy,
    pub per_tier_policies: Vec<TierPolicy>,
    pub capacity_threshold_percent: u8,
    pub fragmentation_threshold: f64,
    pub schedule_interval_hours: u64,
    pub adaptive_mode: bool,
}

impl Default for PolicyEngineConfig {
    fn default() -> Self {
        PolicyEngineConfig {
            enabled: true,
            policy: ReclamationPolicy::Balanced,
            per_tier_policies: vec![
                TierPolicy::for_tier(StorageTier::Hot, ReclamationPolicy::Balanced),
                TierPolicy::for_tier(StorageTier::Warm, ReclamationPolicy::Balanced),
                TierPolicy::for_tier(StorageTier::Cold, ReclamationPolicy::Balanced),
            ],
            capacity_threshold_percent: 85,
            fragmentation_threshold: 0.30,
            schedule_interval_hours: 24,
            adaptive_mode: false,
        }
    }
}

/// Space reclamation policy engine
pub struct ReclamationPolicyEngine {
    config: Arc<Mutex<PolicyEngineConfig>>,
    running: Arc<AtomicBool>,
    
    // Statistics
    total_reclamations: Arc<AtomicU64>,
    total_space_reclaimed: Arc<AtomicU64>,
    total_extents_defragmented: Arc<AtomicU64>,
    
    // Event history
    events: Arc<Mutex<Vec<ReclamationEvent>>>,
}

impl ReclamationPolicyEngine {
    pub fn new(config: PolicyEngineConfig) -> Self {
        ReclamationPolicyEngine {
            config: Arc::new(Mutex::new(config)),
            running: Arc::new(AtomicBool::new(false)),
            total_reclamations: Arc::new(AtomicU64::new(0)),
            total_space_reclaimed: Arc::new(AtomicU64::new(0)),
            total_extents_defragmented: Arc::new(AtomicU64::new(0)),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Start the policy engine
    pub fn start(
        &self,
        storage: Arc<StorageEngine>,
        defrag_engine: Arc<DefragmentationEngine>,
        trim_engine: Arc<TrimEngine>,
        metrics: Arc<Metrics>,
    ) -> Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Ok(()); // Already running
        }

        self.running.store(true, Ordering::SeqCst);

        let running = Arc::clone(&self.running);
        let config = Arc::clone(&self.config);
        let total_reclamations = Arc::clone(&self.total_reclamations);
        let total_space_reclaimed = Arc::clone(&self.total_space_reclaimed);
        let total_extents_defragmented = Arc::clone(&self.total_extents_defragmented);
        let events = Arc::clone(&self.events);

        std::thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                let cfg = config.lock().unwrap().clone();
                if !cfg.enabled {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    continue;
                }

                // Check if we should trigger reclamation
                if let Ok(should_trigger) = Self::check_triggers(&storage, &cfg) {
                    if let Some(trigger) = should_trigger {
                        let start_time = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs();

                        // Execute reclamation
                        match Self::execute_reclamation(
                            &storage,
                            &defrag_engine,
                            &trim_engine,
                            &metrics,
                            trigger,
                        ) {
                            Ok(event) => {
                                total_reclamations.fetch_add(1, Ordering::SeqCst);
                                total_space_reclaimed.fetch_add(
                                    event.space_reclaimed_bytes,
                                    Ordering::SeqCst,
                                );
                                total_extents_defragmented.fetch_add(
                                    event.extents_defragmented,
                                    Ordering::SeqCst,
                                );

                                events.lock().unwrap().push(event);
                            }
                            Err(e) => {
                                eprintln!("Reclamation failed: {}", e);
                            }
                        }
                    }
                }

                // Sleep until next check
                let interval = cfg.schedule_interval_hours * 3600;
                std::thread::sleep(std::time::Duration::from_secs(interval));
            }
        });

        Ok(())
    }

    /// Stop the policy engine
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Change the active policy
    pub fn set_policy(&self, policy: ReclamationPolicy) {
        let mut config = self.config.lock().unwrap();
        config.policy = policy;
        
        // Update per-tier policies based on new preset
        config.per_tier_policies = vec![
            TierPolicy::for_tier(StorageTier::Hot, policy),
            TierPolicy::for_tier(StorageTier::Warm, policy),
            TierPolicy::for_tier(StorageTier::Cold, policy),
        ];
    }

    /// Get current configuration
    pub fn config(&self) -> PolicyEngineConfig {
        self.config.lock().unwrap().clone()
    }

    /// Get statistics
    pub fn stats(&self) -> PolicyEngineStats {
        PolicyEngineStats {
            total_reclamations: self.total_reclamations.load(Ordering::SeqCst),
            total_space_reclaimed: self.total_space_reclaimed.load(Ordering::SeqCst),
            total_extents_defragmented: self
                .total_extents_defragmented
                .load(Ordering::SeqCst),
            recent_events: self.events.lock().unwrap().clone(),
        }
    }

    /// Check if any triggers are met
    fn check_triggers(
        storage: &StorageEngine,
        config: &PolicyEngineConfig,
    ) -> Result<Option<ReclamationTrigger>> {
        // TODO: Complete implementation of fragmentation trigger
        // This currently only checks capacity. For production, also implement:
        // 1. Call defrag_engine.analyze_fragmentation() for fragmentation check
        // 2. Check time-based schedule against last_run timestamp
        // 3. Implement manual trigger mechanism
        
        // Check capacity trigger
        let disks = storage.get_disks();
        let total_capacity: u64 = disks.iter().map(|d| d.capacity_bytes).sum();
        let total_used: u64 = disks.iter().map(|d| d.used_bytes).sum();
        let usage_percent = if total_capacity > 0 {
            (total_used as f64 / total_capacity as f64 * 100.0) as u8
        } else {
            0
        };

        if usage_percent >= config.capacity_threshold_percent {
            return Ok(Some(ReclamationTrigger::Capacity));
        }

        // Fragmentation trigger (simplified - would need defrag_engine integration)
        // In full implementation, call defrag_engine.analyze_fragmentation()

        Ok(None)
    }

    /// Execute space reclamation
    fn execute_reclamation(
        storage: &StorageEngine,
        defrag_engine: &DefragmentationEngine,
        trim_engine: &TrimEngine,
        metrics: &Metrics,
        trigger: ReclamationTrigger,
    ) -> Result<ReclamationEvent> {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Get initial stats
        let defrag_status = defrag_engine.status();
        let trim_stats = trim_engine.stats();
        
        let initial_defragmented = defrag_status.extents_defragmented;
        let initial_trimmed = trim_stats.total_bytes_trimmed;

        // Execute TRIM operations
        let disks = storage.get_disks();
        trim_engine.execute_all_trims(&disks, metrics)?;

        // Get final stats
        let final_defrag_status = defrag_engine.status();
        let final_trim_stats = trim_engine.stats();

        let end_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Ok(ReclamationEvent {
            trigger,
            timestamp: start_time as i64,
            space_reclaimed_bytes: final_trim_stats.total_bytes_trimmed - initial_trimmed,
            extents_defragmented: final_defrag_status.extents_defragmented
                - initial_defragmented,
            duration_secs: end_time - start_time,
        })
    }
}

/// Statistics for the policy engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEngineStats {
    pub total_reclamations: u64,
    pub total_space_reclaimed: u64,
    pub total_extents_defragmented: u64,
    pub recent_events: Vec<ReclamationEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_presets() {
        let aggressive = ReclamationPolicy::Aggressive;
        let defrag = aggressive.defrag_config();
        assert!(defrag.enabled);
        assert_eq!(defrag.intensity, DefragIntensity::High);

        let performance = ReclamationPolicy::Performance;
        let perf_defrag = performance.defrag_config();
        assert!(!perf_defrag.enabled);
    }

    #[test]
    fn test_tier_policies() {
        let hot_policy = TierPolicy::for_tier(StorageTier::Hot, ReclamationPolicy::Balanced);
        assert_eq!(hot_policy.tier, StorageTier::Hot);
        assert!(hot_policy.defrag_enabled);

        let cold_policy = TierPolicy::for_tier(StorageTier::Cold, ReclamationPolicy::Balanced);
        assert_eq!(cold_policy.tier, StorageTier::Cold);
        assert!(!cold_policy.defrag_enabled); // Cold tier doesn't defrag
    }

    #[test]
    fn test_policy_descriptions() {
        assert!(!ReclamationPolicy::Aggressive.description().is_empty());
        assert!(!ReclamationPolicy::Balanced.description().is_empty());
        assert!(!ReclamationPolicy::Conservative.description().is_empty());
        assert!(!ReclamationPolicy::Performance.description().is_empty());
    }
}
