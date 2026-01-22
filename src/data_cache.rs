//! Data Caching Layer for Phase 10.3
//!
//! This module implements an in-memory LRU cache for frequently accessed extent data,
//! significantly reducing latency for hot data by avoiding disk I/O.
//!
//! ## Phase 10.3: Hot Data Caching Layer
//!
//! **Goal**: Achieve <1ms latency for cached reads with 80-90% hit rate
//!
//! ### Features
//! - LRU (Least Recently Used) eviction policy
//! - Configurable capacity based on available memory
//! - Per-extent UUID indexing for O(1) lookups
//! - Atomic read-through semantics
//! - Cache coherency with invalidation on writes
//! - Integration with HMM classifier for hot data prioritization
//!
//! ### Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │  Read Request (extent UUID)             │
//! └────────────────┬────────────────────────┘
//!                  │
//!                  v
//!         ┌────────────────┐
//!         │ Cache Lookup   │
//!         └────────┬───────┘
//!                  │
//!       ┌──────────┴──────────┐
//!       │                     │
//!    Hit│                     │Miss
//!       v                     v
//! ┌──────────┐         ┌─────────────┐
//! │ Return   │         │ Read from   │
//! │ Cached   │         │ Disk        │
//! │ Data     │         └──────┬──────┘
//! └──────────┘                │
//!                             v
//!                      ┌──────────────┐
//!                      │ Populate     │
//!                      │ Cache        │
//!                      └──────────────┘
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Cache entry containing extent data and metadata
#[derive(Clone)]
struct CacheEntry {
    /// The cached extent data
    data: Vec<u8>,
    
    /// Size in bytes
    size: usize,
    
    /// Last access timestamp (for LRU)
    last_access: u64,
    
    /// Access count (for statistics)
    access_count: u64,
    
    /// Whether this extent is classified as hot
    is_hot: bool,
}

/// LRU cache for extent data
///
/// Thread-safe cache implementation using LRU eviction policy.
/// Optimized for concurrent reads with minimal lock contention.
pub struct DataCache {
    /// Cache storage: extent UUID -> cached data
    entries: Arc<Mutex<HashMap<Uuid, CacheEntry>>>,
    
    /// Maximum cache size in bytes
    max_size_bytes: usize,
    
    /// Current cache size in bytes
    current_size: Arc<Mutex<usize>>,
    
    /// Cache statistics
    stats: Arc<Mutex<CacheStats>>,
}

/// Cache statistics for monitoring and debugging
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Total cache hits
    pub hits: u64,
    
    /// Total cache misses
    pub misses: u64,
    
    /// Total evictions
    pub evictions: u64,
    
    /// Total insertions
    pub insertions: u64,
    
    /// Total invalidations
    pub invalidations: u64,
    
    /// Current number of entries
    pub entry_count: usize,
    
    /// Current size in bytes
    pub size_bytes: usize,
}

impl CacheStats {
    /// Calculate cache hit rate (0.0 to 1.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
    
    /// Calculate eviction rate relative to insertions
    pub fn eviction_rate(&self) -> f64 {
        if self.insertions == 0 {
            0.0
        } else {
            self.evictions as f64 / self.insertions as f64
        }
    }
}

impl DataCache {
    /// Create a new data cache with specified maximum size
    ///
    /// # Arguments
    ///
    /// * `max_size_bytes` - Maximum cache size in bytes (e.g., 1GB = 1_073_741_824)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use dynamicfs::data_cache::DataCache;
    ///
    /// // Create 100MB cache
    /// let cache = DataCache::new(100 * 1024 * 1024);
    /// ```
    pub fn new(max_size_bytes: usize) -> Self {
        DataCache {
            entries: Arc::new(Mutex::new(HashMap::new())),
            max_size_bytes,
            current_size: Arc::new(Mutex::new(0)),
            stats: Arc::new(Mutex::new(CacheStats::default())),
        }
    }

    /// Get cached data for an extent
    ///
    /// # Arguments
    ///
    /// * `extent_uuid` - UUID of the extent to look up
    ///
    /// # Returns
    ///
    /// * `Some(data)` if the extent is cached
    /// * `None` if the extent is not in cache
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// if let Some(data) = cache.get(&extent_uuid) {
    ///     // Use cached data
    ///     return Ok(data);
    /// }
    /// // Cache miss - read from disk
    /// ```
    pub fn get(&self, extent_uuid: &Uuid) -> Option<Vec<u8>> {
        let mut entries = self.entries.lock().unwrap();
        let mut stats = self.stats.lock().unwrap();
        
        if let Some(entry) = entries.get_mut(extent_uuid) {
            // Cache hit - update access time and count
            entry.last_access = current_timestamp();
            entry.access_count += 1;
            stats.hits += 1;
            
            log::trace!("Cache hit for extent {}", extent_uuid);
            Some(entry.data.clone())
        } else {
            // Cache miss
            stats.misses += 1;
            log::trace!("Cache miss for extent {}", extent_uuid);
            None
        }
    }

    /// Insert data into cache
    ///
    /// # Arguments
    ///
    /// * `extent_uuid` - UUID of the extent
    /// * `data` - Extent data to cache
    /// * `is_hot` - Whether this extent is classified as hot (prioritized)
    ///
    /// If the cache is full, this will evict the least recently used entry.
    /// Hot extents are less likely to be evicted.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Cache hot extent after reading from disk
    /// if is_hot_extent {
    ///     cache.put(extent_uuid, data.clone(), true);
    /// }
    /// ```
    pub fn put(&self, extent_uuid: Uuid, data: Vec<u8>, is_hot: bool) {
        let data_size = data.len();
        
        // Check if we need to evict entries to make space
        self.maybe_evict(data_size);
        
        let entry = CacheEntry {
            data,
            size: data_size,
            last_access: current_timestamp(),
            access_count: 1,
            is_hot,
        };
        
        let mut entries = self.entries.lock().unwrap();
        let mut stats = self.stats.lock().unwrap();
        
        // Remove old entry if exists (to get accurate size)
        if let Some(old_entry) = entries.remove(&extent_uuid) {
            let mut size = self.current_size.lock().unwrap();
            *size = size.saturating_sub(old_entry.size);
        }
        
        // Insert new entry
        entries.insert(extent_uuid, entry);
        stats.insertions += 1;
        
        // Update size
        let mut size = self.current_size.lock().unwrap();
        *size += data_size;
        
        log::trace!("Cached extent {} ({} bytes, hot={})", extent_uuid, data_size, is_hot);
    }

    /// Invalidate (remove) an extent from cache
    ///
    /// Called when extent is modified, deleted, or rebuilt to maintain cache coherency.
    ///
    /// # Arguments
    ///
    /// * `extent_uuid` - UUID of the extent to invalidate
    pub fn invalidate(&self, extent_uuid: &Uuid) {
        let mut entries = self.entries.lock().unwrap();
        
        if let Some(entry) = entries.remove(extent_uuid) {
            let mut size = self.current_size.lock().unwrap();
            *size = size.saturating_sub(entry.size);
            
            let mut stats = self.stats.lock().unwrap();
            stats.invalidations += 1;
            
            log::debug!("Invalidated cache entry for extent {}", extent_uuid);
        }
    }

    /// Evict entries to make space for new data
    ///
    /// Uses LRU policy, but preferentially evicts cold extents before hot ones.
    fn maybe_evict(&self, needed_bytes: usize) {
        let current = *self.current_size.lock().unwrap();
        
        if current + needed_bytes <= self.max_size_bytes {
            return; // No eviction needed
        }
        
        let to_free = (current + needed_bytes) - self.max_size_bytes;
        let mut freed = 0;
        
        // Lock order: entries -> stats -> current_size (consistent with put())
        let mut entries = self.entries.lock().unwrap();
        let mut stats = self.stats.lock().unwrap();
        
        // Build list of candidates with named struct for clarity
        #[derive(Debug)]
        struct EvictionCandidate {
            uuid: Uuid,
            last_access: u64,
            is_hot: bool,
            size: usize,
        }
        
        let mut candidates: Vec<EvictionCandidate> = entries
            .iter()
            .map(|(uuid, entry)| EvictionCandidate {
                uuid: *uuid,
                last_access: entry.last_access,
                is_hot: entry.is_hot,
                size: entry.size,
            })
            .collect();
        
        // Sort by priority: cold first, then by LRU
        // Hot extents have higher priority (sort to end)
        candidates.sort_by(|a, b| {
            // First compare by hot status (cold first)
            match (a.is_hot, b.is_hot) {
                (false, true) => std::cmp::Ordering::Less,  // Cold < Hot
                (true, false) => std::cmp::Ordering::Greater, // Hot > Cold
                _ => a.last_access.cmp(&b.last_access), // Same hot status, compare by LRU
            }
        });
        
        // Evict entries until we've freed enough space
        for candidate in candidates {
            if freed >= to_free {
                break;
            }
            
            entries.remove(&candidate.uuid);
            freed += candidate.size;
            stats.evictions += 1;
            
            log::trace!("Evicted extent {} ({} bytes)", candidate.uuid, candidate.size);
        }
        
        // Update size (lock acquired last for consistent ordering)
        let mut current_size = self.current_size.lock().unwrap();
        *current_size = current_size.saturating_sub(freed);
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let stats = self.stats.lock().unwrap();
        let mut result = stats.clone();
        
        // Update current counts
        let entries = self.entries.lock().unwrap();
        result.entry_count = entries.len();
        result.size_bytes = *self.current_size.lock().unwrap();
        
        result
    }

    /// Clear all cache entries
    pub fn clear(&self) {
        let mut entries = self.entries.lock().unwrap();
        entries.clear();
        
        let mut size = self.current_size.lock().unwrap();
        *size = 0;
        
        log::info!("Cache cleared");
    }

    /// Get current cache size in bytes
    pub fn size_bytes(&self) -> usize {
        *self.current_size.lock().unwrap()
    }

    /// Get maximum cache size
    pub fn max_size_bytes(&self) -> usize {
        self.max_size_bytes
    }

    /// Get cache utilization (0.0 to 1.0)
    pub fn utilization(&self) -> f64 {
        let current = self.size_bytes() as f64;
        let max = self.max_size_bytes as f64;
        if max == 0.0 {
            0.0
        } else {
            current / max
        }
    }
}

/// Get current timestamp in seconds since UNIX epoch
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("System clock set before Unix epoch")
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic_operations() {
        let cache = DataCache::new(1024 * 1024); // 1MB cache
        let uuid = Uuid::new_v4();
        let data = vec![1u8; 1024]; // 1KB data

        // Miss on empty cache
        assert!(cache.get(&uuid).is_none());

        // Insert and hit
        cache.put(uuid, data.clone(), false);
        assert_eq!(cache.get(&uuid), Some(data.clone()));

        // Stats
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.insertions, 1);
    }

    #[test]
    fn test_cache_eviction() {
        let cache = DataCache::new(2048); // 2KB cache
        
        // Fill cache with 2 entries
        let uuid1 = Uuid::new_v4();
        let uuid2 = Uuid::new_v4();
        let uuid3 = Uuid::new_v4();
        
        cache.put(uuid1, vec![1u8; 1000], false);
        cache.put(uuid2, vec![2u8; 1000], false);
        
        // This should trigger eviction of uuid1 (LRU)
        cache.put(uuid3, vec![3u8; 500], false);
        
        // uuid1 should be evicted
        assert!(cache.get(&uuid1).is_none());
        assert!(cache.get(&uuid2).is_some());
        assert!(cache.get(&uuid3).is_some());
        
        let stats = cache.stats();
        assert!(stats.evictions > 0);
    }

    #[test]
    fn test_cache_hot_priority() {
        let cache = DataCache::new(2048); // 2KB cache
        
        // Insert cold and hot entries
        let cold_uuid = Uuid::new_v4();
        let hot_uuid = Uuid::new_v4();
        
        cache.put(cold_uuid, vec![1u8; 1000], false); // Cold
        cache.put(hot_uuid, vec![2u8; 1000], true);   // Hot
        
        // Add another entry to trigger eviction
        let new_uuid = Uuid::new_v4();
        cache.put(new_uuid, vec![3u8; 500], false);
        
        // Cold entry should be evicted before hot
        assert!(cache.get(&cold_uuid).is_none());
        assert!(cache.get(&hot_uuid).is_some());
    }

    #[test]
    fn test_cache_invalidation() {
        let cache = DataCache::new(1024 * 1024);
        let uuid = Uuid::new_v4();
        let data = vec![1u8; 1024];

        cache.put(uuid, data.clone(), false);
        assert!(cache.get(&uuid).is_some());

        cache.invalidate(&uuid);
        assert!(cache.get(&uuid).is_none());

        let stats = cache.stats();
        assert_eq!(stats.invalidations, 1);
    }

    #[test]
    fn test_cache_stats() {
        let cache = DataCache::new(1024 * 1024);
        
        // Perform various operations
        let uuid = Uuid::new_v4();
        cache.get(&uuid); // Miss
        cache.put(uuid, vec![1u8; 100], false);
        cache.get(&uuid); // Hit
        cache.get(&uuid); // Hit
        
        let stats = cache.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.insertions, 1);
        assert!(stats.hit_rate() > 0.6); // 2/3 = 0.666...
    }

    #[test]
    fn test_cache_clear() {
        let cache = DataCache::new(1024 * 1024);
        let uuid = Uuid::new_v4();
        
        cache.put(uuid, vec![1u8; 1024], false);
        assert!(cache.get(&uuid).is_some());
        
        cache.clear();
        assert!(cache.get(&uuid).is_none());
        assert_eq!(cache.size_bytes(), 0);
    }

    #[test]
    fn test_cache_utilization() {
        let cache = DataCache::new(1000);
        assert_eq!(cache.utilization(), 0.0);
        
        let uuid = Uuid::new_v4();
        cache.put(uuid, vec![1u8; 500], false);
        
        let util = cache.utilization();
        assert!(util > 0.4 && util < 0.6); // ~50% utilization
    }
}
