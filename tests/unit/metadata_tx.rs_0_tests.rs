// moved from src/metadata_tx.rs
use super::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_metadata_root_creation() {
        let root = MetadataRoot::new(2);
        assert_eq!(root.version, 1);
        assert_eq!(root.next_ino, 2);
        assert_eq!(root.state, "committed");
    }
    
    #[test]
    fn test_root_versioning() {
        let root = MetadataRoot::new(2);
        let next = root.next_version();
        
        assert_eq!(next.version, 2);
        assert_eq!(next.state, "pending");
    }
    
    #[test]
    fn test_transaction_commit() {
        let temp_dir = TempDir::new().unwrap();
        let pool_dir = temp_dir.path();
        
        let root = MetadataRoot::new(2);
        let tx = MetadataTransaction::begin(pool_dir, root);
        
        let checksum = "test_checksum".to_string();
        let committed_root = tx.commit(checksum.clone()).unwrap();
        
        assert_eq!(committed_root.version, 2);
        assert_eq!(committed_root.state, "committed");
        assert_eq!(committed_root.state_checksum, checksum);
    }
    
    #[test]
    fn test_transaction_abort() {
        let temp_dir = TempDir::new().unwrap();
        let pool_dir = temp_dir.path();
        
        let root = MetadataRoot::new(2);
        let tx = MetadataTransaction::begin(pool_dir, root.clone());
        
        // Drop without commit
        drop(tx);
        
        // Root should still be at version 1
        let manager = MetadataRootManager::new(pool_dir.to_path_buf()).unwrap();
        assert_eq!(manager.current_root().version, 1);
    }
    
    #[test]
    fn test_root_manager_recovery() {
        let temp_dir = TempDir::new().unwrap();
        let pool_dir = temp_dir.path().to_path_buf();
        
        // Create manager and commit a transaction
        {
            let manager = MetadataRootManager::new(pool_dir.clone()).unwrap();
            let tx = manager.begin_transaction();
            manager.commit_transaction(tx, "checksum1".to_string()).unwrap();
        }
        
        // Create new manager - should recover to version 2
        let manager2 = MetadataRootManager::new(pool_dir).unwrap();
        assert_eq!(manager2.current_root().version, 2);
    }
    
    #[test]
    fn test_old_root_gc() {
        let temp_dir = TempDir::new().unwrap();
        let pool_dir = temp_dir.path().to_path_buf();
        
        let manager = MetadataRootManager::new(pool_dir).unwrap();
        
        // Commit 10 transactions
        for _ in 0..10 {
            let tx = manager.begin_transaction();
            let version = tx.current_root.version;
            manager.commit_transaction(tx, format!("checksum_{}", version)).unwrap();
        }
        
        assert_eq!(manager.current_root().version, 11);
        
        // Keep only last 3
        let deleted = manager.gc_old_roots(3).unwrap();
        assert!(deleted >= 7, "Should delete at least 7 old roots, deleted {}", deleted);
    }
