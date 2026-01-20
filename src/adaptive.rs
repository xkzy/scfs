use crate::extent::{Extent, RedundancyPolicy};
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Detects sequential access patterns for read-ahead optimization
pub struct SequenceDetector {
    /// Max offset gap to consider sequential
    pub max_gap: u64,
    /// Recent access history
    history: Vec<(u64, u64)>, // (offset, timestamp)
    /// Maximum history size
    max_history: usize,
}

impl SequenceDetector {
    pub fn new(max_gap: u64, max_history: usize) -> Self {
        SequenceDetector {
            max_gap,
            history: Vec::new(),
            max_history,
        }
    }

    /// Record a read access at offset
    pub fn record_access(&mut self, offset: u64) {
        let now = current_timestamp();
        self.history.push((offset, now));
        if self.history.len() > self.max_history {
            self.history.remove(0);
        }
    }

    /// Check if recent accesses indicate sequential pattern
    pub fn is_sequential(&self) -> bool {
        if self.history.len() < 2 {
            return false;
        }

        // Check last few accesses for sequential pattern
        let window = self.history.len().min(5);
        for i in 1..window {
            let (prev_offset, _) = self.history[i - 1];
            let (curr_offset, _) = self.history[i];

            // Allow for some variance in sequential access
            let gap = if curr_offset > prev_offset {
                curr_offset - prev_offset
            } else {
                prev_offset - curr_offset
            };

            // If gaps are consistently small, it's sequential
            if gap > self.max_gap {
                return false;
            }
        }

        true
    }

    /// Get recommended read-ahead amount (in bytes)
    pub fn recommended_readahead(&self) -> u64 {
        if self.is_sequential() {
            64 * 1024 // 64KB read-ahead for sequential access
        } else {
            0 // No read-ahead for random access
        }
    }
}

/// Dynamically adjusts extent size based on file characteristics
pub struct DynamicExtentSizer {
    /// Minimum extent size
    pub min_size: usize,
    /// Maximum extent size
    pub max_size: usize,
    /// Current preferred size
    pub current_size: usize,
    /// Access pattern history
    pattern_history: VecDeque<AccessPattern>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessPattern {
    Sequential,
    Random,
    Sparse,
}

impl DynamicExtentSizer {
    pub fn new(min_size: usize, max_size: usize) -> Self {
        let current_size = (min_size + max_size) / 2;
        DynamicExtentSizer {
            min_size,
            max_size,
            current_size,
            pattern_history: VecDeque::new(),
        }
    }

    /// Record access pattern observation
    pub fn observe_pattern(&mut self, pattern: AccessPattern) {
        self.pattern_history.push_back(pattern);
        if self.pattern_history.len() > 10 {
            self.pattern_history.pop_front();
        }

        // Adjust extent size based on dominant pattern
        self.update_size();
    }

    /// Get recommended extent size for new extents
    pub fn recommended_size(&self) -> usize {
        self.current_size
    }

    /// Determine optimal redundancy for extent size
    pub fn redundancy_for_size(&self, file_size: usize) -> RedundancyPolicy {
        let extent_count = (file_size + self.current_size - 1) / self.current_size;

        // Small files: use 3x replication for lower latency
        if extent_count <= 2 {
            return RedundancyPolicy::Replication { copies: 3 };
        }

        // Medium files: use 4+2 EC for balance
        if extent_count <= 10 {
            return RedundancyPolicy::ErasureCoding {
                data_shards: 4,
                parity_shards: 2,
            };
        }

        // Large files: use 6+3 EC for space efficiency
        RedundancyPolicy::ErasureCoding {
            data_shards: 6,
            parity_shards: 3,
        }
    }

    fn update_size(&mut self) {
        if self.pattern_history.is_empty() {
            return;
        }

        // Count pattern occurrences
        let sequential_count = self.pattern_history.iter().filter(|p| **p == AccessPattern::Sequential).count();
        let sparse_count = self.pattern_history.iter().filter(|p| **p == AccessPattern::Sparse).count();

        // Adjust extent size based on dominant pattern
        if sequential_count > sparse_count {
            // Sequential: increase extent size for better throughput
            self.current_size = ((self.current_size as u64 * 3 / 2).min(self.max_size as u64)) as usize;
        } else if sparse_count > sequential_count * 2 {
            // Sparse: decrease extent size to reduce waste
            self.current_size = ((self.current_size as u64 * 2 / 3).max(self.min_size as u64)) as usize;
        }
    }
}

/// Workload-aware cache manager that learns access patterns
pub struct WorkloadCache {
    /// Extent access frequency tracking
    access_counts: std::collections::HashMap<Uuid, u64>,
    /// Hot extent candidates
    hot_extents: VecDeque<Uuid>,
    /// Cold extent candidates
    cold_extents: VecDeque<Uuid>,
    /// Maximum hot/cold tracking
    max_tracked: usize,
}

impl WorkloadCache {
    pub fn new(max_tracked: usize) -> Self {
        WorkloadCache {
            access_counts: std::collections::HashMap::new(),
            hot_extents: VecDeque::new(),
            cold_extents: VecDeque::new(),
            max_tracked,
        }
    }

    /// Record extent access
    pub fn record_access(&mut self, extent_uuid: Uuid) {
        *self.access_counts.entry(extent_uuid).or_insert(0) += 1;
    }

    /// Update hot/cold classifications
    pub fn update_classifications(&mut self) {
        if self.access_counts.is_empty() {
            return;
        }

        // Find median access count
        let mut counts: Vec<u64> = self.access_counts.values().copied().collect();
        counts.sort();
        let median = counts[counts.len() / 2];

        // Clear old classifications
        self.hot_extents.clear();
        self.cold_extents.clear();

        // Reclassify
        for (uuid, count) in &self.access_counts {
            if *count > median * 2 {
                self.hot_extents.push_back(*uuid);
            } else if *count < median / 2 {
                self.cold_extents.push_back(*uuid);
            }

            // Keep size limited
            if self.hot_extents.len() > self.max_tracked {
                self.hot_extents.pop_front();
            }
            if self.cold_extents.len() > self.max_tracked {
                self.cold_extents.pop_front();
            }
        }
    }

    /// Check if extent is hot
    pub fn is_hot(&self, extent_uuid: &Uuid) -> bool {
        self.hot_extents.contains(extent_uuid)
    }

    /// Check if extent is cold
    pub fn is_cold(&self, extent_uuid: &Uuid) -> bool {
        self.cold_extents.contains(extent_uuid)
    }

    /// Get access frequency for extent
    pub fn access_frequency(&self, extent_uuid: &Uuid) -> u64 {
        self.access_counts.get(extent_uuid).copied().unwrap_or(0)
    }
}

/// Adaptive tuning engine combining all optimization strategies
pub struct AdaptiveEngine {
    pub sequence_detector: SequenceDetector,
    pub extent_sizer: DynamicExtentSizer,
    pub workload_cache: WorkloadCache,
}

impl AdaptiveEngine {
    pub fn new(default_extent_size: usize) -> Self {
        AdaptiveEngine {
            sequence_detector: SequenceDetector::new(8192, 20),
            extent_sizer: DynamicExtentSizer::new(default_extent_size / 2, default_extent_size * 2),
            workload_cache: WorkloadCache::new(100),
        }
    }

    /// Record extent access for learning
    pub fn record_extent_access(&mut self, extent_uuid: Uuid, offset: u64) {
        self.sequence_detector.record_access(offset);
        self.workload_cache.record_access(extent_uuid);
    }

    /// Get redundancy recommendation for new extent
    pub fn recommend_redundancy(&self, file_size: usize) -> RedundancyPolicy {
        self.extent_sizer.redundancy_for_size(file_size)
    }

    /// Update workload classification (call periodically)
    pub fn update_classifications(&mut self) {
        self.workload_cache.update_classifications();
    }
}

/// Helper to get current timestamp
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequence_detector() {
        let mut detector = SequenceDetector::new(1024, 10);
        
        // Record sequential accesses
        detector.record_access(0);
        detector.record_access(512);
        detector.record_access(1024);
        
        assert!(detector.is_sequential());
        assert!(detector.recommended_readahead() > 0);
    }

    #[test]
    fn test_dynamic_extent_sizer() {
        let mut sizer = DynamicExtentSizer::new(4096, 65536);
        let initial_size = sizer.recommended_size();
        
        // Record sequential pattern
        for _ in 0..7 {
            sizer.observe_pattern(AccessPattern::Sequential);
        }
        
        // Size should increase for sequential
        let new_size = sizer.recommended_size();
        assert!(new_size >= initial_size);
    }

    #[test]
    fn test_workload_cache() {
        let mut cache = WorkloadCache::new(10);
        let uuid1 = Uuid::new_v4();
        let uuid2 = Uuid::new_v4();
        let uuid3 = Uuid::new_v4();
        
        // Access uuid1 many times (hot)
        for _ in 0..20 {
            cache.record_access(uuid1);
        }
        
        // Access uuid2 medium times (medium)
        for _ in 0..5 {
            cache.record_access(uuid2);
        }
        
        // Access uuid3 few times (cold)
        cache.record_access(uuid3);
        
        cache.update_classifications();
        
        // With counts [1, 5, 20], median is 5
        // uuid1 (20) > 5*2 (10) = true -> hot
        // uuid3 (1) < 5/2 (2) = true -> cold
        assert!(cache.is_hot(&uuid1), "Expected uuid1 to be hot");
        assert!(cache.is_cold(&uuid3), "Expected uuid3 to be cold");
    }
}
