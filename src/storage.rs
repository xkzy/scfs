use anyhow::{anyhow, Result};
use std::sync::{Arc, RwLock, Mutex};
use std::thread;

use crate::disk::Disk;
use crate::extent::{split_into_extents, Extent, RedundancyPolicy, AccessStats, AccessClassification, DEFAULT_EXTENT_SIZE};
use crate::hmm_classifier::HmmClassifier;
use crate::metadata::{ExtentMap, Inode, MetadataManager};
use crate::placement::PlacementEngine;
use crate::redundancy;
use crate::metrics::Metrics;
use crate::scheduler::{ReplicaSelector, ReplicaSelectionStrategy};

/// Storage engine handling read/write operations
pub struct StorageEngine {
    metadata: Arc<RwLock<MetadataManager>>,
    disks: Arc<RwLock<Vec<Arc<Mutex<Disk>>>>>,
    placement: PlacementEngine,
    metrics: Arc<Metrics>,
}

impl StorageEngine {
    pub fn new(metadata: MetadataManager, disks: Vec<Disk>) -> Self {
        let disks = disks.into_iter().map(|d| Arc::new(Mutex::new(d))).collect();
        StorageEngine {
            metadata: Arc::new(RwLock::new(metadata)),
            disks: Arc::new(RwLock::new(disks)),
            placement: PlacementEngine,
            metrics: Arc::new(Metrics::new()),
        }
    }
    
    pub fn with_metrics(metadata: MetadataManager, disks: Vec<Disk>, metrics: Arc<Metrics>) -> Self {
        let disks = disks.into_iter().map(|d| Arc::new(Mutex::new(d))).collect();
        StorageEngine {
            metadata: Arc::new(RwLock::new(metadata)),
            disks: Arc::new(RwLock::new(disks)),
            placement: PlacementEngine,
            metrics,
        }
    }
    
    pub fn metrics(&self) -> Arc<Metrics> {
        self.metrics.clone()
    }
    
    /// Get a reference to the metadata manager
    pub fn metadata(&self) -> Arc<RwLock<MetadataManager>> {
        Arc::clone(&self.metadata)
    }
    
    /// Get a copy of the current disk list
    pub fn get_disks(&self) -> Vec<Disk> {
        self.disks.read().unwrap().iter().map(|d| d.lock().unwrap().clone()).collect()
    }
    
    /// Read extent data by UUID
    pub fn read_extent(&self, extent_uuid: uuid::Uuid) -> Result<Vec<u8>> {
        let metadata = self.metadata.read().unwrap();
        let extent = metadata.load_extent(&extent_uuid)?;
        drop(metadata);
        
        let disks = self.disks.read().unwrap();
        let fragments = self.read_fragments(&extent, &*disks)?;
        drop(disks);
        
        // Reconstruct data from fragments
        redundancy::decode(&fragments, extent.redundancy)
    }
    
    /// Write extent data and return the extent
    pub fn write_extent(&self, data: &[u8], policy: RedundancyPolicy) -> Result<Extent> {
        use uuid::Uuid;
        use blake3;
        
        let extent_uuid = Uuid::new_v4();
        let checksum_hash = blake3::hash(data);
        let checksum: [u8; 32] = checksum_hash.into();
        
        // Create extent object first
        let now = chrono::Utc::now().timestamp();
        let mut extent = Extent {
            uuid: extent_uuid,
            size: data.len(),
            checksum,
            redundancy: policy,
            fragment_locations: Vec::new(),
            previous_policy: None,
            policy_transitions: Vec::new(),
            last_policy_change: None,
            access_stats: AccessStats {
                read_count: 0,
                write_count: 1,
                last_read: 0,
                last_write: now,
                created_at: now,
                classification: AccessClassification::Cold,
                hmm_classifier: Some(HmmClassifier::new()),
            },
            rebuild_in_progress: false,
            rebuild_progress: None,
            generation: 0,
        };
        
        // Encode fragments based on redundancy policy
        let fragments = redundancy::encode(data, policy)?;
        
        // Place extent on disks
        let disks = self.disks.write().unwrap();
        self.placement.place_extent(&mut extent, &*disks, &fragments)?;
        
        // Record metrics
        for fragment in &fragments {
            self.metrics.record_disk_write(fragment.len() as u64);
        }
        
        Ok(extent)
    }
    
    /// Delete an extent and its fragments
    pub fn delete_extent(&self, extent_uuid: uuid::Uuid) -> Result<()> {
        let metadata = self.metadata.read().unwrap();
        let extent = metadata.load_extent(&extent_uuid)?;
        drop(metadata);
        
        let disks = self.disks.write().unwrap();
        for location in &extent.fragment_locations {
            if let Some(disk_arc) = disks.iter().find(|d| d.lock().unwrap().uuid == location.disk_uuid) {
                let _ = disk_arc.lock().unwrap().delete_fragment(&extent_uuid, location.fragment_index);
            }
        }
        
        Ok(())
    }

    /// Perform mount-time rebuild: scan extents and rebuild missing fragments where possible
    pub fn perform_mount_rebuild(&self) -> Result<()> {
        log::info!("Starting mount-time rebuild scan");
        let metadata_w = self.metadata.write().unwrap();
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

            // Determine if rebuild is needed due to missing fragments
            let needs_rebuild = available_count < required && available_count >= min_needed;

            // Also consider draining disks: if any fragment resides on a draining disk, attempt to migrate
            let disks_snapshot = self.disks.read().unwrap();
            let draining_disk_uuids: Vec<uuid::Uuid> = disks_snapshot
                .iter()
                .filter_map(|d| {
                    let d = d.lock().unwrap();
                    if d.health == crate::disk::DiskHealth::Draining { Some(d.uuid) } else { None }
                })
                .collect();
            drop(disks_snapshot);

            let has_draining_fragment = extent
                .fragment_locations
                .iter()
                .any(|loc| draining_disk_uuids.contains(&loc.disk_uuid));

            if needs_rebuild || has_draining_fragment {
                if has_draining_fragment {
                    log::info!("Migrating fragments for extent {:?} away from draining disks", extent_uuid);
                } else {
                    log::info!("Rebuilding extent {:?}: {}/{} available", extent_uuid, available_count, required);
                }

                extent.rebuild_in_progress = true;
                extent.rebuild_progress = Some(available_count);
                metadata_w.save_extent(&extent)?;

                // perform rebuild/migration
                self.metrics.record_rebuild_start();
                let disks_mut = self.disks.write().unwrap();

                // When migrating from draining disks, prefer to preserve existing fragments and only replace those on draining disks
                let rebuild_result = if has_draining_fragment && !needs_rebuild {
                    // Read existing fragments and rewrite missing/draining ones
                    self.placement.rebuild_extent(&mut extent, &*disks_mut, &fragments)
                } else {
                    self.placement.rebuild_extent(&mut extent, &*disks_mut, &fragments)
                };

                if let Err(e) = rebuild_result {
                    self.metrics.record_rebuild_failure();
                    log::error!("Failed to rebuild/migrate extent {:?}: {:?}", extent_uuid, e);
                    extent.rebuild_in_progress = false;
                    metadata_w.save_extent(&extent)?;
                    continue;
                }

                self.metrics.record_rebuild_success(extent.size as u64);
                extent.rebuild_in_progress = false;
                extent.rebuild_progress = Some(extent.fragment_locations.len());
                metadata_w.save_extent(&extent)?;
                log::info!("Rebuild/migration complete for extent {:?}", extent_uuid);
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
        
        // Acquire disks write lock, collect references, then release before spawning
        let disks_arc = self.disks.clone();
        let disk_refs: Vec<Arc<Mutex<Disk>>> = {
            let disks = disks_arc.write().unwrap();
            disks.iter().map(|d| d.clone()).collect()
        }; // RwLock is released here
        
        #[cfg(test)]
        eprintln!("[WRITE_FILE DEBUG] starting placement for {} extents", disk_refs.len());

        for (idx, mut extent) in extents.into_iter().enumerate() {
            let chunk_start = idx * DEFAULT_EXTENT_SIZE;
            let chunk_end = chunk_start + extent.size;
            let chunk = &data[chunk_start..chunk_end];

            let fragments = redundancy::encode(chunk, extent.redundancy)?;
            if let Err(err) = self.placement.place_extent(&mut extent, &disk_refs, &fragments) {
                // Cleanup fragments from previously written extents before exiting
                for previous in &written_extents {
                    for location in &previous.fragment_locations {
                        if let Some(disk_arc) = disk_refs.iter().find(|d| d.lock().unwrap().uuid == location.disk_uuid) {
                            disk_arc.lock().unwrap().delete_fragment(&previous.uuid, location.fragment_index).ok();
                        }
                    }
                }
                return Err(err);
            }

            extent.record_write();
            written_extents.push(extent);
        }

        #[cfg(test)]
        eprintln!("[WRITE_FILE DEBUG] placement complete; {} extents", written_extents.len());

        let extent_ids: Vec<_> = written_extents.iter().map(|e| e.uuid).collect();

        // Persist metadata after all fragments are durable; roll back fragments if persistence fails
        if let Err(err) = (|| -> Result<()> {
            #[cfg(test)]
            eprintln!("[WRITE_FILE DEBUG] persisting metadata: {} extents", written_extents.len());
            let metadata = self.metadata.write().unwrap();
            for extent in &written_extents {
                #[cfg(test)]
                eprintln!("[WRITE_FILE DEBUG] save_extent {}", extent.uuid);
                metadata.save_extent(extent)?;
            }

            let extent_map = ExtentMap {
                ino,
                extents: extent_ids.clone(),
                checksum: None,
            };
            #[cfg(test)]
            eprintln!("[WRITE_FILE DEBUG] save_extent_map ino={}", ino);
            metadata.save_extent_map(&extent_map)?;

            let mut inode = metadata.load_inode(ino)?;
            inode.size = data.len() as u64;
            inode.mtime = chrono::Utc::now().timestamp();
            #[cfg(test)]
            eprintln!("[WRITE_FILE DEBUG] save_inode ino={}", ino);

            metadata.save_inode(&inode)?;
            Ok(())
        })() {
            let disks = self.disks.write().unwrap();
            for extent in &written_extents {
                for location in &extent.fragment_locations {
                    if let Some(disk_arc) = disks.iter().find(|d| d.lock().unwrap().uuid == location.disk_uuid) {
                        disk_arc.lock().unwrap().delete_fragment(&extent.uuid, location.fragment_index).ok();
                    }
                }
            }
            return Err(err);
        }
        
        // Record metrics for write operation
        self.metrics.record_disk_write(data.len() as u64);
        
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
                let disks_mut = self.disks.write().unwrap();
                if let Err(e) = self.placement.rebundle_extent(&mut extent, &*disks_mut, &fragments, recommended_policy) {
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
                
                self.metrics.record_rebuild_start();
                let disks_mut = self.disks.write().unwrap();
                match self.placement.rebuild_extent(&mut extent, &*disks_mut, &fragments) {
                    Ok(_) => {
                        self.metrics.record_rebuild_success(extent.size as u64);
                    }
                    Err(e) => {
                        self.metrics.record_rebuild_failure();
                        return Err(e);
                    }
                }
                metadata.save_extent(&extent)?;
            }
            
            // Save updated extent with new access stats
            metadata.save_extent(&extent)?;
            
            // Append to result (only the actual data, not padding)
            result.extend_from_slice(&extent_data[..extent.size]);
        }
        
        // Record metrics for read operation
        self.metrics.record_disk_read(result.len() as u64);
        
        log::debug!("Read {} bytes from inode {}", result.len(), ino);
        Ok(result)
    }
    
    /// Read fragments of an extent with smart replica selection
    fn read_fragments(&self, extent: &Extent, disks: &[Arc<Mutex<Disk>>]) -> Result<Vec<Option<Vec<u8>>>> {
        let fragment_count = extent.redundancy.fragment_count();
        let mut fragments = vec![None; fragment_count];
        
        // Snapshot disk state without holding locks during reads
        let disk_snapshots: Vec<Disk> = disks
            .iter()
            .map(|d| d.lock().unwrap().clone())
            .collect();
        let disk_refs: Vec<&Disk> = disk_snapshots.iter().collect();
        let disk_lookup: Vec<(uuid::Uuid, Arc<Mutex<Disk>>)> = disks
            .iter()
            .map(|d| {
                let uuid = d.lock().unwrap().uuid;
                (uuid, d.clone())
            })
            .collect();

        // Group locations by fragment index to support smart replica selection
        let mut fragments_by_index: Vec<Vec<(uuid::Uuid, usize)>> = 
            vec![Vec::new(); fragment_count];
        
        for location in &extent.fragment_locations {
            fragments_by_index[location.fragment_index].push((location.disk_uuid, location.fragment_index));
        }
        
        // Collect read tasks for parallel execution
        let mut read_tasks = Vec::new();
        
        // For each fragment index, try to read from best available replica
        let strategy = ReplicaSelectionStrategy::Smart;
        for fragment_index in 0..fragment_count {
            let locations = &fragments_by_index[fragment_index];
            
            // Try to select best replica for this fragment
            let selected_disk = if let Some((disk_uuid, frag_idx)) = ReplicaSelector::select_replica(extent, &disk_refs, strategy) {
                if frag_idx == fragment_index {
                    disk_lookup
                        .iter()
                        .find(|(uuid, _)| *uuid == disk_uuid)
                        .map(|(_, arc)| arc.clone())
                } else {
                    None
                }
            } else {
                None
            };
            
            // If no smart selection, fallback to any available replica
            let disk = selected_disk.or_else(|| {
                locations.iter().find_map(|(disk_uuid, _)| {
                    disk_lookup
                        .iter()
                        .find(|(uuid, _)| uuid == disk_uuid)
                        .map(|(_, arc)| arc.clone())
                })
            });
            
            if let Some(disk) = disk {
                let extent_uuid = extent.uuid;
                let disk_clone = disk.clone();
                let task = thread::spawn(move || {
                    disk_clone.lock().unwrap().read_fragment(&extent_uuid, fragment_index)
                });
                read_tasks.push((fragment_index, task));
            }
        }
        
        // Execute reads in parallel and collect results
        for (fragment_index, task) in read_tasks {
            match task.join() {
                Ok(Ok(data)) => {
                    fragments[fragment_index] = Some(data);
                }
                Ok(Err(e)) => {
                    self.metrics.record_disk_error();
                    log::warn!(
                        "Failed to read fragment {} of extent {}: {}",
                        fragment_index,
                        extent.uuid,
                        e
                    );
                }
                Err(e) => {
                    log::error!("Task join error for fragment {}: {:?}", fragment_index, e);
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
        
        let disks = self.disks.write().unwrap();
        
        // Delete all extents and fragments
        for extent_uuid in &extent_map.extents {
            if let Ok(extent) = metadata.load_extent(extent_uuid) {
                // Delete fragments from disks
                for location in &extent.fragment_locations {
                    if let Some(disk_arc) = disks.iter().find(|d| d.lock().unwrap().uuid == location.disk_uuid) {
                        disk_arc.lock().unwrap().delete_fragment(&extent.uuid, location.fragment_index).ok();
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
        println!("DEBUG: Starting change_file_redundancy for inode {}", ino);
        log::info!("Changing redundancy policy for inode {}", ino);
        
        let extent_map = {
            let metadata = self.metadata.read().unwrap();
            metadata.load_extent_map(ino)?
        };
        println!("DEBUG: Loaded extent_map with {} extents", extent_map.extents.len());
        
        if extent_map.extents.is_empty() {
            log::info!("No extents to rebundle for inode {}", ino);
            return Ok(());
        }
        
        let disks = self.disks.write().unwrap();
        println!("DEBUG: Acquired disks lock");
        
        // Re-bundle each extent
        for extent_uuid in &extent_map.extents {
            println!("DEBUG: Processing extent {}", extent_uuid);
            let mut extent = {
                let metadata = self.metadata.read().unwrap();
                metadata.load_extent(extent_uuid)?
            };
            println!("DEBUG: Loaded extent {}", extent_uuid);
            
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
            println!("DEBUG: Calling read_fragments for extent {}", extent_uuid);
            let fragments = self.read_fragments(&extent, &disks)?;
            println!("DEBUG: read_fragments completed for extent {}", extent_uuid);
            
            // Rebundle
            self.placement.rebundle_extent(
                &mut extent,
                &*disks,
                &fragments,
                new_policy,
            )?;
            println!("DEBUG: rebundle_extent completed for extent {}", extent_uuid);
            
            // Save updated extent
            {
                let metadata = self.metadata.read().unwrap();
                metadata.save_extent(&extent)?;
            }
            println!("DEBUG: Saved extent {}", extent_uuid);
            
            log::info!(
                "Successfully rebundled extent {} to {:?}",
                extent_uuid,
                new_policy
            );
        }
        
        log::info!("Successfully changed redundancy policy for inode {}", ino);
        println!("DEBUG: change_file_redundancy completed for inode {}", ino);
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

impl crate::storage_engine::FilesystemInterface for StorageEngine {
    fn read_file(&self, ino: u64) -> Result<Vec<u8>> {
        self.read_file(ino)
    }

    fn write_file(&self, ino: u64, data: &[u8], offset: u64) -> Result<()> {
        self.write_file(ino, data, offset)
    }

    fn create_file(&self, parent_ino: u64, name: String) -> Result<Inode> {
        self.create_file(parent_ino, name)
    }

    fn create_dir(&self, parent_ino: u64, name: String) -> Result<Inode> {
        self.create_dir(parent_ino, name)
    }

    fn delete_file(&self, ino: u64) -> Result<()> {
        self.delete_file(ino)
    }

    fn delete_dir(&self, ino: u64) -> Result<()> {
        // For now, assume delete_file works for directories too
        // In a real implementation, we'd check if directory is empty
        self.delete_file(ino)
    }

    fn get_inode(&self, ino: u64) -> Result<Inode> {
        self.get_inode(ino)
    }

    fn list_directory(&self, parent_ino: u64) -> Result<Vec<Inode>> {
        self.list_directory(parent_ino)
    }

    fn find_child(&self, parent_ino: u64, name: &str) -> Result<Option<Inode>> {
        self.find_child(parent_ino, name)
    }

    fn update_inode(&self, inode: &Inode) -> Result<()> {
        self.update_inode(inode)
    }

    fn stat(&self) -> Result<crate::storage_engine::FilesystemStats> {
        // For now, return basic stats. In a full implementation, we'd track these metrics.
        // This is a simplified version that doesn't scan all metadata.
        let disks = self.disks.read().unwrap();
        let total_space: u64 = disks.iter().map(|d| d.lock().unwrap().capacity_bytes).sum();
        let used_space: u64 = disks.iter().map(|d| d.lock().unwrap().used_bytes).sum();

        Ok(crate::storage_engine::FilesystemStats {
            total_files: 0, // TODO: implement proper counting
            total_dirs: 1,  // at least root
            total_size: 0,  // TODO: implement proper summing
            used_space,
            free_space: total_space - used_space,
        })
    }
}



#[cfg(test)]
#[path = "crash_tests.rs"]
mod crash_tests;
