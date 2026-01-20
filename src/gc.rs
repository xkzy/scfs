use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use uuid::Uuid;

use crate::disk::Disk;
use crate::metadata::MetadataManager;

/// Orphan fragment information
#[derive(Debug, Clone)]
pub struct OrphanFragment {
    pub disk_path: PathBuf,
    pub fragment_path: PathBuf,
    pub extent_uuid: Uuid,
    pub fragment_index: usize,
    pub age_seconds: u64,
    pub size_bytes: u64,
}

/// Garbage collection manager for orphaned fragments
pub struct GarbageCollector {
    pool_dir: PathBuf,
    disks: Vec<Disk>,
}

impl GarbageCollector {
    pub fn new(pool_dir: PathBuf, disks: Vec<Disk>) -> Self {
        GarbageCollector { pool_dir, disks }
    }

    /// Scan all disks and build a set of all fragment locations on disk
    fn scan_all_fragments(&self) -> Result<HashMap<Uuid, HashSet<usize>>> {
        let mut all_fragments: HashMap<Uuid, HashSet<usize>> = HashMap::new();

        for disk in &self.disks {
            let fragments_dir = disk.path.join("fragments");
            if !fragments_dir.exists() {
                continue;
            }

            for entry in fs::read_dir(&fragments_dir)
                .context(format!("Failed to read fragments dir: {:?}", fragments_dir))?
            {
                let entry = entry?;
                let filename = entry.file_name();
                let filename_str = filename.to_string_lossy();

                // Fragment naming: <uuid>_<index>
                if let Some((uuid_str, index_str)) = filename_str.split_once('_') {
                    if let (Ok(uuid), Ok(index)) = (
                        Uuid::parse_str(uuid_str),
                        index_str.parse::<usize>(),
                    ) {
                        all_fragments
                            .entry(uuid)
                            .or_insert_with(HashSet::new)
                            .insert(index);
                    }
                }
            }
        }

        Ok(all_fragments)
    }

    /// Load all extent metadata and build a set of referenced fragment locations
    fn scan_referenced_fragments(&self) -> Result<HashMap<Uuid, HashSet<usize>>> {
        let metadata = MetadataManager::new(self.pool_dir.clone())?;
        let mut referenced: HashMap<Uuid, HashSet<usize>> = HashMap::new();

        // Load all extents from metadata
        let extents = metadata.list_all_extents()?;

        for extent in extents {
            let uuid = extent.uuid;
            for location in &extent.fragment_locations {
                referenced
                    .entry(uuid)
                    .or_insert_with(HashSet::new)
                    .insert(location.fragment_index);
            }
        }

        Ok(referenced)
    }

    /// Detect orphaned fragments (on disk but not referenced in metadata)
    pub fn detect_orphans(&self) -> Result<Vec<OrphanFragment>> {
        let all_fragments = self.scan_all_fragments()?;
        let referenced = self.scan_referenced_fragments()?;

        let mut orphans = Vec::new();

        for (extent_uuid, fragment_indices) in all_fragments {
            let referenced_indices = referenced.get(&extent_uuid).cloned().unwrap_or_default();

            for fragment_index in fragment_indices {
                if !referenced_indices.contains(&fragment_index) {
                    // This fragment is orphaned - find it on disk
                    for disk in &self.disks {
                        let fragment_path = disk
                            .path
                            .join("fragments")
                            .join(format!("{}_{}", extent_uuid, fragment_index));

                        if fragment_path.exists() {
                            let metadata = fs::metadata(&fragment_path)?;
                            let modified = metadata.modified()?;
                            let age_seconds = SystemTime::now()
                                .duration_since(modified)
                                .unwrap_or_default()
                                .as_secs();

                            orphans.push(OrphanFragment {
                                disk_path: disk.path.clone(),
                                fragment_path: fragment_path.clone(),
                                extent_uuid,
                                fragment_index,
                                age_seconds,
                                size_bytes: metadata.len(),
                            });
                            break;
                        }
                    }
                }
            }
        }

        Ok(orphans)
    }

    /// Clean up orphaned fragments older than the specified age
    ///
    /// # Arguments
    /// * `min_age_seconds` - Minimum age in seconds for a fragment to be considered for cleanup (default: 86400 = 24 hours)
    /// * `dry_run` - If true, only report what would be deleted without actually deleting
    ///
    /// # Returns
    /// Vector of orphans that were (or would be) cleaned up
    pub fn cleanup_orphans(&self, min_age_seconds: u64, dry_run: bool) -> Result<Vec<OrphanFragment>> {
        let orphans = self.detect_orphans()?;
        let mut cleaned = Vec::new();

        for orphan in orphans {
            if orphan.age_seconds >= min_age_seconds {
                if !dry_run {
                    fs::remove_file(&orphan.fragment_path)
                        .context(format!("Failed to remove orphan: {:?}", orphan.fragment_path))?;
                }
                cleaned.push(orphan);
            }
        }

        Ok(cleaned)
    }

    /// Get statistics about orphaned fragments
    pub fn get_orphan_stats(&self) -> Result<OrphanStats> {
        let orphans = self.detect_orphans()?;

        let total_count = orphans.len();
        let total_bytes: u64 = orphans.iter().map(|o| o.size_bytes).sum();
        let old_count = orphans.iter().filter(|o| o.age_seconds >= 86400).count();
        let old_bytes: u64 = orphans
            .iter()
            .filter(|o| o.age_seconds >= 86400)
            .map(|o| o.size_bytes)
            .sum();

        Ok(OrphanStats {
            total_count,
            total_bytes,
            old_count,
            old_bytes,
        })
    }
}

/// Statistics about orphaned fragments
#[derive(Debug, Clone)]
pub struct OrphanStats {
    pub total_count: usize,
    pub total_bytes: u64,
    pub old_count: usize,       // Older than 24 hours
    pub old_bytes: u64,
}
