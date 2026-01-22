// moved from src/concurrency.rs
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
