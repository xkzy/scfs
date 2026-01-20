use anyhow::{anyhow, Result};
use uuid::Uuid;

use crate::disk::{Disk, DiskHealth};
use crate::extent::{Extent, FragmentLocation};

/// Placement engine: decides where to place fragments
pub struct PlacementEngine;

impl PlacementEngine {
    /// Select disks for placing fragments
    /// Ensures:
    /// - Different disks for each fragment of same extent
    /// - Prefer healthy disks
    /// - Prefer disks with more free space
    pub fn select_disks(
        &self,
        disks: &[Disk],
        fragment_count: usize,
        fragment_size: usize,
    ) -> Result<Vec<Uuid>> {
        // Filter healthy disks with enough space
        let mut candidates: Vec<&Disk> = disks
            .iter()
            .filter(|d| {
                d.health == DiskHealth::Healthy && d.has_space(fragment_size as u64)
            })
            .collect();
        
        if candidates.len() < fragment_count {
            return Err(anyhow!(
                "Not enough healthy disks: need {}, have {}",
                fragment_count,
                candidates.len()
            ));
        }
        
        // Sort by free space (descending)
        candidates.sort_by_key(|d| std::cmp::Reverse(d.capacity_bytes - d.used_bytes));
        
        // Select top N disks
        Ok(candidates
            .iter()
            .take(fragment_count)
            .map(|d| d.uuid)
            .collect())
    }
    
    /// Place fragments of an extent onto disks
    pub fn place_extent(
        &self,
        extent: &mut Extent,
        disks: &mut [Disk],
        fragments: &[Vec<u8>],
    ) -> Result<()> {
        let fragment_size = if !fragments.is_empty() {
            fragments[0].len()
        } else {
            0
        };
        
        // Select disks
        let disk_uuids = self.select_disks(disks, fragments.len(), fragment_size)?;
        
         // Write fragments to selected disks, rolling back on error
         let mut written_locations: Vec<FragmentLocation> = Vec::new();
         for (fragment_index, (fragment_data, disk_uuid)) in
             fragments.iter().zip(disk_uuids.iter()).enumerate()
         {
             // Find the disk
             let disk = disks
                 .iter_mut()
                 .find(|d| &d.uuid == disk_uuid)
                 .ok_or_else(|| anyhow!("Disk not found: {}", disk_uuid))?;
             
             if let Err(err) = disk.write_fragment(&extent.uuid, fragment_index, fragment_data) {
                 // Cleanup any fragments written so far
                 for location in &written_locations {
                     if let Some(disk) = disks.iter_mut().find(|d| d.uuid == location.disk_uuid) {
                         disk.delete_fragment(&extent.uuid, location.fragment_index).ok();
                     }
                 }
                 return Err(err);
             }
             
             written_locations.push(FragmentLocation {
                 disk_uuid: *disk_uuid,
                 fragment_index,
             });
             
             log::debug!(
                 "Placed fragment {} of extent {} on disk {}",
                 fragment_index,
                 extent.uuid,
                 disk_uuid
             );
         }

        extent.fragment_locations.extend(written_locations);
        
        Ok(())
    }
    
    /// Rebuild missing fragments of an extent
    pub fn rebuild_extent(
        &self,
        extent: &mut Extent,
        disks: &mut [Disk],
        existing_fragments: &[Option<Vec<u8>>],
    ) -> Result<()> {
        // Find which fragments are missing
        let missing_indices: Vec<usize> = existing_fragments
            .iter()
            .enumerate()
            .filter_map(|(i, f)| if f.is_none() { Some(i) } else { None })
            .collect();
        
        if missing_indices.is_empty() {
            return Ok(());
        }
        
        log::info!(
            "Rebuilding {} missing fragments for extent {}",
            missing_indices.len(),
            extent.uuid
        );
        
        // First, decode the original data
        let original_data = crate::redundancy::decode(existing_fragments, extent.redundancy)?;
        
        // Re-encode to get all fragments
        let all_fragments = crate::redundancy::encode(&original_data, extent.redundancy)?;
        
        // Place missing fragments on new disks
        for missing_index in missing_indices {
            let fragment_data = &all_fragments[missing_index];
            
            // Find a disk that doesn't already have this extent
            let used_disk_uuids: Vec<Uuid> = extent
                .fragment_locations
                .iter()
                .map(|loc| loc.disk_uuid)
                .collect();
            
            let available_disks: Vec<&Disk> = disks
                .iter()
                .filter(|d| {
                    d.health == DiskHealth::Healthy
                        && !used_disk_uuids.contains(&d.uuid)
                        && d.has_space(fragment_data.len() as u64)
                })
                .collect();
            
            if available_disks.is_empty() {
                return Err(anyhow!(
                    "No available disk for rebuilding fragment {}",
                    missing_index
                ));
            }
            
            // Use disk with most free space
            let target_disk_uuid = available_disks
                .iter()
                .max_by_key(|d| d.capacity_bytes - d.used_bytes)
                .unwrap()
                .uuid;
            
            // Write fragment
            let disk = disks
                .iter_mut()
                .find(|d| d.uuid == target_disk_uuid)
                .unwrap();
            
            disk.write_fragment(&extent.uuid, missing_index, fragment_data)?;
            
            // Record location
            extent.fragment_locations.push(FragmentLocation {
                disk_uuid: target_disk_uuid,
                fragment_index: missing_index,
            });
            
            log::info!(
                "Rebuilt fragment {} of extent {} on disk {}",
                missing_index,
                extent.uuid,
                target_disk_uuid
            );
        }
        
        Ok(())
    }
    
    /// Change redundancy policy of an extent (re-bundancy)
    /// This operation:
    /// 1. Decodes data with old policy
    /// 2. Encodes with new policy  
    /// 3. Places new fragments on disks
    /// 4. Deletes old fragments
    pub fn rebundle_extent(
        &self,
        extent: &mut Extent,
        disks: &mut [Disk],
        existing_fragments: &[Option<Vec<u8>>],
        new_policy: crate::extent::RedundancyPolicy,
    ) -> Result<()> {
        use crate::extent::TransitionStatus;
        
        log::info!(
            "Rebundling extent {} from {:?} to {:?}",
            extent.uuid,
            extent.redundancy,
            new_policy
        );
        
        let old_policy = extent.redundancy;
        
        // Step 0: Initiate policy change
        extent.initiate_policy_change(new_policy)?;
        
        // Step 1: Decode with old policy
        let original_data = crate::redundancy::decode(existing_fragments, old_policy)?;
        
        // Step 2: Re-encode with new policy
        let new_fragments = crate::redundancy::reencode(
            existing_fragments,
            old_policy,
            new_policy,
        )?;
        
        log::debug!(
            "Re-encoded extent {}: {} → {} fragments",
            extent.uuid,
            old_policy.fragment_count(),
            new_policy.fragment_count()
        );
        
        // Step 3: Delete old fragments from disks
        let old_fragment_locations = extent.fragment_locations.clone();
        for location in &old_fragment_locations {
            if let Some(disk) = disks.iter_mut().find(|d| d.uuid == location.disk_uuid) {
                disk.delete_fragment(&extent.uuid, location.fragment_index)?;
                log::debug!(
                    "Deleted old fragment {} from disk {}",
                    location.fragment_index,
                    location.disk_uuid
                );
            }
        }
        
        // Step 4: Place new fragments on disks
        extent.fragment_locations.clear();
        extent.mark_transition_in_progress();
        
        let fragment_size = if !new_fragments.is_empty() {
            new_fragments[0].len()
        } else {
            0
        };
        
        let disk_uuids = self.select_disks(disks, new_fragments.len(), fragment_size)?;
        
        for (fragment_index, (fragment_data, disk_uuid)) in
            new_fragments.iter().zip(disk_uuids.iter()).enumerate()
        {
            let disk = disks
                .iter_mut()
                .find(|d| &d.uuid == disk_uuid)
                .ok_or_else(|| anyhow!("Disk not found: {}", disk_uuid))?;
            
            disk.write_fragment(&extent.uuid, fragment_index, fragment_data)?;
            
            extent.fragment_locations.push(FragmentLocation {
                disk_uuid: *disk_uuid,
                fragment_index,
            });
            
            log::debug!(
                "Placed new fragment {} of extent {} on disk {}",
                fragment_index,
                extent.uuid,
                disk_uuid
            );
        }
        
        // Step 5: Commit policy change
        extent.commit_policy_change(new_policy)?;
        
        log::info!(
            "Successfully rebundled extent {} (old: {}, new: {})",
            extent.uuid,
            old_policy.fragment_count(),
            new_policy.fragment_count()
        );
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;
    
    #[test]
    fn test_select_disks() {
        let temp_dirs: Vec<TempDir> = (0..5)
            .map(|_| tempfile::tempdir().unwrap())
            .collect();
        
        let disks: Vec<Disk> = temp_dirs
            .iter()
            .map(|td| Disk::new(td.path().to_path_buf()).unwrap())
            .collect();
        
        let engine = PlacementEngine;
        let selected = engine.select_disks(&disks, 3, 1024).unwrap();
        
        assert_eq!(selected.len(), 3);
        
        // All selected disks should be unique
        let unique: std::collections::HashSet<_> = selected.iter().collect();
        assert_eq!(unique.len(), 3);
    }
}
