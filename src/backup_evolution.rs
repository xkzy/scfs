use uuid::Uuid;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// Incremental backup manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub id: Uuid,
    pub backup_type: BackupType,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub base_backup: Option<Uuid>, // For incremental: ID of base backup
    pub filesystem_version: u32,
    pub extents: Vec<BackupExtentInfo>,
    pub total_size: u64,
    pub compressed_size: Option<u64>,
    pub status: BackupStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackupType {
    Full,
    Incremental,
    Differential, // Only changed blocks
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackupStatus {
    InProgress,
    Completed,
    Failed,
    Verified,
}

/// Information about an extent in a backup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupExtentInfo {
    pub extent_uuid: Uuid,
    pub size: u64,
    pub checksum: [u8; 32],
    pub offset_in_backup: u64,
    pub compressed: bool,
    pub compression_ratio: Option<f64>,
}

/// Change tracking for incremental backups
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeLog {
    pub from_version: u32,
    pub to_version: u32,
    pub entries: Vec<ChangeEntry>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEntry {
    pub extent_uuid: Uuid,
    pub change_type: ChangeType,
    pub timestamp: DateTime<Utc>,
    pub size_delta: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChangeType {
    Created,
    Modified,
    Deleted,
}

/// Backup manager for creating and tracking backups
pub struct BackupManager {
    backups: HashMap<Uuid, BackupManifest>,
    changelog: Vec<ChangeLog>,
    version: u32,
}

impl BackupManager {
    pub fn new() -> Self {
        BackupManager {
            backups: HashMap::new(),
            changelog: Vec::new(),
            version: 0,
        }
    }

    /// Create a full backup
    pub fn create_full_backup(&mut self, extents: &[(Uuid, u64)]) -> BackupManifest {
        let backup_id = Uuid::new_v4();
        let total_size: u64 = extents.iter().map(|(_, size)| size).sum();

        let extent_infos: Vec<_> = extents
            .iter()
            .enumerate()
            .map(|(idx, (uuid, size))| BackupExtentInfo {
                extent_uuid: *uuid,
                size: *size,
                checksum: [0; 32], // Would be computed
                offset_in_backup: (idx as u64) * size,
                compressed: false,
                compression_ratio: None,
            })
            .collect();

        let manifest = BackupManifest {
            id: backup_id,
            backup_type: BackupType::Full,
            created_at: Utc::now(),
            completed_at: None,
            base_backup: None,
            filesystem_version: self.version,
            extents: extent_infos,
            total_size,
            compressed_size: None,
            status: BackupStatus::InProgress,
        };

        self.backups.insert(backup_id, manifest.clone());
        self.version += 1;

        manifest
    }

    /// Create an incremental backup based on a previous backup
    pub fn create_incremental_backup(
        &mut self,
        base_backup_id: Uuid,
        changes: &[(Uuid, ChangeType, u64)],
    ) -> anyhow::Result<BackupManifest> {
        let base_backup = self.backups.get(&base_backup_id)
            .ok_or_else(|| anyhow::anyhow!("Base backup not found"))?
            .clone();

        let backup_id = Uuid::new_v4();
        let mut total_size = 0u64;

        let extent_infos: Vec<_> = changes
            .iter()
            .enumerate()
            .map(|(idx, (uuid, _change_type, size))| {
                total_size += size;
                BackupExtentInfo {
                    extent_uuid: *uuid,
                    size: *size,
                    checksum: [0; 32],
                    offset_in_backup: (idx as u64) * size,
                    compressed: false,
                    compression_ratio: None,
                }
            })
            .collect();

        let manifest = BackupManifest {
            id: backup_id,
            backup_type: BackupType::Incremental,
            created_at: Utc::now(),
            completed_at: None,
            base_backup: Some(base_backup_id),
            filesystem_version: self.version,
            extents: extent_infos,
            total_size,
            compressed_size: None,
            status: BackupStatus::InProgress,
        };

        self.backups.insert(backup_id, manifest.clone());

        // Track changelog
        let changelog = ChangeLog {
            from_version: base_backup.filesystem_version,
            to_version: self.version,
            entries: changes
                .iter()
                .map(|(uuid, change_type, size)| ChangeEntry {
                    extent_uuid: *uuid,
                    change_type: *change_type,
                    timestamp: Utc::now(),
                    size_delta: match change_type {
                        ChangeType::Created => *size as i64,
                        ChangeType::Modified => 0,
                        ChangeType::Deleted => -(*size as i64),
                    },
                })
                .collect(),
            created_at: Utc::now(),
        };

        self.changelog.push(changelog);
        self.version += 1;

        Ok(manifest)
    }

    /// Mark backup as completed
    pub fn complete_backup(&mut self, backup_id: Uuid) -> anyhow::Result<()> {
        if let Some(manifest) = self.backups.get_mut(&backup_id) {
            manifest.completed_at = Some(Utc::now());
            manifest.status = BackupStatus::Completed;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Backup not found"))
        }
    }

    /// Verify backup integrity
    pub fn verify_backup(&mut self, backup_id: Uuid) -> anyhow::Result<bool> {
        if let Some(manifest) = self.backups.get_mut(&backup_id) {
            // In real implementation, would verify checksums
            let valid = !manifest.extents.is_empty();
            if valid {
                manifest.status = BackupStatus::Verified;
            }
            Ok(valid)
        } else {
            Err(anyhow::anyhow!("Backup not found"))
        }
    }

    /// Get backup manifest
    pub fn get_backup(&self, backup_id: Uuid) -> Option<BackupManifest> {
        self.backups.get(&backup_id).cloned()
    }

    /// List all backups
    pub fn list_backups(&self) -> Vec<BackupManifest> {
        let mut backups: Vec<_> = self.backups.values().cloned().collect();
        backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        backups
    }

    /// Calculate incremental backup size
    pub fn calculate_incremental_size(&self, changes: &[(Uuid, ChangeType, u64)]) -> u64 {
        changes.iter().map(|(_, _, size)| size).sum()
    }

    /// Get changelog between versions
    pub fn get_changelog(&self, from_version: u32, to_version: u32) -> Option<ChangeLog> {
        self.changelog.iter().find(|c| c.from_version == from_version && c.to_version == to_version).cloned()
    }

    /// Calculate total backup storage used
    pub fn total_backup_size(&self) -> u64 {
        self.backups.values()
            .map(|b| b.compressed_size.unwrap_or(b.total_size))
            .sum()
    }
}

/// Format versioning for compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub features: Vec<String>,
}

impl FormatVersion {
    pub fn current() -> Self {
        FormatVersion {
            major: 1,
            minor: 0,
            patch: 0,
            features: vec![
                "atomic_metadata".to_string(),
                "checksums".to_string(),
                "erasure_coding".to_string(),
                "snapshots".to_string(),
            ],
        }
    }

    pub fn is_compatible(&self, other: &FormatVersion) -> bool {
        self.major == other.major && self.minor >= other.minor
    }
}

/// Online upgrade tracker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeOperation {
    pub from_version: FormatVersion,
    pub to_version: FormatVersion,
    pub status: UpgradeStatus,
    pub start_time: DateTime<Utc>,
    pub completion_time: Option<DateTime<Utc>>,
    pub progress_percent: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UpgradeStatus {
    NotStarted,
    InProgress,
    Completed,
    RolledBack,
    Failed,
}

impl UpgradeOperation {
    pub fn new(from_version: FormatVersion, to_version: FormatVersion) -> Self {
        UpgradeOperation {
            from_version,
            to_version,
            status: UpgradeStatus::NotStarted,
            start_time: Utc::now(),
            completion_time: None,
            progress_percent: 0,
        }
    }

    pub fn mark_in_progress(&mut self) {
        self.status = UpgradeStatus::InProgress;
    }

    pub fn mark_completed(&mut self) {
        self.status = UpgradeStatus::Completed;
        self.progress_percent = 100;
        self.completion_time = Some(Utc::now());
    }

    pub fn mark_failed(&mut self) {
        self.status = UpgradeStatus::Failed;
        self.completion_time = Some(Utc::now());
    }

    pub fn duration_secs(&self) -> u64 {
        let end = self.completion_time.unwrap_or_else(Utc::now);
        (end - self.start_time).num_seconds() as u64
    }
}

