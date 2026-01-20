use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[cfg(test)]
use crate::crash_sim::{check_crash_point, CrashPoint};

/// Represents a storage disk (backed by a directory)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Disk {
    pub uuid: Uuid,
    pub path: PathBuf,
    pub capacity_bytes: u64,
    pub used_bytes: u64,
    pub health: DiskHealth,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiskHealth {
    Healthy,
    Draining,  // Being removed gracefully
    Failed,    // Unavailable
}

/// Guard to ensure temporary fragment files are cleaned up on failure
struct TempFragmentGuard {
    path: PathBuf,
    committed: bool,
}

impl TempFragmentGuard {
    fn new(path: PathBuf) -> Self {
        TempFragmentGuard {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TempFragmentGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Disk {
    /// Create a new disk from a directory path
    pub fn new(path: PathBuf) -> Result<Self> {
        // Create disk metadata
        let uuid = Uuid::new_v4();
        let capacity_bytes = Self::get_available_space(&path)?;
        
        // Create fragments directory
        let fragments_dir = path.join("fragments");
        fs::create_dir_all(&fragments_dir)
            .context("Failed to create fragments directory")?;
        
        let disk = Disk {
            uuid,
            path,
            capacity_bytes,
            used_bytes: 0,
            health: DiskHealth::Healthy,
        };
        
        disk.save()?;
        Ok(disk)
    }
    
    /// Load disk from its directory
    pub fn load(path: &Path) -> Result<Self> {
        let metadata_path = path.join("disk.json");
        let contents = fs::read_to_string(&metadata_path)
            .context("Failed to read disk metadata")?;
        let disk: Disk = serde_json::from_str(&contents)
            .context("Failed to parse disk metadata")?;
        Ok(disk)
    }
    
    /// Save disk metadata
    pub fn save(&self) -> Result<()> {
        let metadata_path = self.path.join("disk.json");
        let contents = serde_json::to_string_pretty(self)
            .context("Failed to serialize disk metadata")?;
        
        // Atomic write: write to temp file, then rename
        let temp_path = metadata_path.with_extension("json.tmp");
        fs::write(&temp_path, contents)
            .context("Failed to write disk metadata")?;
        fs::rename(&temp_path, &metadata_path)
            .context("Failed to commit disk metadata")?;
        
        Ok(())
    }
    
    /// Get available space in the directory
    fn get_available_space(path: &Path) -> Result<u64> {
        // Use statfs to get filesystem stats
        let statvfs = nix::sys::statvfs::statvfs(path)
            .context("Failed to get filesystem stats")?;
        
        let available_bytes = statvfs.blocks_available() * statvfs.block_size();
        Ok(available_bytes)
    }
    
    /// Update used space
    pub fn update_usage(&mut self) -> Result<()> {
        let fragments_dir = self.path.join("fragments");
        let mut total_size = 0u64;
        
        if fragments_dir.exists() {
            for entry in walkdir::WalkDir::new(&fragments_dir) {
                let entry = entry?;
                if entry.file_type().is_file() {
                    total_size += entry.metadata()?.len();
                }
            }
        }
        
        self.used_bytes = total_size;
        self.save()?;
        Ok(())
    }
    
    /// Check if disk has enough free space
    pub fn has_space(&self, required_bytes: u64) -> bool {
        if self.health != DiskHealth::Healthy {
            return false;
        }
        
        let free_bytes = self.capacity_bytes.saturating_sub(self.used_bytes);
        free_bytes >= required_bytes
    }
    
    /// Get path for a fragment
    pub fn fragment_path(&self, extent_uuid: &Uuid, fragment_index: usize) -> PathBuf {
        self.path
            .join("fragments")
            .join(format!("{}-{}.frag", extent_uuid, fragment_index))
    }
    
    /// Write a fragment to disk
    pub fn write_fragment(
        &mut self,
        extent_uuid: &Uuid,
        fragment_index: usize,
        data: &[u8],
    ) -> Result<()> {
        let fragment_path = self.fragment_path(extent_uuid, fragment_index);
        
        #[cfg(test)]
        check_crash_point(CrashPoint::BeforeFragmentWrite)?;
        
         let temp_path = fragment_path.with_extension("frag.tmp");
         let mut guard = TempFragmentGuard::new(temp_path.clone());
         {
             let mut file = File::create(&temp_path)
                 .context("Failed to open temp fragment file")?;
             file.write_all(data)
                 .context("Failed to write fragment")?;
             file.sync_all()
                 .context("Failed to fsync fragment data")?;
         }
         
         #[cfg(test)]
         check_crash_point(CrashPoint::AfterFragmentWrite)?;
         
         fs::rename(&temp_path, &fragment_path)
             .context("Failed to commit fragment")?;
         guard.commit();

         let written = fs::read(&fragment_path)
             .context("Failed to verify fragment readback")?;
         if written != data {
             return Err(anyhow!(
                 "Fragment verification failed for {}",
                 fragment_path.display()
             ));
         }

         if let Some(parent) = fragment_path.parent() {
             if let Ok(dir) = File::open(parent) {
                 let _ = dir.sync_all();
             }
         }
        
        self.used_bytes += data.len() as u64;
        self.save()?;
        
        Ok(())
    }
    
    /// Read a fragment from disk
    pub fn read_fragment(&self, extent_uuid: &Uuid, fragment_index: usize) -> Result<Vec<u8>> {
        let fragment_path = self.fragment_path(extent_uuid, fragment_index);
        fs::read(&fragment_path).context("Failed to read fragment")
    }

    /// Check if a fragment exists
    pub fn has_fragment(&self, extent_uuid: &Uuid, fragment_index: usize) -> bool {
        self.fragment_path(extent_uuid, fragment_index).exists()
    }
    
    /// Delete a fragment
    pub fn delete_fragment(&mut self, extent_uuid: &Uuid, fragment_index: usize) -> Result<()> {
        let fragment_path = self.fragment_path(extent_uuid, fragment_index);
        if fragment_path.exists() {
            let size = fs::metadata(&fragment_path)?.len();
            fs::remove_file(&fragment_path)?;
            self.used_bytes = self.used_bytes.saturating_sub(size);
            self.save()?;
        }
        Ok(())
    }
    
    /// Mark disk as draining (graceful removal)
    pub fn mark_draining(&mut self) -> Result<()> {
        self.health = DiskHealth::Draining;
        self.save()
    }
    
    /// Mark disk as failed
    pub fn mark_failed(&mut self) -> Result<()> {
        self.health = DiskHealth::Failed;
        self.save()
    }
}

/// Disk pool manager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskPool {
    pub disk_paths: Vec<PathBuf>,
}

impl DiskPool {
    pub fn new() -> Self {
        DiskPool {
            disk_paths: Vec::new(),
        }
    }
    
    pub fn add_disk(&mut self, path: PathBuf) {
        if !self.disk_paths.contains(&path) {
            self.disk_paths.push(path);
        }
    }
    
    pub fn remove_disk(&mut self, path: &Path) {
        self.disk_paths.retain(|p| p != path);
    }
    
    /// Load all disks in the pool
    pub fn load_disks(&self) -> Result<Vec<Disk>> {
        let mut disks = Vec::new();
        for path in &self.disk_paths {
            match Disk::load(path) {
                Ok(disk) => disks.push(disk),
                Err(e) => {
                    log::warn!("Failed to load disk at {:?}: {}", path, e);
                }
            }
        }
        Ok(disks)
    }
    
    /// Save pool metadata
    pub fn save(&self, pool_dir: &Path) -> Result<()> {
        let pool_path = pool_dir.join("pool.json");
        let contents = serde_json::to_string_pretty(self)?;
        
        let temp_path = pool_path.with_extension("json.tmp");
        fs::write(&temp_path, contents)?;
        fs::rename(&temp_path, &pool_path)?;
        
        Ok(())
    }
    
    /// Load pool metadata
    pub fn load(pool_dir: &Path) -> Result<Self> {
        let pool_path = pool_dir.join("pool.json");
        if !pool_path.exists() {
            return Ok(DiskPool::new());
        }
        
        let contents = fs::read_to_string(&pool_path)?;
        let pool: DiskPool = serde_json::from_str(&contents)?;
        Ok(pool)
    }
}

// We need to add nix as a dependency for statvfs
