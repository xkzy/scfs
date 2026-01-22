// moved from src/file_locks.rs
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
