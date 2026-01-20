use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// File lock type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockType {
    Read,      // Shared lock
    Write,     // Exclusive lock
    Unlock,    // Remove lock
}

/// File lock information
#[derive(Debug, Clone)]
pub struct FileLock {
    pub owner: u64,     // Lock owner identifier
    pub pid: u32,       // Process ID
    pub lock_type: LockType,
    pub start: u64,     // Byte range start
    pub end: u64,       // Byte range end (inclusive, u64::MAX for EOF)
}

/// Lock manager for handling file locks
/// 
/// Current implementation uses Vec<FileLock> which results in O(m) operations
/// for lock conflict checking and removal, where m is the number of active locks.
/// 
/// Future optimization: For files with many locks (m >> 10), consider using:
/// - BTreeMap keyed by start offset for O(log m) range queries
/// - Interval tree for O(log m) overlap detection
/// - Current implementation is sufficient for typical use cases (< 100 locks/file)
pub struct LockManager {
    locks: Arc<RwLock<HashMap<u64, Vec<FileLock>>>>, // inode -> locks
}

impl LockManager {
    pub fn new() -> Self {
        LockManager {
            locks: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Test if a lock can be acquired
    pub fn test_lock(&self, ino: u64, lock: &FileLock) -> Result<Option<FileLock>> {
        let locks = self.locks.read().unwrap();
        
        if let Some(existing_locks) = locks.get(&ino) {
            for existing in existing_locks {
                // Check if ranges overlap
                if self.ranges_overlap(lock.start, lock.end, existing.start, existing.end) {
                    // Check for conflicts
                    if lock.lock_type == LockType::Write || existing.lock_type == LockType::Write {
                        // Different owners cause conflict
                        if lock.owner != existing.owner {
                            return Ok(Some(existing.clone()));
                        }
                    }
                }
            }
        }
        
        Ok(None)
    }
    
    /// Acquire a file lock
    pub fn acquire_lock(&self, ino: u64, lock: FileLock) -> Result<()> {
        let mut locks = self.locks.write().unwrap();
        
        // First, test for conflicts
        if let Some(existing_locks) = locks.get(&ino) {
            for existing in existing_locks {
                if self.ranges_overlap(lock.start, lock.end, existing.start, existing.end) {
                    if lock.lock_type == LockType::Write || existing.lock_type == LockType::Write {
                        if lock.owner != existing.owner {
                            return Err(anyhow!("Lock conflict detected"));
                        }
                    }
                }
            }
        }
        
        // Remove any existing locks from this owner in this range
        if let Some(existing_locks) = locks.get_mut(&ino) {
            existing_locks.retain(|l| {
                !(l.owner == lock.owner && 
                  self.ranges_overlap(lock.start, lock.end, l.start, l.end))
            });
        }
        
        // Add the new lock if not unlock
        if lock.lock_type != LockType::Unlock {
            locks.entry(ino).or_insert_with(Vec::new).push(lock);
        }
        
        Ok(())
    }
    
    /// Release a specific lock
    pub fn release_lock(&self, ino: u64, owner: u64, start: u64, end: u64) -> Result<()> {
        let mut locks = self.locks.write().unwrap();
        
        if let Some(existing_locks) = locks.get_mut(&ino) {
            existing_locks.retain(|l| {
                !(l.owner == owner && l.start == start && l.end == end)
            });
            
            if existing_locks.is_empty() {
                locks.remove(&ino);
            }
        }
        
        Ok(())
    }
    
    /// Release all locks for a given owner
    pub fn release_all_locks(&self, ino: u64, owner: u64) -> Result<()> {
        let mut locks = self.locks.write().unwrap();
        
        if let Some(existing_locks) = locks.get_mut(&ino) {
            existing_locks.retain(|l| l.owner != owner);
            
            if existing_locks.is_empty() {
                locks.remove(&ino);
            }
        }
        
        Ok(())
    }
    
    /// Get all locks for a file
    pub fn get_locks(&self, ino: u64) -> Vec<FileLock> {
        let locks = self.locks.read().unwrap();
        locks.get(&ino).cloned().unwrap_or_default()
    }
    
    /// Check if two byte ranges overlap
    fn ranges_overlap(&self, start1: u64, end1: u64, start2: u64, end2: u64) -> bool {
        !(end1 < start2 || end2 < start1)
    }
}

impl Default for LockManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_lock_basic() {
        let manager = LockManager::new();
        
        let lock = FileLock {
            owner: 1,
            pid: 100,
            lock_type: LockType::Write,
            start: 0,
            end: 100,
        };
        
        assert!(manager.acquire_lock(1, lock.clone()).is_ok());
        
        // Same owner can reacquire
        assert!(manager.acquire_lock(1, lock).is_ok());
    }
    
    #[test]
    fn test_lock_conflict() {
        let manager = LockManager::new();
        
        let lock1 = FileLock {
            owner: 1,
            pid: 100,
            lock_type: LockType::Write,
            start: 0,
            end: 100,
        };
        
        let lock2 = FileLock {
            owner: 2,
            pid: 200,
            lock_type: LockType::Write,
            start: 50,
            end: 150,
        };
        
        assert!(manager.acquire_lock(1, lock1).is_ok());
        assert!(manager.acquire_lock(1, lock2).is_err());
    }
    
    #[test]
    fn test_shared_locks() {
        let manager = LockManager::new();
        
        let lock1 = FileLock {
            owner: 1,
            pid: 100,
            lock_type: LockType::Read,
            start: 0,
            end: 100,
        };
        
        let lock2 = FileLock {
            owner: 2,
            pid: 200,
            lock_type: LockType::Read,
            start: 50,
            end: 150,
        };
        
        assert!(manager.acquire_lock(1, lock1).is_ok());
        assert!(manager.acquire_lock(1, lock2).is_ok());
    }
    
    #[test]
    fn test_unlock() {
        let manager = LockManager::new();
        
        let lock = FileLock {
            owner: 1,
            pid: 100,
            lock_type: LockType::Write,
            start: 0,
            end: 100,
        };
        
        assert!(manager.acquire_lock(1, lock.clone()).is_ok());
        assert!(manager.release_lock(1, 1, 0, 100).is_ok());
        
        // Should be able to acquire now
        let lock2 = FileLock {
            owner: 2,
            pid: 200,
            lock_type: LockType::Write,
            start: 0,
            end: 100,
        };
        assert!(manager.acquire_lock(1, lock2).is_ok());
    }
}
