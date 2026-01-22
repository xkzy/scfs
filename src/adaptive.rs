//! Adaptive tuning utilities for SCFS.
//!
//! This module implements lightweight heuristics and data structures used to
//! adapt filesystem behavior at runtime. It includes:
//! - `SequenceDetector` — detects sequential read patterns to drive read-ahead.
//! - `DynamicExtentSizer` — adjusts preferred extent sizes based on observed
//!   access patterns.
//! - `WorkloadCache` — classifies extents as hot or cold using simple counters.
//! - `AdaptiveEngine` — combines components into a single runtime engine.
//!
//! # Examples
//! ```rust
//! # use crate::adaptive::SequenceDetector;
//! let mut det = SequenceDetector::new(8 * 1024, 20);
//! det.record_access(0);
//! det.record_access(4096);
//! assert!(det.is_sequential());
//! ```
//!
//! Module docs are intentionally concise; prefer unit tests for behavioral guarantees.

use crate::extent::RedundancyPolicy;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Detects sequential access patterns for read-ahead optimization.
///
/// This lightweight detector records recent read offsets (with epoch seconds)
/// and determines whether recent accesses form a sequential pattern suitable
/// for read-ahead. The detector uses a conservative `max_gap` to tolerate
/// small variances between successive accesses.
///
/// # Example
/// ```rust
/// # use crate::adaptive::SequenceDetector;
/// let mut det = SequenceDetector::new(8192, 20);
/// det.record_access(0);
/// det.record_access(4096);
/// assert!(det.is_sequential());
/// ```
pub struct SequenceDetector {
    /// Max offset gap to consider sequential (bytes).
    pub max_gap: u64,
    /// Recent access history: (offset, timestamp) where timestamp is seconds since UNIX_EPOCH.
    history: Vec<(u64, u64)>, // (offset, timestamp)
    /// Maximum history size.
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
/// Observed access pattern categories used by `DynamicExtentSizer`.
pub enum AccessPattern {
    /// Sequential access (good for larger extents).
    Sequential,
    /// Random access (no strong size preference).
    Random,
    /// Sparse access (good for smaller extents).
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

/// Adaptive tuning engine combining the detector, sizer and cache components.
///
/// This convenience wrapper bundles the adaptive subsystems and exposes a
/// small surface for recording activity and collecting recommendations that
/// higher-level code (e.g., the IO path or background daemons) can use.
///
/// # Example
/// ```rust
/// # use crate::adaptive::AdaptiveEngine;
/// # use uuid::Uuid;
/// let mut engine = AdaptiveEngine::new(65536);
/// let id = Uuid::new_v4();
/// engine.record_extent_access(id, 0);
/// engine.update_classifications();
/// let policy = engine.recommend_redundancy(1024);
/// ```
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

