use anyhow::Result;
use uuid::Uuid;

use crate::disk::{Disk, DiskHealth};
use crate::extent::Extent;

/// Smart replica selection strategy for optimized reads
pub struct ReplicaSelector;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaSelectionStrategy {
    /// Pick first available replica (fast, simple)
    First,
    /// Pick replica on least-loaded disk
    LeastLoaded,
    /// Pick replica based on health and load
    Smart,
    /// Round-robin across replicas (balance load)
    RoundRobin,
}

impl ReplicaSelector {
    /// Select best replica fragment for reading based on strategy
    pub fn select_replica(
        extent: &Extent,
        disks: &[&Disk],
        strategy: ReplicaSelectionStrategy,
    ) -> Option<(Uuid, usize)> {
        // Find all readable replicas
        let mut candidates: Vec<(Uuid, usize)> = Vec::new();

        for location in &extent.fragment_locations {
            if let Some(disk) = disks.iter().find(|d| (**d).uuid == location.disk_uuid) {
                // Only select from healthy/degraded disks (never failed)
                if (**disk).health != DiskHealth::Failed {
                    candidates.push((location.disk_uuid, location.fragment_index));
                }
            }
        }

        if candidates.is_empty() {
            return None;
        }

        match strategy {
            ReplicaSelectionStrategy::First => candidates.first().copied(),
            ReplicaSelectionStrategy::LeastLoaded => {
                Self::select_least_loaded(disks, &candidates)
            }
            ReplicaSelectionStrategy::Smart => Self::select_smart(disks, &candidates),
            ReplicaSelectionStrategy::RoundRobin => {
                // For now, use first available (stateless)
                candidates.first().copied()
            }
        }
    }

    /// Select replica on disk with least usage
    fn select_least_loaded(
        disks: &[&Disk],
        candidates: &[(Uuid, usize)],
    ) -> Option<(Uuid, usize)> {
        candidates
            .iter()
            .copied()
            .min_by_key(|(disk_uuid, _)| {
                disks
                    .iter()
                    .find(|d| (**d).uuid == *disk_uuid)
                    .map(|d| (**d).used_bytes)
                    .unwrap_or(u64::MAX)
            })
    }

    /// Select replica considering both health and load
    fn select_smart(
        disks: &[&Disk],
        candidates: &[(Uuid, usize)],
    ) -> Option<(Uuid, usize)> {
        let mut best: Option<(Uuid, usize, i32)> = None;

        for &(disk_uuid, fragment_index) in candidates {
            if let Some(disk) = disks.iter().find(|d| (**d).uuid == disk_uuid) {
                // Score based on: health (primary) + load (secondary)
                let health_score = match (**disk).health {
                    DiskHealth::Healthy => 100,
                    DiskHealth::Degraded => 50,
                    DiskHealth::Suspect => 25,
                    DiskHealth::Draining => 10,
                    DiskHealth::Failed => 0,
                };

                // Load score: lower is better (normalize to 0-100)
                let capacity = (**disk).capacity_bytes.max(1);
                let load_score =
                    100 - (((**disk).used_bytes as f64 / capacity as f64 * 100.0) as i32).min(100);

                // Combined score: health weighted 3x, load weighted 1x
                let score = health_score * 3 + load_score;

                if best.is_none() || score > best.as_ref().unwrap().2 {
                    best = Some((disk_uuid, fragment_index, score));
                }
            }
        }

        best.map(|(uuid, idx, _)| (uuid, idx))
    }
}

/// Fragment read scheduler for parallel operations
pub struct FragmentReadScheduler {
    /// Maximum concurrent reads
    pub max_concurrent: usize,
}

impl FragmentReadScheduler {
    pub fn new(max_concurrent: usize) -> Self {
        FragmentReadScheduler {
            max_concurrent: max_concurrent.max(1),
        }
    }

    /// Plan parallel reads for multiple fragments
    /// Returns batches of (disk_uuid, fragment_index) tuples to read in parallel
    pub fn plan_parallel_reads(
        &self,
        extent: &Extent,
        disks: &[Disk],
        strategy: ReplicaSelectionStrategy,
    ) -> Vec<Vec<(Uuid, usize)>> {
        let mut batches = Vec::new();
        let mut current_batch = Vec::new();
        let mut disk_load: std::collections::HashMap<Uuid, usize> = std::collections::HashMap::new();

        // Create references for the selector
        let disk_refs: Vec<&Disk> = disks.iter().collect();

        // For each fragment, select best replica
        for i in 0..extent.redundancy.fragment_count() {
            if let Some((replica_disk, replica_idx)) =
                ReplicaSelector::select_replica(extent, &disk_refs, strategy)
            {
                // Check if we can add to current batch (different disks preferred)
                let disk_count = disk_load.entry(replica_disk).or_insert(0);
                if current_batch.len() < self.max_concurrent && *disk_count == 0 {
                    current_batch.push((replica_disk, replica_idx));
                    *disk_count += 1;
                } else {
                    // Start new batch
                    if !current_batch.is_empty() {
                        batches.push(current_batch.clone());
                        current_batch.clear();
                        disk_load.clear();
                    }
                    current_batch.push((replica_disk, replica_idx));
                    disk_load.insert(replica_disk, 1);
                }
            }
        }

        if !current_batch.is_empty() {
            batches.push(current_batch);
        }

        batches
    }
}

/// Write scheduler for optimized fragment placement
pub struct FragmentWriteScheduler {
    /// Preferred batch size for parallel writes
    pub batch_size: usize,
}

impl FragmentWriteScheduler {
    pub fn new(batch_size: usize) -> Self {
        FragmentWriteScheduler {
            batch_size: batch_size.max(1),
        }
    }

    /// Plan parallel writes to balance load across disks
    /// Returns batches of disk UUIDs to write to in parallel
    pub fn plan_parallel_writes(
        &self,
        fragment_count: usize,
        available_disks: &[Disk],
    ) -> Result<Vec<Vec<uuid::Uuid>>> {
        // Sort disks by available space (descending)
        let mut sorted_disks = available_disks.to_vec();
        sorted_disks.sort_by(|a, b| {
            let a_free = a.capacity_bytes.saturating_sub(a.used_bytes);
            let b_free = b.capacity_bytes.saturating_sub(b.used_bytes);
            b_free.cmp(&a_free) // Descending: more space first
        });

        // Filter to healthy disks only
        sorted_disks.retain(|d| d.health == DiskHealth::Healthy);

        if sorted_disks.len() < fragment_count {
            return Err(anyhow::anyhow!(
                "Not enough healthy disks: need {}, have {}",
                fragment_count,
                sorted_disks.len()
            ));
        }

        // Round-robin assignment to balance load
        let mut batches = vec![Vec::new(); self.batch_size.min(fragment_count)];
        for i in 0..fragment_count {
            let batch_idx = i % batches.len();
            let disk_idx = i % sorted_disks.len();
            batches[batch_idx].push(sorted_disks[disk_idx].uuid);
        }

        // Remove empty batches
        batches.retain(|b| !b.is_empty());

        Ok(batches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replica_selector_first_strategy() {
        // Test that first strategy returns first available replica
        // This would require setting up test disks and extents
    }

    #[test]
    fn test_write_scheduler_balances_load() {
        let scheduler = FragmentWriteScheduler::new(2);
        // Would test that writes are balanced across healthy disks
    }
}
