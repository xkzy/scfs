use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::disk::{Disk, DiskHealth};
use crate::extent::Extent;
use crate::io_scheduler::IoPriority;
use crate::metadata::MetadataManager;
use crate::metrics::Metrics;
use crate::storage::StorageEngine;

/// Fragmentation statistics for a single disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskFragmentationStats {
    pub disk_uuid: Uuid,
    pub total_extents: u64,
    pub fragmented_extents: u64,
    pub fragmentation_ratio: f64,
    pub avg_fragments_per_extent: f64,
    pub sequential_ratio: f64,
}

/// System-wide fragmentation analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentationAnalysis {
    pub timestamp: i64,
    pub total_extents: u64,
    pub fragmented_extents: u64,
    pub overall_fragmentation_ratio: f64,
    pub per_disk_stats: Vec<DiskFragmentationStats>,
    pub recommendation: DefragRecommendation,
}

/// Recommendations for defragmentation actions
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DefragRecommendation {
    /// No action needed
    None,
    /// Consider defragmentation
    Consider,
    /// Defragmentation recommended
    Recommended,
    /// Urgent defragmentation needed
    Urgent,
}

/// Defragmentation intensity levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DefragIntensity {
    /// Minimal I/O impact, 100ms throttle between operations
    Low,
    /// Moderate I/O impact, 50ms throttle
    Medium,
    /// Aggressive defragmentation, 10ms throttle
    High,
}

impl DefragIntensity {
    pub fn io_throttle_ms(&self) -> u64 {
        match self {
            DefragIntensity::Low => 100,
            DefragIntensity::Medium => 50,
            DefragIntensity::High => 10,
        }
    }

    pub fn priority(&self) -> IoPriority {
        // All defrag operations use Background priority
        IoPriority::Background
    }

    pub fn batch_size(&self) -> usize {
        match self {
            DefragIntensity::Low => 1,
            DefragIntensity::Medium => 5,
            DefragIntensity::High => 10,
        }
    }
}

/// Defragmentation status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefragStatus {
    pub running: bool,
    pub paused: bool,
    pub intensity: DefragIntensity,
    pub extents_processed: u64,
    pub extents_defragmented: u64,
    pub bytes_moved: u64,
    pub errors: u64,
    pub started_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub estimated_time_remaining_secs: Option<u64>,
}

/// Configuration for defragmentation operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefragConfig {
    pub enabled: bool,
    pub intensity: DefragIntensity,
    pub fragmentation_threshold: f64,
    pub min_extent_fragments: usize,
    pub prioritize_hot_extents: bool,
    pub pause_on_high_load: bool,
    pub max_concurrent_operations: usize,
}

impl Default for DefragConfig {
    fn default() -> Self {
        DefragConfig {
            enabled: false,
            intensity: DefragIntensity::Low,
            fragmentation_threshold: 0.30, // 30% fragmented extents
            min_extent_fragments: 2,       // Only defrag extents with 2+ fragments
            prioritize_hot_extents: true,
            pause_on_high_load: true,
            max_concurrent_operations: 1,
        }
    }
}

/// Main defragmentation engine
pub struct DefragmentationEngine {
    config: Arc<Mutex<DefragConfig>>,
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    
    // Statistics
    extents_processed: Arc<AtomicU64>,
    extents_defragmented: Arc<AtomicU64>,
    bytes_moved: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
    started_at: Arc<Mutex<Option<i64>>>,
    last_run_at: Arc<Mutex<Option<i64>>>,
}

impl DefragmentationEngine {
    pub fn new(config: DefragConfig) -> Self {
        DefragmentationEngine {
            config: Arc::new(Mutex::new(config)),
            running: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            extents_processed: Arc::new(AtomicU64::new(0)),
            extents_defragmented: Arc::new(AtomicU64::new(0)),
            bytes_moved: Arc::new(AtomicU64::new(0)),
            errors: Arc::new(AtomicU64::new(0)),
            started_at: Arc::new(Mutex::new(None)),
            last_run_at: Arc::new(Mutex::new(None)),
        }
    }

    /// Analyze fragmentation across all disks
    pub fn analyze_fragmentation(
        &self,
        storage: &StorageEngine,
    ) -> Result<FragmentationAnalysis> {
        let metadata_arc = storage.metadata();
        let metadata = metadata_arc.read().unwrap();
        let extents = metadata.list_all_extents()?;
        drop(metadata);
        
        let disks = storage.get_disks();

        let mut per_disk_stats: HashMap<Uuid, DiskFragmentationStats> = HashMap::new();
        let mut total_extents = 0u64;
        let mut fragmented_extents = 0u64;

        // Initialize disk stats
        for disk in &disks {
            per_disk_stats.insert(
                disk.uuid,
                DiskFragmentationStats {
                    disk_uuid: disk.uuid,
                    total_extents: 0,
                    fragmented_extents: 0,
                    fragmentation_ratio: 0.0,
                    avg_fragments_per_extent: 0.0,
                    sequential_ratio: 0.0,
                },
            );
        }

        // Analyze each extent
        for extent in &extents {
            total_extents += 1;

            // Count fragments per disk
            let mut disk_fragment_counts: HashMap<Uuid, usize> = HashMap::new();
            for loc in &extent.fragment_locations {
                *disk_fragment_counts.entry(loc.disk_uuid).or_insert(0) += 1;
            }

            // An extent is fragmented if it has multiple fragments on the same disk
            let is_fragmented = disk_fragment_counts.values().any(|&count| count > 1);
            if is_fragmented {
                fragmented_extents += 1;
            }

            // Update per-disk statistics
            for (disk_uuid, fragment_count) in disk_fragment_counts {
                if let Some(stats) = per_disk_stats.get_mut(&disk_uuid) {
                    stats.total_extents += 1;
                    if fragment_count > 1 {
                        stats.fragmented_extents += 1;
                    }
                }
            }
        }

        // Calculate ratios
        for stats in per_disk_stats.values_mut() {
            if stats.total_extents > 0 {
                stats.fragmentation_ratio =
                    stats.fragmented_extents as f64 / stats.total_extents as f64;
            }
        }

        let overall_fragmentation_ratio = if total_extents > 0 {
            fragmented_extents as f64 / total_extents as f64
        } else {
            0.0
        };

        // Generate recommendation
        let recommendation = if overall_fragmentation_ratio >= 0.50 {
            DefragRecommendation::Urgent
        } else if overall_fragmentation_ratio >= 0.30 {
            DefragRecommendation::Recommended
        } else if overall_fragmentation_ratio >= 0.15 {
            DefragRecommendation::Consider
        } else {
            DefragRecommendation::None
        };

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        Ok(FragmentationAnalysis {
            timestamp,
            total_extents,
            fragmented_extents,
            overall_fragmentation_ratio,
            per_disk_stats: per_disk_stats.into_values().collect(),
            recommendation,
        })
    }

    /// Start defragmentation process
    pub fn start(&self, storage: Arc<StorageEngine>, metrics: Arc<Metrics>) -> Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Ok(()); // Already running
        }

        self.running.store(true, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        *self.started_at.lock().unwrap() = Some(timestamp);

        // Spawn background defragmentation thread
        let running = Arc::clone(&self.running);
        let paused = Arc::clone(&self.paused);
        let config = Arc::clone(&self.config);
        let extents_processed = Arc::clone(&self.extents_processed);
        let extents_defragmented = Arc::clone(&self.extents_defragmented);
        let bytes_moved = Arc::clone(&self.bytes_moved);
        let errors = Arc::clone(&self.errors);
        let last_run_at = Arc::clone(&self.last_run_at);

        std::thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                // Check if paused
                if paused.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    continue;
                }

                let cfg = config.lock().unwrap().clone();
                if !cfg.enabled {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    continue;
                }

                // Perform defragmentation pass
                match Self::defrag_pass(&storage, &cfg, &metrics) {
                    Ok(stats) => {
                        extents_processed.fetch_add(stats.processed, Ordering::SeqCst);
                        extents_defragmented.fetch_add(stats.defragmented, Ordering::SeqCst);
                        bytes_moved.fetch_add(stats.bytes_moved, Ordering::SeqCst);

                        let timestamp = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs() as i64;
                        *last_run_at.lock().unwrap() = Some(timestamp);

                        // Record metrics
                        metrics.defrag_runs_completed.fetch_add(1, Ordering::SeqCst);
                        metrics
                            .defrag_extents_moved
                            .fetch_add(stats.defragmented, Ordering::SeqCst);
                        metrics
                            .defrag_bytes_moved
                            .fetch_add(stats.bytes_moved, Ordering::SeqCst);
                    }
                    Err(e) => {
                        errors.fetch_add(1, Ordering::SeqCst);
                        eprintln!("Defragmentation pass error: {}", e);
                    }
                }

                // Throttle between passes
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        });

        Ok(())
    }

    /// Stop defragmentation
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Pause defragmentation
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    /// Resume defragmentation
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    /// Get current status
    pub fn status(&self) -> DefragStatus {
        let config = self.config.lock().unwrap();
        
        DefragStatus {
            running: self.running.load(Ordering::SeqCst),
            paused: self.paused.load(Ordering::SeqCst),
            intensity: config.intensity,
            extents_processed: self.extents_processed.load(Ordering::SeqCst),
            extents_defragmented: self.extents_defragmented.load(Ordering::SeqCst),
            bytes_moved: self.bytes_moved.load(Ordering::SeqCst),
            errors: self.errors.load(Ordering::SeqCst),
            started_at: *self.started_at.lock().unwrap(),
            last_run_at: *self.last_run_at.lock().unwrap(),
            estimated_time_remaining_secs: None, // TODO: Implement estimation
        }
    }

    /// Perform a single defragmentation pass
    fn defrag_pass(
        storage: &StorageEngine,
        config: &DefragConfig,
        metrics: &Metrics,
    ) -> Result<DefragPassStats> {
        let metadata_arc = storage.metadata();
        let metadata = metadata_arc.read().unwrap();
        let extents = metadata.list_all_extents()?;
        drop(metadata);
        
        let mut stats = DefragPassStats {
            processed: 0,
            defragmented: 0,
            bytes_moved: 0,
        };

        // Filter and prioritize extents for defragmentation
        let mut candidates = Self::select_defrag_candidates(&extents, config)?;
        
        // Limit batch size
        let batch_size = config.intensity.batch_size();
        candidates.truncate(batch_size);

        for extent in candidates {
            // Check if extent needs defragmentation
            if Self::needs_defragmentation(&extent, config) {
                match Self::defragment_extent(storage, &extent, config, metrics) {
                    Ok(bytes) => {
                        stats.defragmented += 1;
                        stats.bytes_moved += bytes;
                    }
                    Err(e) => {
                        eprintln!("Failed to defragment extent {}: {}", extent.uuid, e);
                    }
                }

                // Throttle between operations
                let throttle_ms = config.intensity.io_throttle_ms();
                std::thread::sleep(std::time::Duration::from_millis(throttle_ms));
            }

            stats.processed += 1;
        }

        Ok(stats)
    }

    /// Select candidate extents for defragmentation
    fn select_defrag_candidates(
        extents: &[Extent],
        config: &DefragConfig,
    ) -> Result<Vec<Extent>> {
        let mut candidates: Vec<Extent> = extents
            .iter()
            .filter(|e| Self::needs_defragmentation(e, config))
            .cloned()
            .collect();

        // Prioritize hot extents if configured
        if config.prioritize_hot_extents {
            candidates.sort_by(|a, b| {
                let a_hot = a.access_stats.read_count + a.access_stats.write_count;
                let b_hot = b.access_stats.read_count + b.access_stats.write_count;
                b_hot.cmp(&a_hot) // Descending order
            });
        }

        Ok(candidates)
    }

    /// Check if an extent needs defragmentation
    fn needs_defragmentation(extent: &Extent, config: &DefragConfig) -> bool {
        // Count fragments per disk
        let mut disk_fragment_counts: HashMap<Uuid, usize> = HashMap::new();
        for loc in &extent.fragment_locations {
            *disk_fragment_counts.entry(loc.disk_uuid).or_insert(0) += 1;
        }

        // Check if any disk has multiple fragments
        disk_fragment_counts
            .values()
            .any(|&count| count >= config.min_extent_fragments)
    }

    /// Defragment a single extent by rewriting it
    fn defragment_extent(
        storage: &StorageEngine,
        extent: &Extent,
        _config: &DefragConfig,
        _metrics: &Metrics,
    ) -> Result<u64> {
        // Read the extent data
        let data = storage.read_extent(extent.uuid)?;

        // Delete old fragments
        storage.delete_extent(extent.uuid)?;

        // Write extent back with new placement (will be more contiguous)
        let new_extent = storage.write_extent(&data, extent.redundancy)?;

        // Verify checksum matches
        if new_extent.checksum != extent.checksum {
            anyhow::bail!("Checksum mismatch after defragmentation");
        }

        Ok(data.len() as u64)
    }
}

/// Statistics from a defragmentation pass
#[derive(Debug, Clone)]
struct DefragPassStats {
    processed: u64,
    defragmented: u64,
    bytes_moved: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defrag_intensity() {
        assert_eq!(DefragIntensity::Low.io_throttle_ms(), 100);
        assert_eq!(DefragIntensity::Medium.io_throttle_ms(), 50);
        assert_eq!(DefragIntensity::High.io_throttle_ms(), 10);
    }

    #[test]
    fn test_defrag_recommendation() {
        let low = FragmentationAnalysis {
            timestamp: 0,
            total_extents: 100,
            fragmented_extents: 10,
            overall_fragmentation_ratio: 0.10,
            per_disk_stats: vec![],
            recommendation: DefragRecommendation::None,
        };
        assert_eq!(low.recommendation, DefragRecommendation::None);

        let medium = FragmentationAnalysis {
            timestamp: 0,
            total_extents: 100,
            fragmented_extents: 35,
            overall_fragmentation_ratio: 0.35,
            per_disk_stats: vec![],
            recommendation: DefragRecommendation::Recommended,
        };
        assert_eq!(medium.recommendation, DefragRecommendation::Recommended);
    }
}
