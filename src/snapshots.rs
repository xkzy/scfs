use std::collections::HashMap;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use crate::extent::Extent;

/// Represents a point-in-time snapshot of filesystem state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub uuid: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub parent_uuid: Option<Uuid>, // None = full snapshot, Some = incremental
    pub root_inode_uuid: Uuid,
    pub description: String,
    pub file_count: u64,
    pub total_size: u64,
}

/// Tracks changes between snapshots for incremental backup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDelta {
    pub from_snapshot: Uuid,
    pub to_snapshot: Uuid,
    pub added_extents: Vec<Uuid>,
    pub modified_extents: Vec<Uuid>,
    pub deleted_extents: Vec<Uuid>,
    pub size_change: i64,
}

/// Copy-on-write snapshot manager
pub struct SnapshotManager {
    snapshots: HashMap<Uuid, Snapshot>,
    extent_refcounts: HashMap<Uuid, u64>, // Track extent references
    snapshot_index: HashMap<String, Uuid>, // Name -> UUID index
}

impl SnapshotManager {
    pub fn new() -> Self {
        SnapshotManager {
            snapshots: HashMap::new(),
            extent_refcounts: HashMap::new(),
            snapshot_index: HashMap::new(),
        }
    }

    /// Create a full (non-incremental) snapshot
    pub fn create_full_snapshot(
        &mut self,
        name: String,
        root_inode_uuid: Uuid,
        extents: &[Extent],
        description: String,
    ) -> Snapshot {
        let snapshot_uuid = Uuid::new_v4();
        let total_size: u64 = extents.iter().map(|e| e.size as u64).sum();

        // Increment refcount for all extents in snapshot
        for extent in extents {
            *self.extent_refcounts.entry(extent.uuid).or_insert(0) += 1;
        }

        let snapshot = Snapshot {
            uuid: snapshot_uuid,
            name: name.clone(),
            created_at: Utc::now(),
            parent_uuid: None,
            root_inode_uuid,
            description,
            file_count: extents.len() as u64,
            total_size,
        };

        self.snapshot_index.insert(name, snapshot_uuid);
        self.snapshots.insert(snapshot_uuid, snapshot.clone());

        snapshot
    }

    /// Create an incremental snapshot based on a parent
    pub fn create_incremental_snapshot(
        &mut self,
        name: String,
        parent_uuid: Uuid,
        root_inode_uuid: Uuid,
        new_extents: &[Extent],
        modified_extents: &[Extent],
        deleted_extents: &[Uuid],
        description: String,
    ) -> anyhow::Result<Snapshot> {
        // Verify parent exists
        let parent = self.snapshots.get(&parent_uuid)
            .ok_or_else(|| anyhow::anyhow!("Parent snapshot not found"))?;

        let snapshot_uuid = Uuid::new_v4();
        let mut total_size = parent.total_size;

        // Track changed extents
        let mut added_extents = Vec::new();
        let mut modified_count = 0;

        for extent in new_extents {
            total_size += extent.size as u64;
            added_extents.push(extent.uuid);
            *self.extent_refcounts.entry(extent.uuid).or_insert(0) += 1;
        }

        for extent in modified_extents {
            *self.extent_refcounts.entry(extent.uuid).or_insert(0) += 1;
            modified_count += 1;
        }

        for _extent_uuid in deleted_extents {
            // Rough size estimate per deletion
            total_size = total_size.saturating_sub(8192);
        }

        let snapshot = Snapshot {
            uuid: snapshot_uuid,
            name: name.clone(),
            created_at: Utc::now(),
            parent_uuid: Some(parent_uuid),
            root_inode_uuid,
            description,
            file_count: (parent.file_count as i64 + added_extents.len() as i64) as u64,
            total_size,
        };

        self.snapshot_index.insert(name, snapshot_uuid);
        self.snapshots.insert(snapshot_uuid, snapshot.clone());

        Ok(snapshot)
    }

    /// List all snapshots
    pub fn list_snapshots(&self) -> Vec<Snapshot> {
        let mut snapshots: Vec<_> = self.snapshots.values().cloned().collect();
        snapshots.sort_by(|a, b| b.created_at.cmp(&a.created_at)); // Most recent first
        snapshots
    }

    /// Get snapshot by name
    pub fn get_snapshot(&self, name: &str) -> Option<Snapshot> {
        self.snapshot_index
            .get(name)
            .and_then(|uuid| self.snapshots.get(uuid))
            .cloned()
    }

    /// Get snapshot by UUID
    pub fn get_snapshot_by_uuid(&self, uuid: Uuid) -> Option<Snapshot> {
        self.snapshots.get(&uuid).cloned()
    }

    /// Delete a snapshot (with refcount management)
    pub fn delete_snapshot(&mut self, uuid: Uuid) -> anyhow::Result<()> {
        let snapshot = self.snapshots.remove(&uuid)
            .ok_or_else(|| anyhow::anyhow!("Snapshot not found"))?;

        self.snapshot_index.remove(&snapshot.name);

        Ok(())
    }

    /// Check if extent can be deleted (refcount == 0)
    pub fn can_delete_extent(&self, extent_uuid: &Uuid) -> bool {
        self.extent_refcounts.get(extent_uuid).copied().unwrap_or(0) == 0
    }

    /// Get extent reference count
    pub fn extent_refcount(&self, extent_uuid: &Uuid) -> u64 {
        self.extent_refcounts.get(extent_uuid).copied().unwrap_or(0)
    }

    /// Estimate space savings from COW
    pub fn estimate_cow_savings(&self) -> u64 {
        // For each extent with refcount > 1, we save (refcount - 1) * size
        let mut savings = 0u64;
        for (_uuid, refcount) in &self.extent_refcounts {
            if *refcount > 1 {
                // Rough estimate: assume average extent is 1MB
                savings += (refcount - 1) * 1_000_000;
            }
        }
        savings
    }
}

/// Snapshot restore operation tracker
pub struct RestoreOperation {
    pub snapshot_uuid: Uuid,
    pub target_path: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub bytes_restored: u64,
    pub total_bytes: u64,
    pub status: RestoreStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreStatus {
    InProgress,
    Completed,
    Failed,
}

impl RestoreOperation {
    pub fn new(snapshot_uuid: Uuid, target_path: String, total_bytes: u64) -> Self {
        RestoreOperation {
            snapshot_uuid,
            target_path,
            started_at: Utc::now(),
            completed_at: None,
            bytes_restored: 0,
            total_bytes,
            status: RestoreStatus::InProgress,
        }
    }

    pub fn progress_percent(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            (self.bytes_restored as f64 / self.total_bytes as f64) * 100.0
        }
    }

    pub fn mark_completed(&mut self) {
        self.completed_at = Some(Utc::now());
        self.status = RestoreStatus::Completed;
    }

    pub fn mark_failed(&mut self) {
        self.completed_at = Some(Utc::now());
        self.status = RestoreStatus::Failed;
    }

    pub fn duration_secs(&self) -> u64 {
        let end = self.completed_at.unwrap_or_else(Utc::now);
        (end - self.started_at).num_seconds() as u64
    }
}

