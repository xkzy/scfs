use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::disk::Disk;
use crate::metrics::Metrics;

/// Represents a range of blocks to be trimmed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimRange {
    pub disk_uuid: Uuid,
    pub path: PathBuf,
    pub offset: u64,
    pub length: u64,
    pub timestamp: i64,
}

/// TRIM operation batch for efficient processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimBatch {
    pub disk_uuid: Uuid,
    pub ranges: Vec<TrimRange>,
    pub total_bytes: u64,
    pub created_at: i64,
}

/// TRIM intensity levels
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrimIntensity {
    /// Conservative: Only TRIM on explicit request
    Conservative,
    /// Balanced: TRIM when threshold reached (1GB or daily)
    Balanced,
    /// Aggressive: TRIM immediately after deletions
    Aggressive,
}

impl TrimIntensity {
    pub fn batch_threshold_bytes(&self) -> u64 {
        match self {
            TrimIntensity::Conservative => 10 * 1024 * 1024 * 1024, // 10GB
            TrimIntensity::Balanced => 1024 * 1024 * 1024,          // 1GB
            TrimIntensity::Aggressive => 10 * 1024 * 1024,          // 10MB
        }
    }

    pub fn batch_delay_secs(&self) -> u64 {
        match self {
            TrimIntensity::Conservative => 7 * 24 * 3600, // Weekly
            TrimIntensity::Balanced => 24 * 3600,         // Daily
            TrimIntensity::Aggressive => 3600,            // Hourly
        }
    }
}

/// TRIM configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimConfig {
    pub enabled: bool,
    pub intensity: TrimIntensity,
    pub batch_size_mb: u64,
    pub secure_erase: bool,
    pub discard_granularity: u64,
}

impl Default for TrimConfig {
    fn default() -> Self {
        TrimConfig {
            enabled: true,
            intensity: TrimIntensity::Balanced,
            batch_size_mb: 100,
            secure_erase: false,
            discard_granularity: 4096, // 4KB default
        }
    }
}

/// TRIM operation statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimStats {
    pub total_trim_operations: u64,
    pub total_bytes_trimmed: u64,
    pub total_ranges_trimmed: u64,
    pub failed_operations: u64,
    pub last_trim_at: Option<i64>,
    pub pending_bytes: u64,
    pub pending_ranges: u64,
}

/// SSD health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsdHealth {
    pub disk_uuid: Uuid,
    pub total_bytes_written: u64,
    pub trim_supported: bool,
    pub wear_level_percent: Option<u8>,
    pub estimated_lifetime_remaining_percent: Option<u8>,
}

/// Main TRIM engine for managing TRIM/DISCARD operations
pub struct TrimEngine {
    config: Arc<Mutex<TrimConfig>>,
    running: Arc<AtomicBool>,
    
    // Pending TRIM operations per disk
    pending_trims: Arc<Mutex<HashMap<Uuid, VecDeque<TrimRange>>>>,
    
    // Statistics
    total_operations: Arc<AtomicU64>,
    total_bytes_trimmed: Arc<AtomicU64>,
    total_ranges_trimmed: Arc<AtomicU64>,
    failed_operations: Arc<AtomicU64>,
    last_trim_at: Arc<Mutex<Option<i64>>>,
}

impl TrimEngine {
    pub fn new(config: TrimConfig) -> Self {
        TrimEngine {
            config: Arc::new(Mutex::new(config)),
            running: Arc::new(AtomicBool::new(false)),
            pending_trims: Arc::new(Mutex::new(HashMap::new())),
            total_operations: Arc::new(AtomicU64::new(0)),
            total_bytes_trimmed: Arc::new(AtomicU64::new(0)),
            total_ranges_trimmed: Arc::new(AtomicU64::new(0)),
            failed_operations: Arc::new(AtomicU64::new(0)),
            last_trim_at: Arc::new(Mutex::new(None)),
        }
    }

    /// Queue a deleted fragment for TRIM
    pub fn queue_trim(&self, disk_uuid: Uuid, path: PathBuf, size: u64) -> Result<()> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let trim_range = TrimRange {
            disk_uuid,
            path,
            offset: 0,
            length: size,
            timestamp,
        };

        let mut pending = self.pending_trims.lock().unwrap();
        pending
            .entry(disk_uuid)
            .or_insert_with(VecDeque::new)
            .push_back(trim_range);

        Ok(())
    }

    /// Execute pending TRIM operations for a disk
    pub fn execute_trim(&self, disk_uuid: Uuid, metrics: &Metrics) -> Result<u64> {
        let config = self.config.lock().unwrap().clone();
        if !config.enabled {
            return Ok(0);
        }

        let mut pending = self.pending_trims.lock().unwrap();
        let ranges = match pending.get_mut(&disk_uuid) {
            Some(queue) => queue,
            None => return Ok(0),
        };

        if ranges.is_empty() {
            return Ok(0);
        }

        // Build batch up to threshold
        let mut batch = Vec::new();
        let mut total_bytes = 0u64;
        let threshold_bytes = config.batch_size_mb * 1024 * 1024;

        while total_bytes < threshold_bytes {
            match ranges.pop_front() {
                Some(range) => {
                    total_bytes += range.length;
                    batch.push(range);
                }
                None => break,
            }
        }

        drop(pending); // Release lock before I/O

        if batch.is_empty() {
            return Ok(0);
        }

        // Execute TRIM operations
        let mut bytes_trimmed = 0u64;
        for range in &batch {
            match Self::trim_range(&range, &config) {
                Ok(bytes) => {
                    bytes_trimmed += bytes;
                    self.total_ranges_trimmed.fetch_add(1, Ordering::SeqCst);
                }
                Err(e) => {
                    eprintln!("Failed to TRIM {}: {}", range.path.display(), e);
                    self.failed_operations.fetch_add(1, Ordering::SeqCst);
                }
            }
        }

        // Update statistics
        self.total_operations.fetch_add(1, Ordering::SeqCst);
        self.total_bytes_trimmed
            .fetch_add(bytes_trimmed, Ordering::SeqCst);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        *self.last_trim_at.lock().unwrap() = Some(timestamp);

        // Update metrics
        metrics.trim_operations.fetch_add(1, Ordering::SeqCst);
        metrics
            .trim_bytes_reclaimed
            .fetch_add(bytes_trimmed, Ordering::SeqCst);

        Ok(bytes_trimmed)
    }

    /// Execute TRIM on all disks
    pub fn execute_all_trims(&self, disks: &[Disk], metrics: &Metrics) -> Result<u64> {
        let mut total_bytes = 0u64;
        
        for disk in disks {
            match self.execute_trim(disk.uuid, metrics) {
                Ok(bytes) => total_bytes += bytes,
                Err(e) => eprintln!("TRIM failed for disk {}: {}", disk.uuid, e),
            }
        }

        Ok(total_bytes)
    }

    /// Start background TRIM daemon
    pub fn start(&self, disks: Vec<Disk>, metrics: Arc<Metrics>) -> Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Ok(()); // Already running
        }

        self.running.store(true, Ordering::SeqCst);

        let running = Arc::clone(&self.running);
        let config = Arc::clone(&self.config);
        let pending_trims = Arc::clone(&self.pending_trims);
        let total_operations = Arc::clone(&self.total_operations);
        let total_bytes_trimmed = Arc::clone(&self.total_bytes_trimmed);
        let total_ranges_trimmed = Arc::clone(&self.total_ranges_trimmed);
        let failed_operations = Arc::clone(&self.failed_operations);
        let last_trim_at = Arc::clone(&self.last_trim_at);

        std::thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                let cfg = config.lock().unwrap().clone();
                if !cfg.enabled {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    continue;
                }

                // Check if we should run TRIM based on intensity
                let should_trim = Self::should_run_trim(&cfg, &pending_trims);
                
                if should_trim {
                    for disk in &disks {
                        // Execute TRIM using the existing engine instance references
                        if let Err(e) = Self::execute_trim_for_disk(
                            &disk.uuid,
                            &pending_trims,
                            &config,
                            &metrics,
                            &total_operations,
                            &total_bytes_trimmed,
                            &total_ranges_trimmed,
                            &failed_operations,
                            &last_trim_at,
                        ) {
                            eprintln!("TRIM error for disk {}: {}", disk.uuid, e);
                        }
                    }
                }

                // Sleep based on intensity
                let delay_secs = cfg.intensity.batch_delay_secs();
                std::thread::sleep(std::time::Duration::from_secs(delay_secs));
            }
        });

        Ok(())
    }

    /// Stop background TRIM daemon
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Get TRIM statistics
    pub fn stats(&self) -> TrimStats {
        let pending = self.pending_trims.lock().unwrap();
        let pending_bytes: u64 = pending
            .values()
            .flat_map(|queue| queue.iter())
            .map(|range| range.length)
            .sum();
        let pending_ranges: u64 = pending
            .values()
            .map(|queue| queue.len() as u64)
            .sum();

        TrimStats {
            total_trim_operations: self.total_operations.load(Ordering::SeqCst),
            total_bytes_trimmed: self.total_bytes_trimmed.load(Ordering::SeqCst),
            total_ranges_trimmed: self.total_ranges_trimmed.load(Ordering::SeqCst),
            failed_operations: self.failed_operations.load(Ordering::SeqCst),
            last_trim_at: *self.last_trim_at.lock().unwrap(),
            pending_bytes,
            pending_ranges,
        }
    }

    /// Get SSD health information
    pub fn get_ssd_health(&self, disk: &Disk) -> Result<SsdHealth> {
        // In a real implementation, this would query SMART data
        // For now, return basic information
        Ok(SsdHealth {
            disk_uuid: disk.uuid,
            total_bytes_written: disk.used_bytes,
            trim_supported: Self::check_trim_support(&disk.path)?,
            wear_level_percent: None,
            estimated_lifetime_remaining_percent: None,
        })
    }

    /// Check if TRIM is supported on the disk
    fn check_trim_support(path: &Path) -> Result<bool> {
        // On Linux, check /sys/block/*/queue/discard_granularity
        // For directory-backed testing, always return true
        if path.is_dir() {
            return Ok(true);
        }

        // TODO: Implement actual TRIM support detection for block devices
        Ok(true)
    }

    /// Perform actual TRIM operation on a range
    fn trim_range(range: &TrimRange, config: &TrimConfig) -> Result<u64> {
        // TODO: For directory-backed storage, we can't use actual TRIM/DISCARD
        // Instead, we can use fallocate with FALLOC_FL_PUNCH_HOLE to release space
        // For production block devices, implement proper TRIM/DISCARD via ioctl
        
        if !range.path.exists() {
            // File already deleted, consider it trimmed
            return Ok(range.length);
        }

        // For secure erase, overwrite with zeros before releasing
        if config.secure_erase {
            Self::secure_erase_file(&range.path)?;
        }

        // On Linux with supported filesystems, we could use:
        // fallocate(fd, FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE, offset, len)
        // For now, just remove the file which releases the space
        if range.path.is_file() {
            std::fs::remove_file(&range.path)
                .with_context(|| format!("Failed to remove file for TRIM: {:?}", range.path))?;
        }

        Ok(range.length)
    }

    /// Securely erase a file by overwriting with zeros
    fn secure_erase_file(path: &Path) -> Result<()> {
        use std::io::Write;

        if !path.exists() {
            return Ok(());
        }

        let metadata = std::fs::metadata(path)?;
        let size = metadata.len();

        let mut file = File::options().write(true).open(path)?;
        
        // Overwrite with zeros in alignment-aware way
        let chunk_size = 1024 * 1024; // 1MB nominal chunk
        let mut remaining = size;

        // Try using aligned direct writes where possible (falls back internally)
        while remaining > 0 {
            let to_write = remaining.min(chunk_size as u64) as usize;
            let zeros = vec![0u8; to_write];
            match crate::io_alignment::write_aligned_file(path, &zeros, true) {
                Ok(_) => {
                    // written via aligned direct (or fallback)
                }
                Err(e) => {
                    // If direct aligned fails for some reason, do buffered writes
                    log::warn!("aligned secure erase failed for {:?}: {}. falling back to buffered.", path, e);
                    file.write_all(&zeros)?;
                }
            }
            remaining -= to_write as u64;
        }

        file.sync_all()?;
        Ok(())
    }

    /// Check if we should run TRIM based on configuration
    fn should_run_trim(
        config: &TrimConfig,
        pending_trims: &Arc<Mutex<HashMap<Uuid, VecDeque<TrimRange>>>>,
    ) -> bool {
        let pending = pending_trims.lock().unwrap();
        let total_pending_bytes: u64 = pending
            .values()
            .flat_map(|queue| queue.iter())
            .map(|range| range.length)
            .sum();

        total_pending_bytes >= config.intensity.batch_threshold_bytes()
    }
    
    /// Execute TRIM for a single disk (helper for background thread)
    fn execute_trim_for_disk(
        disk_uuid: &Uuid,
        pending_trims: &Arc<Mutex<HashMap<Uuid, VecDeque<TrimRange>>>>,
        config: &Mutex<TrimConfig>,
        metrics: &Metrics,
        total_operations: &AtomicU64,
        total_bytes_trimmed: &AtomicU64,
        total_ranges_trimmed: &AtomicU64,
        failed_operations: &AtomicU64,
        last_trim_at: &Mutex<Option<i64>>,
    ) -> Result<()> {
        let cfg = config.lock().unwrap().clone();
        if !cfg.enabled {
            return Ok(());
        }

        let mut pending = pending_trims.lock().unwrap();
        let ranges = match pending.get_mut(disk_uuid) {
            Some(queue) => queue,
            None => return Ok(()),
        };

        if ranges.is_empty() {
            return Ok(());
        }

        // Build batch up to threshold
        let mut batch = Vec::new();
        let mut total_bytes = 0u64;
        let threshold_bytes = cfg.batch_size_mb * 1024 * 1024;

        while total_bytes < threshold_bytes {
            match ranges.pop_front() {
                Some(range) => {
                    total_bytes += range.length;
                    batch.push(range);
                }
                None => break,
            }
        }

        drop(pending); // Release lock before I/O

        if batch.is_empty() {
            return Ok(());
        }

        // Execute TRIM operations
        let mut bytes_trimmed = 0u64;
        for range in &batch {
            match Self::trim_range(&range, &cfg) {
                Ok(bytes) => {
                    bytes_trimmed += bytes;
                    total_ranges_trimmed.fetch_add(1, Ordering::SeqCst);
                }
                Err(e) => {
                    eprintln!("Failed to TRIM {}: {}", range.path.display(), e);
                    failed_operations.fetch_add(1, Ordering::SeqCst);
                }
            }
        }

        // Update statistics
        total_operations.fetch_add(1, Ordering::SeqCst);
        total_bytes_trimmed.fetch_add(bytes_trimmed, Ordering::SeqCst);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        *last_trim_at.lock().unwrap() = Some(timestamp);

        // Update metrics
        metrics.trim_operations.fetch_add(1, Ordering::SeqCst);
        metrics
            .trim_bytes_reclaimed
            .fetch_add(bytes_trimmed, Ordering::SeqCst);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim_intensity() {
        assert_eq!(
            TrimIntensity::Conservative.batch_threshold_bytes(),
            10 * 1024 * 1024 * 1024
        );
        assert_eq!(
            TrimIntensity::Balanced.batch_threshold_bytes(),
            1024 * 1024 * 1024
        );
        assert_eq!(
            TrimIntensity::Aggressive.batch_threshold_bytes(),
            10 * 1024 * 1024
        );
    }

    #[test]
    fn test_trim_config_default() {
        let config = TrimConfig::default();
        assert!(config.enabled);
        assert_eq!(config.intensity, TrimIntensity::Balanced);
        assert_eq!(config.discard_granularity, 4096);
    }

    #[test]
    fn test_trim_stats() {
        let engine = TrimEngine::new(TrimConfig::default());
        let stats = engine.stats();
        
        assert_eq!(stats.total_trim_operations, 0);
        assert_eq!(stats.total_bytes_trimmed, 0);
        assert_eq!(stats.pending_bytes, 0);
    }
}
