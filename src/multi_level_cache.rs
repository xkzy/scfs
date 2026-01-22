//! Multi-Level Caching System - Phase 14
//!
//! This module implements a coherent multi-level caching system with:
//! - **L1 Cache**: In-memory LRU cache (from Phase 10.3)
//! - **L2 Cache**: Local NVMe-backed persistent cache
//! - **L3 Cache**: Optional remote/proxy cache (interface)
//!
//! ## Phase 14: Multi-Level Caching Optimization
//!
//! **Goal**: 5-20x read latency reduction for hot data with multi-tier caching
//!
//! ### Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │         Read Request                    │
//! └────────────────┬────────────────────────┘
//!                  │
//!                  v
//!         ┌────────────────┐
//!         │ L1: Memory     │ <1ms
//!         │ (DataCache)    │
//!         └────┬───────────┘
//!              │
//!     ┌────────┴────────┐
//!     │ Hit         Miss│
//!     v                 v
//! ┌────────┐   ┌────────────────┐
//! │Return  │   │ L2: NVMe Cache │ 1-5ms
//! │Data    │   │ (File-backed)  │
//! └────────┘   └────┬───────────┘
//!                   │
//!          ┌────────┴────────┐
//!          │ Hit         Miss│
//!          v                 v
//!      ┌────────┐   ┌────────────────┐
//!      │Promote │   │ L3/Backend     │ 10-100ms
//!      │to L1   │   │ (Disk I/O)     │
//!      └────────┘   └────────────────┘
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::fs;
use std::io::{Read, Write};
use uuid::Uuid;
use anyhow::{Result, Context};
use crate::data_cache::DataCache;

/// Multi-level cache manager
///
/// Coordinates L1 (memory), L2 (NVMe), and optional L3 (remote) caches.
pub struct MultiLevelCache {
    /// L1: In-memory cache (fast, small)
    l1_cache: Arc<DataCache>,
    
    /// L2: NVMe-backed cache (medium speed, larger)
    l2_cache: Arc<Mutex<L2Cache>>,
    
    /// L3: Remote cache interface (optional)
    l3_cache: Option<Arc<Mutex<dyn L3CacheInterface + Send>>>,
    
    /// Cache statistics
    stats: Arc<Mutex<MultiLevelCacheStats>>,
    
    /// Adaptive policy engine
    policy: Arc<Mutex<CachePolicy>>,
}

/// L2 Cache: NVMe-backed persistent cache
pub struct L2Cache {
    /// Cache directory path
    cache_dir: PathBuf,
    
    /// Index: extent UUID -> cache file info
    index: HashMap<Uuid, L2CacheEntry>,
    
    /// Maximum cache size in bytes
    max_size_bytes: usize,
    
    /// Current size in bytes
    current_size: usize,
    
    /// Statistics
    hits: u64,
    misses: u64,
    evictions: u64,
}

/// L2 cache entry metadata
#[derive(Debug, Clone)]
struct L2CacheEntry {
    /// File name in cache directory
    filename: String,
    
    /// Size in bytes
    size: usize,
    
    /// Last access timestamp
    last_access: u64,
    
    /// Whether this is a hot extent
    is_hot: bool,
}

/// L3 Cache interface for remote/distributed caching
pub trait L3CacheInterface {
    /// Get data from remote cache
    fn get(&self, extent_uuid: &Uuid) -> Result<Option<Vec<u8>>>;
    
    /// Put data into remote cache
    fn put(&self, extent_uuid: Uuid, data: Vec<u8>) -> Result<()>;
    
    /// Invalidate entry in remote cache
    fn invalidate(&self, extent_uuid: &Uuid) -> Result<()>;
}

/// Multi-level cache statistics
#[derive(Debug, Clone, Default)]
pub struct MultiLevelCacheStats {
    pub l1_hits: u64,
    pub l1_misses: u64,
    pub l2_hits: u64,
    pub l2_misses: u64,
    pub l3_hits: u64,
    pub l3_misses: u64,
    pub promotions_to_l1: u64,
    pub evictions_from_l1: u64,
    pub evictions_from_l2: u64,
    pub backend_reads: u64,
}

impl MultiLevelCacheStats {
    /// Calculate overall cache hit rate
    pub fn overall_hit_rate(&self) -> f64 {
        let total_requests = self.l1_hits + self.l1_misses;
        if total_requests == 0 {
            0.0
        } else {
            self.l1_hits as f64 / total_requests as f64
        }
    }
    
    /// Calculate L2 hit rate (when L1 misses)
    pub fn l2_hit_rate(&self) -> f64 {
        let l2_requests = self.l2_hits + self.l2_misses;
        if l2_requests == 0 {
            0.0
        } else {
            self.l2_hits as f64 / l2_requests as f64
        }
    }
    
    /// Calculate backend I/O reduction
    pub fn backend_io_reduction(&self) -> f64 {
        let total_requests = self.l1_hits + self.l1_misses;
        let cache_hits = self.l1_hits + self.l2_hits + self.l3_hits;
        if total_requests == 0 {
            0.0
        } else {
            cache_hits as f64 / total_requests as f64
        }
    }
}

/// Cache policy for adaptive behavior
#[derive(Debug, Clone)]
pub struct CachePolicy {
    /// L1 admission policy: always, hot_only, sampled
    pub l1_admission: AdmissionPolicy,
    
    /// L2 admission policy
    pub l2_admission: AdmissionPolicy,
    
    /// Promotion threshold (accesses before L1 promotion)
    pub l1_promotion_threshold: u32,
    
    /// Whether to enable write-back for L2
    pub l2_write_back: bool,
}

impl Default for CachePolicy {
    fn default() -> Self {
        CachePolicy {
            l1_admission: AdmissionPolicy::HotOnly,
            l2_admission: AdmissionPolicy::Always,
            l1_promotion_threshold: 2,
            l2_write_back: false,
        }
    }
}

/// Admission policy for cache levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionPolicy {
    /// Always admit to cache
    Always,
    
    /// Only admit hot-classified extents
    HotOnly,
    
    /// Sample-based admission (admit X% of requests)
    Sampled(u8), // 0-100 percentage
}

impl L2Cache {
    /// Create a new L2 cache
    ///
    /// # Arguments
    ///
    /// * `cache_dir` - Directory to store cache files
    /// * `max_size_bytes` - Maximum cache size in bytes
    pub fn new(cache_dir: PathBuf, max_size_bytes: usize) -> Result<Self> {
        // Create cache directory if it doesn't exist
        fs::create_dir_all(&cache_dir)
            .context("Failed to create L2 cache directory")?;
        
        // Load or create index
        let index = Self::load_index(&cache_dir)?;
        
        // Calculate current size from index
        let current_size = index.values().map(|e| e.size).sum();
        
        Ok(L2Cache {
            cache_dir,
            index,
            max_size_bytes,
            current_size,
            hits: 0,
            misses: 0,
            evictions: 0,
        })
    }
    
    /// Load index from disk (or create empty)
    fn load_index(cache_dir: &Path) -> Result<HashMap<Uuid, L2CacheEntry>> {
        let index_path = cache_dir.join("index.json");
        
        if index_path.exists() {
            let content = fs::read_to_string(&index_path)
                .context("Failed to read L2 cache index")?;
            let index: HashMap<Uuid, L2CacheEntry> = serde_json::from_str(&content)
                .context("Failed to parse L2 cache index")?;
            Ok(index)
        } else {
            Ok(HashMap::new())
        }
    }
    
    /// Save index to disk
    fn save_index(&self) -> Result<()> {
        let index_path = self.cache_dir.join("index.json");
        let content = serde_json::to_string(&self.index)
            .context("Failed to serialize L2 cache index")?;
        fs::write(&index_path, content)
            .context("Failed to write L2 cache index")?;
        Ok(())
    }
    
    /// Get data from L2 cache
    pub fn get(&mut self, extent_uuid: &Uuid) -> Result<Option<Vec<u8>>> {
        if let Some(entry) = self.index.get_mut(extent_uuid) {
            // Update access time
            entry.last_access = current_timestamp();
            
            // Read from file
            let file_path = self.cache_dir.join(&entry.filename);
            let data = fs::read(&file_path)
                .context("Failed to read L2 cache file")?;
            
            self.hits += 1;
            log::trace!("L2 cache hit for extent {}", extent_uuid);
            Ok(Some(data))
        } else {
            self.misses += 1;
            log::trace!("L2 cache miss for extent {}", extent_uuid);
            Ok(None)
        }
    }
    
    /// Put data into L2 cache
    pub fn put(&mut self, extent_uuid: Uuid, data: Vec<u8>, is_hot: bool) -> Result<()> {
        let data_size = data.len();
        
        // Check if we need to evict
        self.maybe_evict(data_size)?;
        
        // Generate filename
        let filename = format!("{}.cache", extent_uuid);
        let file_path = self.cache_dir.join(&filename);
        
        // Write to file
        fs::write(&file_path, &data)
            .context("Failed to write L2 cache file")?;
        
        // Update index
        let entry = L2CacheEntry {
            filename,
            size: data_size,
            last_access: current_timestamp(),
            is_hot,
        };
        
        // Remove old entry if exists
        if let Some(old_entry) = self.index.remove(&extent_uuid) {
            self.current_size = self.current_size.saturating_sub(old_entry.size);
            let old_path = self.cache_dir.join(&old_entry.filename);
            let _ = fs::remove_file(old_path); // Ignore errors
        }
        
        self.index.insert(extent_uuid, entry);
        self.current_size += data_size;
        
        // Save index periodically (every 100 insertions for performance)
        if self.index.len() % 100 == 0 {
            self.save_index()?;
        }
        
        log::trace!("L2 cached extent {} ({} bytes)", extent_uuid, data_size);
        Ok(())
    }
    
    /// Invalidate entry in L2 cache
    pub fn invalidate(&mut self, extent_uuid: &Uuid) -> Result<()> {
        if let Some(entry) = self.index.remove(extent_uuid) {
            self.current_size = self.current_size.saturating_sub(entry.size);
            
            let file_path = self.cache_dir.join(&entry.filename);
            fs::remove_file(&file_path)
                .context("Failed to remove L2 cache file")?;
            
            log::debug!("Invalidated L2 cache entry for extent {}", extent_uuid);
        }
        Ok(())
    }
    
    /// Evict entries to make space
    fn maybe_evict(&mut self, needed_bytes: usize) -> Result<()> {
        if self.current_size + needed_bytes <= self.max_size_bytes {
            return Ok(()); // No eviction needed
        }
        
        let to_free = (self.current_size + needed_bytes) - self.max_size_bytes;
        let mut freed = 0;
        
        // Build eviction candidates (prefer cold, then LRU)
        let mut candidates: Vec<(Uuid, u64, bool, usize)> = self.index
            .iter()
            .map(|(uuid, entry)| (*uuid, entry.last_access, entry.is_hot, entry.size))
            .collect();
        
        // Sort: cold first, then by LRU
        candidates.sort_by(|a, b| {
            match (a.2, b.2) {
                (false, true) => std::cmp::Ordering::Less,
                (true, false) => std::cmp::Ordering::Greater,
                _ => a.1.cmp(&b.1),
            }
        });
        
        // Evict entries
        for (uuid, _, _, size) in candidates {
            if freed >= to_free {
                break;
            }
            
            self.invalidate(&uuid)?;
            freed += size;
            self.evictions += 1;
        }
        
        Ok(())
    }
    
    /// Get statistics
    pub fn stats(&self) -> (u64, u64, u64, usize, usize) {
        (self.hits, self.misses, self.evictions, self.index.len(), self.current_size)
    }
    
    /// Flush all entries
    pub fn flush(&mut self) -> Result<()> {
        for entry in self.index.values() {
            let file_path = self.cache_dir.join(&entry.filename);
            let _ = fs::remove_file(file_path);
        }
        self.index.clear();
        self.current_size = 0;
        self.save_index()?;
        log::info!("L2 cache flushed");
        Ok(())
    }
}

impl MultiLevelCache {
    /// Create a new multi-level cache
    ///
    /// # Arguments
    ///
    /// * `l1_size_bytes` - L1 (memory) cache size
    /// * `l2_cache_dir` - L2 (NVMe) cache directory
    /// * `l2_size_bytes` - L2 cache size
    pub fn new(
        l1_size_bytes: usize,
        l2_cache_dir: PathBuf,
        l2_size_bytes: usize,
    ) -> Result<Self> {
        let l1_cache = Arc::new(DataCache::new(l1_size_bytes));
        let l2_cache = Arc::new(Mutex::new(L2Cache::new(l2_cache_dir, l2_size_bytes)?));
        
        Ok(MultiLevelCache {
            l1_cache,
            l2_cache,
            l3_cache: None,
            stats: Arc::new(Mutex::new(MultiLevelCacheStats::default())),
            policy: Arc::new(Mutex::new(CachePolicy::default())),
        })
    }
    
    /// Get data with multi-level lookup
    ///
    /// Checks L1 -> L2 -> L3 -> Backend, promoting data up the hierarchy.
    pub fn get(&self, extent_uuid: &Uuid, is_hot: bool) -> Option<Vec<u8>> {
        let mut stats = self.stats.lock().unwrap();
        
        // Try L1
        if let Some(data) = self.l1_cache.get(extent_uuid) {
            stats.l1_hits += 1;
            drop(stats);
            log::trace!("Multi-level cache: L1 hit for {}", extent_uuid);
            return Some(data);
        }
        stats.l1_misses += 1;
        drop(stats);
        
        // Try L2
        let mut l2 = self.l2_cache.lock().unwrap();
        if let Ok(Some(data)) = l2.get(extent_uuid) {
            let mut stats = self.stats.lock().unwrap();
            stats.l2_hits += 1;
            drop(stats);
            drop(l2);
            
            // Promote to L1 if meets policy
            let policy = self.policy.lock().unwrap();
            let should_promote = match policy.l1_admission {
                AdmissionPolicy::Always => true,
                AdmissionPolicy::HotOnly => is_hot,
                AdmissionPolicy::Sampled(pct) => {
                    (extent_uuid.as_bytes()[0] as u32 * 100 / 255) < pct as u32
                }
            };
            drop(policy);
            
            if should_promote {
                self.l1_cache.put(*extent_uuid, data.clone(), is_hot);
                let mut stats = self.stats.lock().unwrap();
                stats.promotions_to_l1 += 1;
            }
            
            log::trace!("Multi-level cache: L2 hit for {}", extent_uuid);
            return Some(data);
        }
        drop(l2);
        
        let mut stats = self.stats.lock().unwrap();
        stats.l2_misses += 1;
        stats.backend_reads += 1;
        drop(stats);
        
        log::trace!("Multi-level cache: Miss for {}", extent_uuid);
        None
    }
    
    /// Put data into appropriate cache levels
    pub fn put(&self, extent_uuid: Uuid, data: Vec<u8>, is_hot: bool) -> Result<()> {
        let policy = self.policy.lock().unwrap();
        
        // L1 admission
        let admit_l1 = match policy.l1_admission {
            AdmissionPolicy::Always => true,
            AdmissionPolicy::HotOnly => is_hot,
            AdmissionPolicy::Sampled(pct) => {
                (extent_uuid.as_bytes()[0] as u32 * 100 / 255) < pct as u32
            }
        };
        
        if admit_l1 {
            self.l1_cache.put(extent_uuid, data.clone(), is_hot);
        }
        
        // L2 admission
        let admit_l2 = match policy.l2_admission {
            AdmissionPolicy::Always => true,
            AdmissionPolicy::HotOnly => is_hot,
            AdmissionPolicy::Sampled(pct) => {
                (extent_uuid.as_bytes()[1] as u32 * 100 / 255) < pct as u32
            }
        };
        
        drop(policy);
        
        if admit_l2 {
            let mut l2 = self.l2_cache.lock().unwrap();
            l2.put(extent_uuid, data, is_hot)?;
        }
        
        Ok(())
    }
    
    /// Invalidate entry across all cache levels
    pub fn invalidate(&self, extent_uuid: &Uuid) -> Result<()> {
        self.l1_cache.invalidate(extent_uuid);
        
        let mut l2 = self.l2_cache.lock().unwrap();
        l2.invalidate(extent_uuid)?;
        
        if let Some(l3) = &self.l3_cache {
            let l3 = l3.lock().unwrap();
            l3.invalidate(extent_uuid)?;
        }
        
        Ok(())
    }
    
    /// Get comprehensive statistics
    pub fn stats(&self) -> MultiLevelCacheStats {
        self.stats.lock().unwrap().clone()
    }
    
    /// Flush all cache levels
    pub fn flush(&self) -> Result<()> {
        self.l1_cache.clear();
        
        let mut l2 = self.l2_cache.lock().unwrap();
        l2.flush()?;
        
        log::info!("Multi-level cache flushed");
        Ok(())
    }
}

/// Get current timestamp
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("System clock set before Unix epoch")
        .as_secs()
}

// Implement Serialize/Deserialize for L2CacheEntry
use serde::{Serialize, Deserialize};

impl Serialize for L2CacheEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("L2CacheEntry", 4)?;
        state.serialize_field("filename", &self.filename)?;
        state.serialize_field("size", &self.size)?;
        state.serialize_field("last_access", &self.last_access)?;
        state.serialize_field("is_hot", &self.is_hot)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for L2CacheEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            filename: String,
            size: usize,
            last_access: u64,
            is_hot: bool,
        }
        
        let helper = Helper::deserialize(deserializer)?;
        Ok(L2CacheEntry {
            filename: helper.filename,
            size: helper.size,
            last_access: helper.last_access,
            is_hot: helper.is_hot,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_l2_cache_basic() {
        let temp_dir = TempDir::new().unwrap();
        let mut l2 = L2Cache::new(temp_dir.path().to_path_buf(), 10 * 1024).unwrap();
        
        let uuid = Uuid::new_v4();
        let data = vec![1u8; 1024];
        
        // Miss
        assert!(l2.get(&uuid).unwrap().is_none());
        
        // Put and hit
        l2.put(uuid, data.clone(), false).unwrap();
        assert_eq!(l2.get(&uuid).unwrap(), Some(data));
    }

    #[test]
    fn test_multi_level_cache() {
        let temp_dir = TempDir::new().unwrap();
        let cache = MultiLevelCache::new(
            1024 * 1024,
            temp_dir.path().to_path_buf(),
            10 * 1024 * 1024,
        ).unwrap();
        
        let uuid = Uuid::new_v4();
        let data = vec![1u8; 1024];
        
        // Put and get
        cache.put(uuid, data.clone(), true).unwrap();
        assert_eq!(cache.get(&uuid, true), Some(data));
        
        // Check stats
        let stats = cache.stats();
        assert_eq!(stats.l1_hits, 1);
    }
}
