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
    /// Kind of backing (directory or block device)
    #[serde(default)]
    pub kind: DiskKind,

    /// In-memory allocator and index (not serialized)
    #[serde(skip)]
    pub allocator: Option<crate::allocator::BitmapAllocator>,
    #[serde(skip)]
    pub free_index: Option<crate::free_extent::FreeExtentIndex>,
    #[serde(skip)]
    /// On-device allocator (for block devices)
    pub on_device_allocator: Option<crate::on_device_allocator::OnDeviceAllocator>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiskKind {
    Directory,
    BlockDevice,
}

impl Default for DiskKind {
    fn default() -> Self {
        DiskKind::Directory
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiskHealth {
    /// Fully operational (read/write)
    Healthy,
    /// Partial failures; read-only (never selected for new writes)
    Degraded,
    /// Intermittent errors; read-only (never selected for new writes)
    Suspect,
    /// Being removed gracefully; read-only (never selected for new writes)
    Draining,
    /// Completely offline/unavailable
    Failed,
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
        
        // Create fragments directory (only for directory-backed disks)
        let fragments_dir = path.join("fragments");
        fs::create_dir_all(&fragments_dir)
            .context("Failed to create fragments directory")?;
        
        let mut disk = Disk {
            uuid,
            path: path.clone(),
            capacity_bytes,
            used_bytes: 0,
            health: DiskHealth::Healthy,
            kind: DiskKind::Directory,
            allocator: None,
            free_index: None,
            on_device_allocator: None,
        };

        // Initialize allocator and free-index for directory-backed disk
        disk.init_allocator_and_index()?;
        disk.save()?;
        Ok(disk)
    }

    /// Initialize a disk backed by a raw block device
    pub fn from_block_device(path: PathBuf) -> Result<Self> {
        // Do not create directories on raw devices
        let uuid = Uuid::new_v4();
        let capacity_bytes = Self::get_block_device_size(&path)?;

        let mut disk = Disk {
            uuid,
            path: path.clone(),
            capacity_bytes,
            used_bytes: 0,
            health: DiskHealth::Healthy,
            kind: DiskKind::BlockDevice,
            allocator: None,
            free_index: None,
            on_device_allocator: None,
        };

        // Try loading on-device allocator if present (non-fatal)
        match crate::on_device_allocator::OnDeviceAllocator::load_from_device(&path) {
            Ok(oda) => {
                disk.on_device_allocator = Some(oda);
            }
            Err(_) => {
                // Device may be unformatted for on-device allocator; defer until explicitly formatted
            }
        }

        disk.save()?;
        Ok(disk)
    }
    
    /// Load disk from its directory
    pub fn load(path: &Path) -> Result<Self> {
        let metadata_path = path.join("disk.json");
        let contents = fs::read_to_string(&metadata_path)
            .context("Failed to read disk metadata")?;
        let mut disk: Disk = serde_json::from_str(&contents)
            .context("Failed to parse disk metadata")?;

        // Initialize runtime-only fields
        disk.allocator = None;
        disk.free_index = None;

        // If directory-backed, attempt to load allocator and free-index
        if disk.kind == DiskKind::Directory {
            let _ = disk.init_allocator_and_index();
        }

        // If block-device, attempt to attach on-device allocator (non-fatal)
        if disk.kind == DiskKind::BlockDevice {
            if let Ok(oda) = crate::on_device_allocator::OnDeviceAllocator::load_from_device(&disk.path) {
                disk.on_device_allocator = Some(oda);
            }
        }

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

    fn get_block_device_size(path: &Path) -> Result<u64> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;
        use std::fs::OpenOptions;
        use libc;

        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_RDONLY)
            .open(path)
            .context("Failed to open block device")?;
        let fd = file.as_raw_fd();

        let mut size: u64 = 0;
        const BLKGETSIZE64: libc::c_ulong = 0x80081272;
        let ret = unsafe { libc::ioctl(fd, BLKGETSIZE64 as _, &mut size) };
        if ret != 0 {
            return Err(anyhow!("BLKGETSIZE64 ioctl failed"));
        }
        Ok(size)
    }

    /// Initialize allocator and free-extent index for directory-backed disk
    fn init_allocator_and_index(&mut self) -> Result<()> {
        // Use default allocation unit = 1 MiB (matching extent size)
        const UNIT_SIZE: u64 = 1024 * 1024;
        let total_units = self.capacity_bytes / UNIT_SIZE;
        let alloc_path = self.path.join("allocator.bin");
        let index_path = self.path.join("free_extent.bin");

        // If allocator file exists, load it; otherwise create new allocator and persist
        let allocator = if alloc_path.exists() {
            crate::allocator::BitmapAllocator::load_from_path(&alloc_path)?
        } else {
            crate::allocator::BitmapAllocator::new(UNIT_SIZE, total_units, Some(alloc_path.clone()))?
        };

        // If free extent index exists, load it; otherwise build from allocator free bits
        let mut index = if index_path.exists() {
            crate::free_extent::FreeExtentIndex::new(Some(index_path.clone()))?
        } else {
            let mut idx = crate::free_extent::FreeExtentIndex::new(Some(index_path.clone()))?;
            // scan allocator for free runs
            let mut run_start: Option<u64> = None;
            let mut run_len: u64 = 0;
            for unit in 0..total_units {
                if allocator.is_free(unit) {
                    if run_start.is_none() {
                        run_start = Some(unit);
                        run_len = 1;
                    } else {
                        run_len += 1;
                    }
                } else if let Some(rs) = run_start {
                    idx.insert_run(rs, run_len)?;
                    run_start = None;
                    run_len = 0;
                }
            }
            if let Some(rs) = run_start {
                if run_len > 0 {
                    idx.insert_run(rs, run_len)?;
                }
            }
            idx
        };

        self.allocator = Some(allocator);
        self.free_index = Some(index);
        Ok(())
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
        // Handle block device backed disks using on-device allocator when available
        if self.kind == DiskKind::BlockDevice {
            if let Some(oda) = &mut self.on_device_allocator {
                // Prepare fragment header
                let ch = blake3::hash(&data);
                let hdr = crate::on_device_allocator::FragmentHeader {
                    extent_uuid: *extent_uuid,
                    fragment_index: fragment_index as u32,
                    total_length: data.len() as u64,
                    data_checksum: *ch.as_bytes(),
                };

                // units required
                let n_units = ((hdr.total_length + (16+4+8+32+4) as u64) + oda.unit_size - 1) / oda.unit_size;
                let start = oda.allocate_contiguous(n_units).ok_or_else(|| anyhow!("Failed to allocate on-device space"))?;

                #[cfg(test)]
                check_crash_point(CrashPoint::BeforeFragmentWrite)?;

                oda.write_fragment_at(start, data, &hdr)?;

                #[cfg(test)]
                check_crash_point(CrashPoint::AfterFragmentWrite)?;

                // verify readback
                let (_rh, rd) = oda.read_fragment_at(start)?;
                if rd != data {
                    anyhow::bail!("Fragment verification failed on device");
                }

                self.used_bytes += data.len() as u64;
                self.save()?;
                return Ok(());
            } else {
                return Err(anyhow!("Block device missing on-device allocator (not formatted)"));
            }
        }

        // Regular directory-backed behavior
        let fragment_path = self.fragment_path(extent_uuid, fragment_index);
        
        #[cfg(test)]
        check_crash_point(CrashPoint::BeforeFragmentWrite)?;
        
         let temp_path = fragment_path.with_extension("frag.tmp");
         let mut guard = TempFragmentGuard::new(temp_path.clone());
         {
             // Use alignment-aware write (prefer direct when available)
             if let Err(e) = crate::io_alignment::write_aligned_file(&temp_path, data, true) {
                 // Fallback to buffered write if aligned/direct write fails
                 let mut file = File::create(&temp_path)
                     .context("Failed to open temp fragment file")?;
                 file.write_all(data)
                     .context("Failed to write fragment")?;
                 file.sync_all()
                     .context("Failed to fsync fragment data")?;
                 log::warn!("Aligned write failed for {}: {}. used buffered write.", temp_path.display(), e);
             }
         }
         
         #[cfg(test)]
         check_crash_point(CrashPoint::AfterFragmentWrite)?;
         
         fs::rename(&temp_path, &fragment_path)
             .context("Failed to commit fragment")?;
         guard.commit();

         // Verify readback
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
