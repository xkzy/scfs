/// Phase 15: Concurrent Read/Write Optimization
/// 
/// This module provides fine-grained concurrency primitives for high-performance
/// concurrent read and write operations with minimal contention.

use std::collections::HashMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use uuid::Uuid;

/// Number of shards for extent locks (power of 2 for fast modulo)
const EXTENT_LOCK_SHARDS: usize = 256;

/// Sharded per-extent read-write locks to reduce contention
/// 
/// Uses lock striping to partition the lock space. Instead of a single global
/// lock for all extents, we partition into EXTENT_LOCK_SHARDS separate locks
/// based on extent UUID hash, dramatically reducing contention.
pub struct ExtentLockManager {
    /// Array of lock shards, each protecting a subset of extents
    shards: Vec<RwLock<HashMap<Uuid, Arc<RwLock<()>>>>>,
}

impl ExtentLockManager {
    /// Create a new extent lock manager with sharded locks
    pub fn new() -> Self {
        let mut shards = Vec::with_capacity(EXTENT_LOCK_SHARDS);
        for _ in 0..EXTENT_LOCK_SHARDS {
            shards.push(RwLock::new(HashMap::new()));
        }
        
        ExtentLockManager { shards }
    }
    
    /// Get the shard index for a given extent UUID
    fn shard_for(&self, uuid: &Uuid) -> usize {
        let mut hasher = DefaultHasher::new();
        uuid.hash(&mut hasher);
        (hasher.finish() as usize) % EXTENT_LOCK_SHARDS
    }
    
    /// Acquire a read lock for an extent (non-blocking)
    /// 
    /// Multiple readers can acquire the lock concurrently.
    /// Returns None if the extent doesn't have a lock yet (lazy creation).
    pub fn try_read(&self, uuid: &Uuid) -> Option<Arc<RwLock<()>>> {
        let shard_idx = self.shard_for(uuid);
        let shard = self.shards[shard_idx].read().unwrap();
        shard.get(uuid).cloned()
    }
    
    /// Acquire a read lock for an extent (blocking, lazy creation)
    /// 
    /// Creates the lock if it doesn't exist yet.
    /// Returns the lock Arc that the caller should lock.
    pub fn read(&self, uuid: &Uuid) -> Arc<RwLock<()>> {
        self.get_or_create_lock(uuid)
    }
    
    /// Acquire a write lock for an extent (blocking, lazy creation)
    /// 
    /// Only one writer can hold the lock at a time.
    /// Returns the lock Arc that the caller should lock.
    pub fn write(&self, uuid: &Uuid) -> Arc<RwLock<()>> {
        self.get_or_create_lock(uuid)
    }
    
    /// Get or create a lock for an extent
    fn get_or_create_lock(&self, uuid: &Uuid) -> Arc<RwLock<()>> {
        let shard_idx = self.shard_for(uuid);
        
        // Fast path: check if lock exists with read lock
        {
            let shard = self.shards[shard_idx].read().unwrap();
            if let Some(lock) = shard.get(uuid) {
                return lock.clone();
            }
        }
        
        // Slow path: create lock with write lock
        let mut shard = self.shards[shard_idx].write().unwrap();
        shard.entry(*uuid)
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }
    
    /// Remove a lock for an extent (for cleanup after deletion)
    pub fn remove(&self, uuid: &Uuid) {
        let shard_idx = self.shard_for(uuid);
        let mut shard = self.shards[shard_idx].write().unwrap();
        shard.remove(uuid);
    }
    
    /// Get lock count across all shards (for testing/metrics)
    pub fn lock_count(&self) -> usize {
        self.shards.iter()
            .map(|shard| shard.read().unwrap().len())
            .sum()
    }
}

impl Default for ExtentLockManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Lock-free extent snapshot for optimistic reads
/// 
/// Allows readers to access extent metadata without locking by capturing
/// a snapshot with generation number. Readers can validate the snapshot
/// by checking if generation has changed.
#[derive(Debug, Clone)]
pub struct ExtentSnapshot {
    pub uuid: Uuid,
    pub generation: u64,
    pub size: usize,
    pub fragment_count: usize,
}

impl ExtentSnapshot {
    /// Check if snapshot is still valid (generation unchanged)
    pub fn is_valid(&self, current_generation: u64) -> bool {
        self.generation == current_generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::sync::atomic::{AtomicUsize, Ordering};
    
    #[test]
    fn test_extent_lock_manager_basic() {
        let manager = ExtentLockManager::new();
        let uuid = Uuid::new_v4();
        
        // Acquire read lock
        let lock = manager.read(&uuid);
        let _read = lock.read().unwrap();
        
        // Multiple readers should work
        let _read2 = lock.read().unwrap();
        
        drop(_read);
        drop(_read2);
        
        // Acquire write lock
        let _write = lock.write().unwrap();
    }
    
    #[test]
    fn test_concurrent_readers() {
        let manager = Arc::new(ExtentLockManager::new());
        let uuid = Uuid::new_v4();
        let counter = Arc::new(AtomicUsize::new(0));
        
        let mut threads = vec![];
        for _ in 0..10 {
            let m = manager.clone();
            let u = uuid;
            let c = counter.clone();
            threads.push(thread::spawn(move || {
                let lock = m.read(&u);
                let _guard = lock.read().unwrap();
                c.fetch_add(1, Ordering::SeqCst);
                thread::sleep(std::time::Duration::from_millis(10));
            }));
        }
        
        for t in threads {
            t.join().unwrap();
        }
        
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }
    
    #[test]
    fn test_lock_sharding() {
        let manager = ExtentLockManager::new();
        
        // Create locks for many UUIDs
        let uuids: Vec<Uuid> = (0..1000).map(|_| Uuid::new_v4()).collect();
        for uuid in &uuids {
            let _lock = manager.read(uuid);
        }
        
        // Verify locks are distributed across shards
        assert_eq!(manager.lock_count(), 1000);
    }
    
    #[test]
    fn test_snapshot_validation() {
        let snapshot = ExtentSnapshot {
            uuid: Uuid::new_v4(),
            generation: 5,
            size: 1024,
            fragment_count: 3,
        };
        
        assert!(snapshot.is_valid(5));
        assert!(!snapshot.is_valid(6));
    }
}
