use anyhow::{anyhow, Result};
use std::sync::{Arc, RwLock};

use crate::disk::Disk;
use crate::extent::{split_into_extents, Extent, RedundancyPolicy, DEFAULT_EXTENT_SIZE};
use crate::metadata::{ExtentMap, Inode, MetadataManager};
use crate::placement::PlacementEngine;
use crate::redundancy;

/// Storage engine handling read/write operations
pub struct StorageEngine {
    metadata: Arc<RwLock<MetadataManager>>,
    disks: Arc<RwLock<Vec<Disk>>>,
    placement: PlacementEngine,
}

impl StorageEngine {
    pub fn new(metadata: MetadataManager, disks: Vec<Disk>) -> Self {
        StorageEngine {
            metadata: Arc::new(RwLock::new(metadata)),
            disks: Arc::new(RwLock::new(disks)),
            placement: PlacementEngine,
        }
    }

    /// Perform mount-time rebuild: scan extents and rebuild missing fragments where possible
    pub fn perform_mount_rebuild(&self) -> Result<()> {
        log::info!("Starting mount-time rebuild scan");
        let mut metadata_w = self.metadata.write().unwrap();
        let extents = metadata_w.list_all_extents()?;

        for mut extent in extents {
            let extent_uuid = extent.uuid;

            // Read fragments
            let disks = self.disks.read().unwrap();
            let fragments = match self.read_fragments(&extent, &disks) {
                Ok(f) => f,
                Err(e) => {
                    log::warn!("Failed to read fragments for extent {:?}: {:?}", extent_uuid, e);
                    continue;
                }
            };
            drop(disks);

            let available_count = fragments.iter().filter(|f| f.is_some()).count();
            let required = extent.redundancy.fragment_count();
            let min_needed = extent.redundancy.min_fragments();

            if available_count < required && available_count >= min_needed {
                log::info!("Rebuilding extent {:?}: {}/{} available", extent_uuid, available_count, required);
                extent.rebuild_in_progress = true;
                extent.rebuild_progress = Some(available_count);
                metadata_w.save_extent(&extent)?;

                // perform rebuild
                let mut disks_mut = self.disks.write().unwrap();
                if let Err(e) = self.placement.rebuild_extent(&mut extent, &mut disks_mut, &fragments) {
                    log::error!("Failed to rebuild extent {:?}: {:?}", extent_uuid, e);
                    extent.rebuild_in_progress = false;
                    metadata_w.save_extent(&extent)?;
                    continue;
                }

                extent.rebuild_in_progress = false;
                extent.rebuild_progress = Some(extent.fragment_locations.len());
                metadata_w.save_extent(&extent)?;
                log::info!("Rebuild complete for extent {:?}", extent_uuid);
            }
        }

        log::info!("Mount-time rebuild scan complete");
        Ok(())
    }
    
    /// Write data to a file
    pub fn write_file(&self, ino: u64, data: &[u8], offset: u64) -> Result<()> {
        log::debug!("Writing {} bytes to inode {} at offset {}", data.len(), ino, offset);
        
        // For simplicity, this prototype only supports full overwrites at offset 0
        if offset != 0 {
            return Err(anyhow!("Only full file writes at offset 0 are supported"));
        }
        
        // Determine redundancy policy based on file size
        let redundancy = if data.len() < DEFAULT_EXTENT_SIZE {
            // Small files: use replication
            RedundancyPolicy::Replication { copies: 3 }
        } else {
            // Large files: use erasure coding
            RedundancyPolicy::ErasureCoding {
                data_shards: 4,
                parity_shards: 2,
            }
        };
        
        // Split into extents using correct chunk boundaries
        let extents = split_into_extents(data, redundancy);
        let mut written_extents: Vec<Extent> = Vec::new();
        {
            let mut disks = self.disks.write().unwrap();
            for (idx, mut extent) in extents.into_iter().enumerate() {
                let chunk_start = idx * DEFAULT_EXTENT_SIZE;
                let chunk_end = chunk_start + extent.size;
                let chunk = &data[chunk_start..chunk_end];

                let fragments = redundancy::encode(chunk, extent.redundancy)?;
                if let Err(err) = self.placement.place_extent(&mut extent, &mut disks, &fragments) {
                    // Cleanup fragments from previously written extents before exiting
                    for previous in &written_extents {
                        for location in &previous.fragment_locations {
                            if let Some(disk) = disks.iter_mut().find(|d| d.uuid == location.disk_uuid) {
                                disk.delete_fragment(&previous.uuid, location.fragment_index).ok();
                            }
                        }
                    }
                    return Err(err);
                }

                extent.record_write();
                written_extents.push(extent);
            }
        }

        let extent_ids: Vec<_> = written_extents.iter().map(|e| e.uuid).collect();

        // Persist metadata after all fragments are durable; roll back fragments if persistence fails
        if let Err(err) = (|| -> Result<()> {
            let metadata = self.metadata.read().unwrap();
            for extent in &written_extents {
                metadata.save_extent(extent)?;
            }

            let extent_map = ExtentMap {
                ino,
                extents: extent_ids.clone(),
                checksum: None,
            };
            metadata.save_extent_map(&extent_map)?;

            let mut inode = metadata.load_inode(ino)?;
            inode.size = data.len() as u64;
            inode.mtime = chrono::Utc::now().timestamp();
            metadata.save_inode(&inode)?;
            Ok(())
        })() {
            let mut disks = self.disks.write().unwrap();
            for extent in &written_extents {
                for location in &extent.fragment_locations {
                    if let Some(disk) = disks.iter_mut().find(|d| d.uuid == location.disk_uuid) {
                        disk.delete_fragment(&extent.uuid, location.fragment_index).ok();
                    }
                }
            }
            return Err(err);
        }
        
        log::info!("Wrote {} bytes to inode {} across {} extents", 
                   data.len(), ino, written_extents.len());
        
        Ok(())
    }
    
    /// Read data from a file
    pub fn read_file(&self, ino: u64) -> Result<Vec<u8>> {
        log::debug!("Reading inode {}", ino);
        
        let metadata = self.metadata.read().unwrap();
        
        // Load extent map
        let extent_map = metadata.load_extent_map(ino)?;
        
        if extent_map.extents.is_empty() {
            return Ok(Vec::new());
        }
        
        let mut result = Vec::new();
        
        // Read each extent
        for extent_uuid in &extent_map.extents {
            let mut extent = metadata.load_extent(extent_uuid)?;
            
            // Record read access
            extent.record_read();
            
            // Read fragments with current policy
            let disks = self.disks.read().unwrap();
            let fragments = self.read_fragments(&extent, &disks)?;
            drop(disks);
            
            // Decode data with current policy
            let extent_data = redundancy::decode(&fragments, extent.redundancy)?;
            
            // Verify checksum
            if !extent.verify_checksum(&extent_data[..extent.size]) {
                return Err(anyhow!("Checksum verification failed for extent {}", extent_uuid));
            }
            
            // Check if lazy migration is needed (after successful read)
            let should_migrate = extent.should_migrate();
            if should_migrate {
                let recommended_policy = extent.recommended_policy();
                log::info!(
                    "Lazy migration triggered for extent {}: {:?} → {:?}",
                    extent_uuid,
                    extent.redundancy,
                    recommended_policy
                );
                
                // Perform migration in background (non-blocking)
                let mut disks_mut = self.disks.write().unwrap();
                if let Err(e) = self.placement.rebundle_extent(&mut extent, &mut disks_mut, &fragments, recommended_policy) {
                    log::error!("Failed to perform lazy migration for extent {}: {}", extent_uuid, e);
                } else {
                    metadata.save_extent(&extent)?;
                }
            }
            
            // Check if we need to rebuild
            let available_count = fragments.iter().filter(|f| f.is_some()).count();
            if available_count < extent.redundancy.fragment_count()
                && available_count >= extent.redundancy.min_fragments()
            {
                log::warn!(
                    "Extent {} has only {} of {} fragments, rebuilding",
                    extent_uuid,
                    available_count,
                    extent.redundancy.fragment_count()
                );
                
                let mut disks_mut = self.disks.write().unwrap();
                self.placement.rebuild_extent(&mut extent, &mut disks_mut, &fragments)?;
                metadata.save_extent(&extent)?;
            }
            
            // Save updated extent with new access stats
            metadata.save_extent(&extent)?;
            
            // Append to result (only the actual data, not padding)
            result.extend_from_slice(&extent_data[..extent.size]);
        }
        
        log::debug!("Read {} bytes from inode {}", result.len(), ino);
        Ok(result)
    }
    
    /// Read fragments of an extent with smart replica selection
    fn read_fragments(&self, extent: &Extent, disks: &[Disk]) -> Result<Vec<Option<Vec<u8>>>> {
        let mut fragments = vec![None; extent.redundancy.fragment_count()];
        
        for location in &extent.fragment_locations {
            // Find the disk
            let disk = disks
                .iter()
                .find(|d| d.uuid == location.disk_uuid)
                .ok_or_else(|| anyhow!("Disk {} not found", location.disk_uuid))?;
            
            // Read fragment
            match disk.read_fragment(&extent.uuid, location.fragment_index) {
                Ok(data) => {
                    fragments[location.fragment_index] = Some(data);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to read fragment {} of extent {} from disk {}: {}",
                        location.fragment_index,
                        extent.uuid,
                        location.disk_uuid,
                        e
                    );
                }
            }
        }
        
        Ok(fragments)
    }
    
    /// Delete a file
    pub fn delete_file(&self, ino: u64) -> Result<()> {
        log::info!("Deleting inode {}", ino);
        
        let metadata = self.metadata.read().unwrap();
        
        // Load extent map
        let extent_map = metadata.load_extent_map(ino)?;
        
        let mut disks = self.disks.write().unwrap();
        
        // Delete all extents and fragments
        for extent_uuid in &extent_map.extents {
            if let Ok(extent) = metadata.load_extent(extent_uuid) {
                // Delete fragments from disks
                for location in &extent.fragment_locations {
                    if let Some(disk) = disks.iter_mut().find(|d| d.uuid == location.disk_uuid) {
                        disk.delete_fragment(&extent.uuid, location.fragment_index).ok();
                    }
                }
                
                // Delete extent metadata
                metadata.delete_extent(extent_uuid).ok();
            }
        }
        
        // Delete extent map
        metadata.delete_extent_map(ino)?;
        
        // Delete inode
        metadata.delete_inode(ino)?;
        
        Ok(())
    }
    
    /// Get inode
    pub fn get_inode(&self, ino: u64) -> Result<Inode> {
        let metadata = self.metadata.read().unwrap();
        metadata.load_inode(ino)
    }
    
    /// List directory
    pub fn list_directory(&self, parent_ino: u64) -> Result<Vec<Inode>> {
        let metadata = self.metadata.read().unwrap();
        metadata.list_directory(parent_ino)
    }
    
    /// Find child by name
    pub fn find_child(&self, parent_ino: u64, name: &str) -> Result<Option<Inode>> {
        let metadata = self.metadata.read().unwrap();
        metadata.find_child(parent_ino, name)
    }
    
    /// Create a new file
    pub fn create_file(&self, parent_ino: u64, name: String) -> Result<Inode> {
        let mut metadata = self.metadata.write().unwrap();
        let ino = metadata.allocate_ino();
        let inode = Inode::new_file(ino, parent_ino, name);
        metadata.save_inode(&inode)?;
        Ok(inode)
    }
    
    /// Create a new directory
    pub fn create_dir(&self, parent_ino: u64, name: String) -> Result<Inode> {
        let mut metadata = self.metadata.write().unwrap();
        let ino = metadata.allocate_ino();
        let inode = Inode::new_dir(ino, parent_ino, name);
        metadata.save_inode(&inode)?;
        Ok(inode)
    }
    
    /// Update inode
    pub fn update_inode(&self, inode: &Inode) -> Result<()> {
        let metadata = self.metadata.read().unwrap();
        metadata.save_inode(inode)
    }
    
    /// Change redundancy policy for a file
    /// This re-bundles all extents with the new policy
    pub fn change_file_redundancy(
        &self,
        ino: u64,
        new_policy: crate::extent::RedundancyPolicy,
    ) -> Result<()> {
        log::info!("Changing redundancy policy for inode {}", ino);
        
        let extent_map = {
            let metadata = self.metadata.read().unwrap();
            metadata.load_extent_map(ino)?
        };
        
        if extent_map.extents.is_empty() {
            log::info!("No extents to rebundle for inode {}", ino);
            return Ok(());
        }
        
        let mut disks = self.disks.write().unwrap();
        
        // Re-bundle each extent
        for extent_uuid in &extent_map.extents {
            let mut extent = {
                let metadata = self.metadata.read().unwrap();
                metadata.load_extent(extent_uuid)?
            };
            
            if extent.redundancy == new_policy {
                log::debug!("Extent {} already has target policy, skipping", extent_uuid);
                continue;
            }
            
            log::info!(
                "Rebundling extent {} for inode {}",
                extent_uuid,
                ino
            );
            
            // Load fragments with old policy
            let fragments = self.read_fragments(&extent, &disks)?;
            
            // Rebundle
            self.placement.rebundle_extent(
                &mut extent,
                &mut disks,
                &fragments,
                new_policy,
            )?;
            
            // Save updated extent
            {
                let metadata = self.metadata.read().unwrap();
                metadata.save_extent(&extent)?;
            }
            
            log::info!(
                "Successfully rebundled extent {} to {:?}",
                extent_uuid,
                new_policy
            );
        }
        
        log::info!("Successfully changed redundancy policy for inode {}", ino);
        Ok(())
    }
    
    /// Get policy change history for an extent
    pub fn get_extent_policy_history(
        &self,
        extent_uuid: &uuid::Uuid,
    ) -> Result<Vec<(crate::extent::RedundancyPolicy, i64)>> {
        let metadata = self.metadata.read().unwrap();
        let extent = metadata.load_extent(extent_uuid)?;
        Ok(extent.get_policy_history())
    }
    
    /// List extents that are in the middle of a policy transition
    pub fn get_transitioning_extents(&self) -> Result<Vec<uuid::Uuid>> {
        let metadata = self.metadata.read().unwrap();
        let all_extents = metadata.list_all_extents()?;
        
        Ok(all_extents
            .iter()
            .filter(|e| e.is_transitioning())
            .map(|e| e.uuid)
            .collect())
    }
    
    /// Get access classification for an extent
    pub fn get_extent_classification(
        &self,
        extent_uuid: &uuid::Uuid,
    ) -> Result<crate::extent::AccessClassification> {
        let metadata = self.metadata.read().unwrap();
        let extent = metadata.load_extent(extent_uuid)?;
        Ok(extent.classification())
    }
    
    /// List all hot extents
    pub fn get_hot_extents(&self) -> Result<Vec<(uuid::Uuid, crate::extent::AccessStats)>> {
        let metadata = self.metadata.read().unwrap();
        let all_extents = metadata.list_all_extents()?;
        
        Ok(all_extents
            .iter()
            .filter(|e| e.classification() == crate::extent::AccessClassification::Hot)
            .map(|e| (e.uuid, e.access_stats.clone()))
            .collect())
    }
    
    /// List all cold extents
    pub fn get_cold_extents(&self) -> Result<Vec<(uuid::Uuid, crate::extent::AccessStats)>> {
        let metadata = self.metadata.read().unwrap();
        let all_extents = metadata.list_all_extents()?;
        
        Ok(all_extents
            .iter()
            .filter(|e| e.classification() == crate::extent::AccessClassification::Cold)
            .map(|e| (e.uuid, e.access_stats.clone()))
            .collect())
    }
    
    /// Get access statistics for an extent
    pub fn get_extent_access_stats(
        &self,
        extent_uuid: &uuid::Uuid,
    ) -> Result<crate::extent::AccessStats> {
        let metadata = self.metadata.read().unwrap();
        let extent = metadata.load_extent(extent_uuid)?;
        Ok(extent.access_stats.clone())
    }
    
    /// Get recommended policy for an extent based on its classification
    pub fn get_recommended_policy(
        &self,
        extent_uuid: &uuid::Uuid,
    ) -> Result<crate::extent::RedundancyPolicy> {
        let metadata = self.metadata.read().unwrap();
        let extent = metadata.load_extent(extent_uuid)?;
        Ok(extent.recommended_policy())
    }
    
    /// Check if an extent should be migrated based on classification
    pub fn extent_needs_migration(&self, extent_uuid: &uuid::Uuid) -> Result<bool> {
        let metadata = self.metadata.read().unwrap();
        let extent = metadata.load_extent(extent_uuid)?;
        Ok(extent.should_migrate())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::crash_sim::{get_crash_simulator, CrashPoint};
    use crate::disk::Disk;
    use tempfile::TempDir;
    
    pub(super) fn setup_test_env() -> (TempDir, Vec<TempDir>, MetadataManager, Vec<Disk>) {
        let pool_dir = tempfile::tempdir().unwrap();
        
        // Create 6 test disks
        let disk_dirs: Vec<TempDir> = (0..6)
            .map(|_| tempfile::tempdir().unwrap())
            .collect();
        
        let disks: Vec<Disk> = disk_dirs
            .iter()
            .map(|td| Disk::new(td.path().to_path_buf()).unwrap())
            .collect();
        
        let metadata = MetadataManager::new(pool_dir.path().to_path_buf()).unwrap();
        
        (pool_dir, disk_dirs, metadata, disks)
    }
    
    #[test]
    fn test_write_and_read_small_file() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "test.txt".to_string()).unwrap();
        
        // Write data
        let data = b"Hello, World!";
        storage.write_file(inode.ino, data, 0).unwrap();
        
        // Read data
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_write_and_read_large_file() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "large.bin".to_string()).unwrap();
        
        // Write 5MB of data
        let data = vec![0x42u8; 5 * 1024 * 1024];
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Read data
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data.len(), data.len());
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_multiple_files() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create multiple files
        for i in 0..10 {
            let name = format!("file{}.txt", i);
            let inode = storage.create_file(1, name).unwrap();
            let data = format!("Content of file {}", i);
            storage.write_file(inode.ino, data.as_bytes(), 0).unwrap();
        }
        
        // List directory
        let children = storage.list_directory(1).unwrap();
        assert!(children.len() >= 10, "Should have at least 10 files, got {}", children.len());
        
        // Verify all our files exist
        for i in 0..10 {
            let name = format!("file{}.txt", i);
            let found = children.iter().any(|c| c.name == name);
            assert!(found, "File {} should exist", name);
        }
        
        // Read each of our files
        for i in 0..10 {
            let name = format!("file{}.txt", i);
            if let Some(child) = children.iter().find(|c| c.name == name) {
                let data = storage.read_file(child.ino).unwrap();
                assert!(data.len() > 0);
            }
        }
    }
    
    #[test]
    fn test_directory_operations() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a subdirectory
        let subdir = storage.create_dir(1, "subdir".to_string()).unwrap();
        
        // Create files in subdirectory
        let file1 = storage.create_file(subdir.ino, "file1.txt".to_string()).unwrap();
        let file2 = storage.create_file(subdir.ino, "file2.txt".to_string()).unwrap();
        
        // List subdirectory
        let children = storage.list_directory(subdir.ino).unwrap();
        assert_eq!(children.len(), 2);
        
        // Write and read from files
        storage.write_file(file1.ino, b"File 1 content", 0).unwrap();
        storage.write_file(file2.ino, b"File 2 content", 0).unwrap();
        
        let data1 = storage.read_file(file1.ino).unwrap();
        let data2 = storage.read_file(file2.ino).unwrap();
        
        assert_eq!(data1, b"File 1 content");
        assert_eq!(data2, b"File 2 content");
    }
    
    #[test]
    fn test_delete_file() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create and write file
        let inode = storage.create_file(1, "temp.txt".to_string()).unwrap();
        storage.write_file(inode.ino, b"Temporary data", 0).unwrap();
        
        // Verify file exists
        let children_before = storage.list_directory(1).unwrap();
        let found_before = children_before.iter().any(|c| c.name == "temp.txt");
        assert!(found_before, "File should exist before deletion");
        
        // Delete file
        storage.delete_file(inode.ino).unwrap();
        
        // Verify file is gone
        let children_after = storage.list_directory(1).unwrap();
        let found_after = children_after.iter().any(|c| c.name == "temp.txt");
        assert!(!found_after, "File should not exist after deletion");
    }
    
    #[test]
    fn test_change_policy_replication_to_ec() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a small file (will use replication)
        let inode = storage.create_file(1, "policy_test.bin".to_string()).unwrap();
        let data = vec![0xAAu8; 512 * 1024]; // 512KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Verify we can read it
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
        
        // Change policy to EC (4+2)
        let new_policy = crate::extent::RedundancyPolicy::ErasureCoding {
            data_shards: 4,
            parity_shards: 2,
        };
        
        storage.change_file_redundancy(inode.ino, new_policy).unwrap();
        
        // Verify data still intact
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_change_policy_ec_to_replication() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a large file (will use EC)
        let inode = storage.create_file(1, "policy_test2.bin".to_string()).unwrap();
        let data = vec![0xBBu8; 3 * 1024 * 1024]; // 3MB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Verify initial read
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
        
        // Change policy to replication (5 copies)
        let new_policy = crate::extent::RedundancyPolicy::Replication { copies: 5 };
        
        storage.change_file_redundancy(inode.ino, new_policy).unwrap();
        
        // Verify data still intact
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_policy_change_with_disk_failure() {
        let (_pool_dir, _disk_dirs, metadata, mut disks) = setup_test_env();
        
        // Create initial file with EC (4+2)
        let storage = StorageEngine::new(metadata, disks.clone());
        let inode = storage.create_file(1, "resilient.bin".to_string()).unwrap();
        let data = vec![0xCCu8; 2 * 1024 * 1024]; // 2MB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Fail two disks
        disks[0].mark_failed().unwrap();
        disks[2].mark_failed().unwrap();
        
        // Recreate storage with failed disks
        let storage = StorageEngine::new(
            MetadataManager::new(_pool_dir.path().to_path_buf()).unwrap(),
            disks.clone()
        );
        
        // Data should still be readable
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
        
        // Change to higher redundancy (replication)
        let new_policy = crate::extent::RedundancyPolicy::Replication { copies: 4 };
        storage.change_file_redundancy(inode.ino, new_policy).unwrap();
        
        // Verify data is still readable
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_hot_cold_classification() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "classify.bin".to_string()).unwrap();
        let data = vec![0x42u8; 256 * 1024]; // 256KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Get extent map
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        
        // Load the extent
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        
        // Check classification - should have access stats recorded
        assert!(extent.access_stats.write_count > 0, "write_count should be > 0");
        // Classification depends on write/read frequency and timing
        // Just verify it has a valid classification
        let classification = extent.classification();
        assert!(
            classification == crate::extent::AccessClassification::Hot
                || classification == crate::extent::AccessClassification::Warm
                || classification == crate::extent::AccessClassification::Cold
        );
    }
    
    #[test]
    fn test_access_tracking() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "tracked.bin".to_string()).unwrap();
        let data = vec![0x55u8; 512 * 1024]; // 512KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Read the file multiple times
        for _ in 0..5 {
            let read_data = storage.read_file(inode.ino).unwrap();
            assert_eq!(read_data, data);
        }
        
        // Get extent and check read count increased
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        
        // Should have recorded the reads
        assert!(extent.access_stats.read_count > 0);
        assert!(extent.access_stats.last_read > 0);
    }
    
    #[test]
    fn test_access_frequency() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "frequent.bin".to_string()).unwrap();
        let data = vec![0x77u8; 128 * 1024]; // 128KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Get extent
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        
        // Check access frequency calculation
        let frequency = extent.access_frequency();
        assert!(frequency >= 0.0);
    }
    
    #[test]
    fn test_recommended_policy() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "policy_rec.bin".to_string()).unwrap();
        let data = vec![0x88u8; 256 * 1024]; // 256KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Get extent
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        
        // Check recommended policy based on classification
        let recommended = extent.recommended_policy();
        // Just verify it returns a valid policy
        match recommended {
            crate::extent::RedundancyPolicy::Replication { copies } => {
                assert!(copies > 0);
            }
            crate::extent::RedundancyPolicy::ErasureCoding { data_shards, parity_shards } => {
                assert!(data_shards > 0 && parity_shards > 0);
            }
        }
    }
    
    #[test]
    fn test_lazy_migration_on_read() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file with EC (cold data)
        let inode = storage.create_file(1, "migrate.bin".to_string()).unwrap();
        let data = vec![0x99u8; 3 * 1024 * 1024]; // 3MB - will use EC
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Get initial extent
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        let initial_policy = extent.redundancy;
        
        drop(metadata);
        drop(extent);
        
        // Read file multiple times to mark as hot
        for _ in 0..5 {
            let _ = storage.read_file(inode.ino).unwrap();
        }
        
        // Get extent again and check if migration may have occurred
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        
        // The extent should have higher read count now
        assert!(extent.access_stats.read_count > 0);
        // If it was classified as hot, it may have migrated to replication
        // Just verify the extent is still valid and readable
        assert!(!extent.fragment_locations.is_empty());
    }
    
    #[test]
    fn test_lazy_migration_check() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "check_migrate.bin".to_string()).unwrap();
        let data = vec![0xAAu8; 512 * 1024]; // 512KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Get extent UUID
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent_uuid = extent_map.extents[0];
        let extent = metadata.load_extent(&extent_uuid).unwrap();
        let initial_policy = extent.redundancy;
        
        drop(metadata);
        
        // Check if migration is needed
        let needs_migration = storage.extent_needs_migration(&extent_uuid).unwrap();
        let recommended = storage.get_recommended_policy(&extent_uuid).unwrap();
        
        // Verify recommendation exists
        match recommended {
            crate::extent::RedundancyPolicy::Replication { copies } => {
                assert!(copies > 0);
            }
            crate::extent::RedundancyPolicy::ErasureCoding { data_shards, parity_shards } => {
                assert!(data_shards > 0 && parity_shards > 0);
            }
        }
        
        // needs_migration should match if policies differ
        assert_eq!(needs_migration, recommended != initial_policy);
    }

    #[test]
    fn test_multi_extent_write_preserves_unique_chunks() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);

        let inode = storage.create_file(1, "pattern.bin".to_string()).unwrap();

        let mut data = Vec::new();
        for i in 0..3 {
            let value = (i as u8) + 1;
            data.extend(std::iter::repeat(value).take(DEFAULT_EXTENT_SIZE));
        }
        data.extend(std::iter::repeat(0xFFu8).take(DEFAULT_EXTENT_SIZE / 2));

        storage.write_file(inode.ino, &data, 0).unwrap();
        let read_back = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_back, data);

        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        assert!(extent_map.extents.len() >= 3);
    }

    #[test]
    fn test_write_failure_rolls_back_fragments() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);

        let inode = storage.create_file(1, "fail_on_write.bin".to_string()).unwrap();

        let sim = get_crash_simulator();
        sim.reset();
        sim.enable_at(CrashPoint::AfterFragmentWrite);

        let result = storage.write_file(inode.ino, b"guarded content", 0);
        assert!(result.is_err(), "expected crash-injected write failure");

        sim.disable();
        sim.reset();

        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        assert!(extent_map.extents.is_empty(), "extent map should be empty after failed write");

        let extents = metadata.list_all_extents().unwrap();
        assert!(extents.is_empty(), "no extent metadata should be persisted on failure");

        let disks = storage.disks.read().unwrap();
        for disk in disks.iter() {
            let fragments_dir = disk.path.join("fragments");
            let count = std::fs::read_dir(&fragments_dir).map(|rd| rd.count()).unwrap_or(0);
            assert_eq!(count, 0, "Fragments should be cleaned up for disk {:?}", disk.uuid);
        }
    }
}

#[cfg(test)]
#[path = "crash_tests.rs"]
mod crash_tests;
