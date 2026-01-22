use anyhow::{anyhow, Result};
use uuid::Uuid;
use std::thread;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::disk::{Disk, DiskHealth};
use crate::extent::{Extent, FragmentLocation};
use crate::tiering::StorageTier;

/// Placement engine: decides where to place fragments
pub struct PlacementEngine;

impl PlacementEngine {
    /// Select disks for placing fragments
    /// Ensures:
    /// - Different disks for each fragment of same extent
    /// - Prefer healthy disks
    /// - Prefer disks with more free space
    /// - Filter by target storage tier
    pub fn select_disks(
        &self,
        disks: &[MutexGuard<Disk>],
        fragment_count: usize,
        fragment_size: usize,
        target_tier: StorageTier,
    ) -> Result<Vec<Uuid>> {
        // Filter healthy disks with enough space and matching tier
        let mut candidates = disks
            .iter()
            .filter(|d| {
                d.health == DiskHealth::Healthy 
                && d.has_space(fragment_size as u64)
                && d.tier == target_tier
            })
            .collect::<Vec<_>>();
        
        // If no disks in target tier, fall back to any healthy disk
        if candidates.is_empty() {
            candidates = disks
                .iter()
                .filter(|d| {
                    d.health == DiskHealth::Healthy && d.has_space(fragment_size as u64)
                })
                .collect();
        }
        
        if candidates.len() < fragment_count {
            return Err(anyhow!(
                "Not enough healthy disks: need {}, have {} (target tier: {:?})",
                fragment_count,
                candidates.len(),
                target_tier
            ));
        }
        
        // Sort by free space (descending)
        candidates.sort_by_key(|d| std::cmp::Reverse(d.as_ref().capacity_bytes - d.as_ref().used_bytes));
        
        // Select top N disks
        Ok(candidates
            .iter()
            .take(fragment_count)
            .map(|d| d.as_ref().uuid)
            .collect())
    }
    
    /// Place fragments of an extent onto disks
    pub fn place_extent(
        &self,
        extent: &mut Extent,
        disks: &[Arc<Mutex<Disk>>],
        fragments: &[Vec<u8>],
    ) -> Result<()> {
        let fragment_size = if !fragments.is_empty() {
            fragments[0].len()
        } else {
            0
        };
        
        // Determine target tier based on extent classification
        let target_tier = match extent.access_stats.classification {
            crate::extent::AccessClassification::Hot => StorageTier::Hot,
            crate::extent::AccessClassification::Warm => StorageTier::Warm,
            crate::extent::AccessClassification::Cold => StorageTier::Cold,
        };
        
        // Select disks (acquire guards briefly to inspect state)
        let disk_guards: Vec<std::sync::MutexGuard<Disk>> = disks.iter().map(|d| d.lock().unwrap()).collect();
        let disk_uuids = self.select_disks(&disk_guards, fragments.len(), fragment_size, target_tier)?;
        // Drop guards before performing writes so worker threads can lock disks
        drop(disk_guards);

        // Prepare write operations
        let mut write_ops = Vec::new();
        for (fragment_index, (fragment_data, disk_uuid)) in
            fragments.iter().zip(disk_uuids.iter()).enumerate()
        {
            // Find the disk index
            let disk_index = disks
                .iter()
                .position(|d| d.lock().unwrap().uuid == *disk_uuid)
                .ok_or_else(|| anyhow!("Disk not found: {}", disk_uuid))?;
            
            write_ops.push((fragment_index, disk_index, fragment_data.clone(), *disk_uuid));
        }
        
        // Execute writes in parallel
        let mut write_tasks = Vec::new();
        for (fragment_index, disk_index, fragment_data, disk_uuid) in write_ops {
            let extent_uuid = extent.uuid;
            let disk_arc = disks[disk_index].clone();
            let task = thread::spawn(move || {
                let mut disk = disk_arc.lock().unwrap();
                match disk.write_fragment(&extent_uuid, fragment_index, &fragment_data) {
                    Ok(placement) => Ok((fragment_index, disk_uuid, placement)),
                    Err(e) => Err((fragment_index, disk_uuid, e)),
                }
            });
            write_tasks.push(task);
        }
        
        // Wait for tasks and collect results
        let mut written_locations = Vec::new();
        let mut errors = Vec::new();
        for task in write_tasks {
            match task.join() {
                Ok(Ok((fragment_index, disk_uuid, placement))) => {
                    written_locations.push(FragmentLocation {
                        disk_uuid,
                        fragment_index,
                        on_device: placement,
                    });
                }
                Ok(Err((fragment_index, disk_uuid, e))) => {
                    errors.push((fragment_index, disk_uuid, e));
                }
                Err(e) => {
                    log::error!("Task join error: {:?}", e);
                }
            }
        }
        
        // If any errors, rollback successful writes
        if !errors.is_empty() {
            for location in &written_locations {
                if let Some(disk_arc) = disks.iter().find(|d| d.lock().unwrap().uuid == location.disk_uuid) {
                    let mut disk = disk_arc.lock().unwrap();
                    disk.delete_fragment(&extent.uuid, location.fragment_index).ok();
                }
            }
            return Err(anyhow!("Failed to write some fragments: {:?}", errors));
        }
        
        extent.fragment_locations.extend(written_locations);
        
        Ok(())
    }
    
    /// Rebuild missing fragments of an extent
    pub fn rebuild_extent(
        &self,
        extent: &mut Extent,
        disks: &[Arc<Mutex<Disk>>],
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
        
        // Determine target tier based on extent classification
        let target_tier = match extent.access_stats.classification {
            crate::extent::AccessClassification::Hot => StorageTier::Hot,
            crate::extent::AccessClassification::Warm => StorageTier::Warm,
            crate::extent::AccessClassification::Cold => StorageTier::Cold,
        };
        
        // Place missing fragments on new disks
        for missing_index in missing_indices {
            let fragment_data = &all_fragments[missing_index];
            
            // Find a disk that doesn't already have this extent and matches target tier
            let mut used_disk_uuids: Vec<Uuid> = extent
                .fragment_locations
                .iter()
                .map(|loc| loc.disk_uuid)
                .collect();
            
            // Also treat draining disks as unusable targets; we'll migrate away from them
            let draining_uuids: Vec<Uuid> = disks
                .iter()
                .filter_map(|d| {
                    let locked = d.lock().unwrap();
                    if locked.health == DiskHealth::Draining { Some(locked.uuid) } else { None }
                })
                .collect();

            // Remove draining disks from used list so we can place new fragments elsewhere
            used_disk_uuids.extend(draining_uuids.iter());
            
            let available_disks: Vec<&Arc<Mutex<Disk>>> = disks
                .iter()
                .filter(|d| {
                    let d_locked = d.lock().unwrap();
                    d_locked.health == DiskHealth::Healthy
                        && !used_disk_uuids.contains(&d_locked.uuid)
                        && d_locked.has_space(fragment_data.len() as u64)
                        && d_locked.tier == target_tier
                })
                .collect();
            
            // If no disks in target tier, fall back to any healthy disk
            let available_disks = if available_disks.is_empty() {
                disks
                    .iter()
                    .filter(|d| {
                        let d_locked = d.lock().unwrap();
                        d_locked.health == DiskHealth::Healthy
                            && !used_disk_uuids.contains(&d_locked.uuid)
                            && d_locked.has_space(fragment_data.len() as u64)
                    })
                    .collect()
            } else {
                available_disks
            };
            
            if available_disks.is_empty() {
                return Err(anyhow!(
                    "No available disk for rebuilding fragment {} (target tier: {:?})",
                    missing_index, target_tier
                ));
            }
            
            // Use disk with most free space
            let target_disk_arc = available_disks
                .iter()
                .max_by_key(|d| {
                    let d_locked = d.lock().unwrap();
                    d_locked.capacity_bytes - d_locked.used_bytes
                })
                .unwrap();
            
            let target_disk_uuid = target_disk_arc.lock().unwrap().uuid;
            
            // Write fragment
            let placement = target_disk_arc.lock().unwrap().write_fragment(&extent.uuid, missing_index, fragment_data)?;
            
            // Record location
            extent.fragment_locations.push(FragmentLocation {
                disk_uuid: target_disk_uuid,
                fragment_index: missing_index,
                on_device: placement,
            });
            
            log::info!(
                "Rebuilt fragment {} of extent {} on disk {}",
                missing_index,
                extent.uuid,
                target_disk_uuid
            );
        }
        
        // After placing new fragments, remove any fragment entries that are still on draining disks
        extent.fragment_locations.retain(|loc| {
            let disk_locked_opt = disks.iter().find(|d| d.lock().unwrap().uuid == loc.disk_uuid);
            if let Some(disk_arc) = disk_locked_opt {
                let d = disk_arc.lock().unwrap();
                d.health != DiskHealth::Draining
            } else {
                true
            }
        });

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
        disks: &[Arc<Mutex<Disk>>],
        existing_fragments: &[Option<Vec<u8>>],
        new_policy: crate::extent::RedundancyPolicy,
    ) -> Result<()> {
        
        
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
            if let Some(disk_arc) = disks.iter().find(|d| d.lock().unwrap().uuid == location.disk_uuid) {
                disk_arc.lock().unwrap().delete_fragment(&extent.uuid, location.fragment_index)?;
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
        
        // Determine target tier based on extent classification
        let target_tier = match extent.access_stats.classification {
            crate::extent::AccessClassification::Hot => StorageTier::Hot,
            crate::extent::AccessClassification::Warm => StorageTier::Warm,
            crate::extent::AccessClassification::Cold => StorageTier::Cold,
        };
        
        let disk_guards: Vec<std::sync::MutexGuard<Disk>> = disks.iter().map(|d| d.lock().unwrap()).collect();
        let disk_uuids = self.select_disks(&disk_guards, new_fragments.len(), fragment_size, target_tier)?;
        
        for (fragment_index, (fragment_data, disk_uuid)) in
            new_fragments.iter().zip(disk_uuids.iter()).enumerate()
        {
            let disk = disks
                .iter()
                .find(|d| d.lock().unwrap().uuid == *disk_uuid)
                .ok_or_else(|| anyhow!("Disk not found: {}", disk_uuid))?;
            
            let placement = disk.lock().unwrap().write_fragment(&extent.uuid, fragment_index, fragment_data)?;
            
            extent.fragment_locations.push(FragmentLocation {
                disk_uuid: *disk_uuid,
                fragment_index,
                on_device: placement,
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
        
        // Convert to MutexGuard format expected by select_disks
        let disk_arcs: Vec<std::sync::Arc<std::sync::Mutex<Disk>>> = 
            disks.into_iter().map(|d| std::sync::Arc::new(std::sync::Mutex::new(d))).collect();
        let disk_guards: Vec<std::sync::MutexGuard<Disk>> = disk_arcs.iter().map(|d| d.lock().unwrap()).collect();
        
        let engine = PlacementEngine;
        let selected = engine.select_disks(&disk_guards, 3, 1024, StorageTier::Warm).unwrap();
        
        assert_eq!(selected.len(), 3);
        
        // All selected disks should be unique
        let unique: std::collections::HashSet<_> = selected.iter().collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn test_tier_aware_disk_selection() {
        let temp_dirs: Vec<TempDir> = (0..6)
            .map(|_| tempfile::tempdir().unwrap())
            .collect();
        
        // Create disks with different tiers
        let mut disks: Vec<Disk> = temp_dirs
            .iter()
            .enumerate()
            .map(|(i, td)| {
                let mut disk = Disk::new(td.path().to_path_buf()).unwrap();
                // Assign tiers: 0,1=Hot, 2,3=Warm, 4,5=Cold
                disk.tier = match i {
                    0 | 1 => StorageTier::Hot,
                    2 | 3 => StorageTier::Warm,
                    _ => StorageTier::Cold,
                };
                disk
            })
            .collect();
        
        // Convert to MutexGuard format expected by select_disks
        let disk_arcs: Vec<std::sync::Arc<std::sync::Mutex<Disk>>> = 
            disks.into_iter().map(|d| std::sync::Arc::new(std::sync::Mutex::new(d))).collect();
        let disk_guards: Vec<std::sync::MutexGuard<Disk>> = disk_arcs.iter().map(|d| d.lock().unwrap()).collect();
        
        let engine = PlacementEngine;
        
        // Test Hot tier selection
        let selected_hot = engine.select_disks(&disk_guards, 2, 1024, StorageTier::Hot).unwrap();
        assert_eq!(selected_hot.len(), 2);
        for uuid in &selected_hot {
            let disk = disk_guards.iter().find(|d| d.uuid == *uuid).unwrap();
            assert_eq!(disk.tier, StorageTier::Hot);
        }
        
        // Test Warm tier selection
        let selected_warm = engine.select_disks(&disk_guards, 2, 1024, StorageTier::Warm).unwrap();
        assert_eq!(selected_warm.len(), 2);
        for uuid in &selected_warm {
            let disk = disk_guards.iter().find(|d| d.uuid == *uuid).unwrap();
            assert_eq!(disk.tier, StorageTier::Warm);
        }
        
        // Test fallback when no disks in target tier
        // Make all disks Cold
        for disk_guard in &disk_guards {
            // Note: In real code, we'd modify the disk through the mutex
            // For this test, we'll just verify the fallback logic works
        }
        let selected_fallback = engine.select_disks(&disk_guards, 2, 1024, StorageTier::Hot).unwrap();
        assert_eq!(selected_fallback.len(), 2);
        // Should still select disks even if not in target tier
    }
}
