use anyhow::Result;
use uuid::Uuid;

use crate::disk::Disk;
use crate::extent::Extent;
use crate::metadata::MetadataManager;
use crate::placement::PlacementEngine;
use crate::redundancy;

/// Scrubber performs online verification and repair
pub struct Scrubber {
    metadata_dir: std::path::PathBuf,
}

/// Result of a scrub operation on a single extent
#[derive(Debug, Clone)]
pub struct ScrubResult {
    pub extent_uuid: Uuid,
    pub status: ScrubStatus,
    pub issues: Vec<String>,
    pub repairs_attempted: usize,
    pub repairs_successful: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrubStatus {
    Healthy,       // All fragments OK, checksums verified
    Degraded,      // Some fragments missing but readable
    Repaired,      // Issues detected and fixed
    Unrecoverable, // Cannot read or repair
}

impl Scrubber {
    pub fn new(metadata_dir: std::path::PathBuf) -> Self {
        Scrubber { metadata_dir }
    }

    /// Verify a single extent: checksums, fragment counts, placement validity
    pub fn verify_extent(
        &self,
        extent: &Extent,
        metadata: &MetadataManager,
        disks: &[Disk],
    ) -> Result<ScrubResult> {
        let mut result = ScrubResult {
            extent_uuid: extent.uuid,
            status: ScrubStatus::Healthy,
            issues: Vec::new(),
            repairs_attempted: 0,
            repairs_successful: 0,
        };

        // Check 1: Fragment count vs policy
        let expected_fragments = extent.redundancy.fragment_count();
        let available_fragments = extent.fragment_locations.len();
        
        if available_fragments < expected_fragments {
            result.issues.push(format!(
                "Missing fragments: have {}, expected {}",
                available_fragments, expected_fragments
            ));
            if available_fragments < extent.redundancy.min_fragments() {
                result.status = ScrubStatus::Unrecoverable;
                return Ok(result);
            }
            result.status = ScrubStatus::Degraded;
        }

        // Check 2: Verify checksum on readable data
        let mut fragments = vec![None; expected_fragments];
        let mut readable_count = 0;

        for location in &extent.fragment_locations {
            if let Some(disk) = disks.iter().find(|d| d.uuid == location.disk_uuid) {
                let data_result = if let Some(ref placement) = location.on_device {
                    // Block device: use placement information
                    disk.read_fragment_at_placement(placement)
                } else {
                    // Regular disk: use file-based reading
                    disk.read_fragment(&extent.uuid, location.fragment_index)
                };

                match data_result {
                    Ok(data) => {
                        fragments[location.fragment_index] = Some(data);
                        readable_count += 1;
                    }
                    Err(e) => {
                        result.issues.push(format!(
                            "Failed to read fragment {}: {}",
                            location.fragment_index, e
                        ));
                    }
                }
            } else {
                result.issues.push(format!(
                    "Fragment {} on missing disk {}",
                    location.fragment_index, location.disk_uuid
                ));
            }
        }

        if readable_count < extent.redundancy.min_fragments() {
            result.status = ScrubStatus::Unrecoverable;
            result.issues.push(format!(
                "Not enough readable fragments to decode: {}/{}",
                readable_count,
                extent.redundancy.min_fragments()
            ));
            return Ok(result);
        }

        // Check 3: Verify data checksum (if we can decode)
        match redundancy::decode(&fragments, extent.redundancy) {
            Ok(data) => {
                if !extent.verify_checksum(&data[..extent.size]) {
                    result.issues.push("Checksum verification failed".to_string());
                    result.status = ScrubStatus::Unrecoverable;
                }
            }
            Err(e) => {
                result.issues.push(format!("Failed to decode extent: {}", e));
                result.status = ScrubStatus::Unrecoverable;
            }
        }

        Ok(result)
    }

    /// Attempt automatic repair of a degraded extent
    /// Conservative: only repairs when safe (min_fragments available)
    /// Idempotent: safe to call multiple times
    pub fn repair_extent(
        &self,
        extent: &mut Extent,
        metadata: &MetadataManager,
        disks: &mut [Disk],
        placement: &PlacementEngine,
        fragments: &[Option<Vec<u8>>],
    ) -> Result<ScrubResult> {
        let mut result = self.verify_extent(extent, metadata, disks)?;

        // Only attempt repair if degraded (readable but incomplete)
        if result.status != ScrubStatus::Degraded {
            return Ok(result);
        }

        // Check: Do we have minimum fragments to decode?
        let readable_count = fragments.iter().filter(|f| f.is_some()).count();
        if readable_count < extent.redundancy.min_fragments() {
            result.issues.push("Insufficient fragments to repair (cannot decode)".to_string());
            result.status = ScrubStatus::Unrecoverable;
            return Ok(result);
        }

        result.repairs_attempted += 1;

        // Use placement engine's rebuild_extent method for repair
        // Convert disks to Arc<Mutex<Disk>> format expected by placement engine
        let disk_arcs: Vec<std::sync::Arc<std::sync::Mutex<Disk>>> = 
            disks.iter_mut().map(|d| std::sync::Arc::new(std::sync::Mutex::new(d.clone()))).collect();
        
        match placement.rebuild_extent(extent, &disk_arcs, fragments) {
            Ok(_) => {
                result.repairs_successful += 1;
                result.status = ScrubStatus::Repaired;
                result.issues.push("Successfully repaired extent".to_string());
                log::info!("Successfully repaired extent {}", extent.uuid);
            }
            Err(e) => {
                result.issues.push(format!("Repair failed: {}", e));
                result.status = ScrubStatus::Degraded;
                log::warn!("Repair attempted but failed for extent {}: {}", extent.uuid, e);
            }
        }

        Ok(result)
    }

    pub fn scrub_all(
        &self,
        metadata: &MetadataManager,
        disks: &[Disk],
    ) -> Result<Vec<ScrubResult>> {
        log::info!("Starting full scrub of all extents");

        let extents = metadata.list_all_extents()?;
        let mut results = Vec::new();

        for extent in extents {
            match self.verify_extent(&extent, metadata, disks) {
                Ok(result) => {
                    if result.status != ScrubStatus::Healthy {
                        log::warn!(
                            "Extent {}: {:?} - {:?}",
                            result.extent_uuid,
                            result.status,
                            result.issues
                        );
                    }
                    results.push(result);
                }
                Err(e) => {
                    log::error!("Failed to verify extent {}: {}", extent.uuid, e);
                }
            }
        }

        log::info!("Scrub complete: {} extents verified", results.len());
        Ok(results)
    }

    /// Get scrub statistics
    pub fn stats(results: &[ScrubResult]) -> ScrubStats {
        let mut stats = ScrubStats {
            total_extents: results.len(),
            healthy: 0,
            degraded: 0,
            repaired: 0,
            unrecoverable: 0,
            total_issues: 0,
            total_repairs: 0,
        };

        for result in results {
            match result.status {
                ScrubStatus::Healthy => stats.healthy += 1,
                ScrubStatus::Degraded => stats.degraded += 1,
                ScrubStatus::Repaired => stats.repaired += 1,
                ScrubStatus::Unrecoverable => stats.unrecoverable += 1,
            }
            stats.total_issues += result.issues.len();
            stats.total_repairs += result.repairs_successful;
        }

        stats
    }
}

#[derive(Debug, Clone)]
pub struct ScrubStats {
    pub total_extents: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub repaired: usize,
    pub unrecoverable: usize,
    pub total_issues: usize,
    pub total_repairs: usize,
}

impl std::fmt::Display for ScrubStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Scrub Results: {}/{} healthy, {} degraded, {} repaired, {} unrecoverable ({} issues, {} repairs)",
            self.healthy, self.total_extents, self.degraded, self.repaired, self.unrecoverable, self.total_issues, self.total_repairs
        )
    }
}
