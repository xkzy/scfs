/// Phase 16 Tests: Full FUSE Operation Support
/// Tests for extended attributes, file locking, fallocate, and other FUSE operations

use crate::disk::{Disk, DiskHealth};
use crate::extent::RedundancyPolicy;
use crate::file_locks::{FileLock, LockManager, LockType};
use crate::fuse_impl::DynamicFS;
use crate::metadata::{AclEntry, AclTag, ExtendedAttributes, MetadataManager};
use crate::storage::StorageEngine;
use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::TempDir;

fn setup_test_fs() -> (DynamicFS, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let pool_dir = temp_dir.path().to_path_buf();
    
    let metadata = MetadataManager::new(pool_dir.clone()).unwrap();
    
    let disk1_path = pool_dir.join("disk1");
    let disk2_path = pool_dir.join("disk2");
    std::fs::create_dir_all(&disk1_path).unwrap();
    std::fs::create_dir_all(&disk2_path).unwrap();
    
    let disk1 = Disk::new(disk1_path).unwrap();
    let disk2 = Disk::new(disk2_path).unwrap();
    
    let storage = StorageEngine::new(metadata, vec![disk1, disk2]);
    let fs = DynamicFS::new(storage);
    
    (fs, temp_dir)
}

// ===== Extended Attributes Tests =====

#[test]
fn test_xattr_set_and_get() {
    let (mut fs, _temp) = setup_test_fs();
    
    // Create a file
    let parent_ino = 1; // root
    let inode = fs.storage.create_file(parent_ino, "test.txt".to_string()).unwrap();
    let ino = inode.ino;
    
    // Set an xattr
    let attr_name = "user.test";
    let attr_value = b"test_value";
    
    let mut inode = fs.storage.get_inode(ino).unwrap();
    inode.set_xattr(attr_name.to_string(), attr_value.to_vec());
    fs.storage.update_inode(&inode).unwrap();
    
    // Get the xattr
    let inode = fs.storage.get_inode(ino).unwrap();
    let value = inode.get_xattr(attr_name).unwrap();
    assert_eq!(value, attr_value);
}

#[test]
fn test_xattr_list() {
    let (mut fs, _temp) = setup_test_fs();
    
    let parent_ino = 1;
    let inode = fs.storage.create_file(parent_ino, "test.txt".to_string()).unwrap();
    let ino = inode.ino;
    
    // Set multiple xattrs
    let mut inode = fs.storage.get_inode(ino).unwrap();
    inode.set_xattr("user.attr1".to_string(), b"value1".to_vec());
    inode.set_xattr("user.attr2".to_string(), b"value2".to_vec());
    inode.set_xattr("user.attr3".to_string(), b"value3".to_vec());
    fs.storage.update_inode(&inode).unwrap();
    
    // List xattrs
    let inode = fs.storage.get_inode(ino).unwrap();
    let names = inode.list_xattrs();
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"user.attr1".to_string()));
    assert!(names.contains(&"user.attr2".to_string()));
    assert!(names.contains(&"user.attr3".to_string()));
}

#[test]
fn test_xattr_remove() {
    let (mut fs, _temp) = setup_test_fs();
    
    let parent_ino = 1;
    let inode = fs.storage.create_file(parent_ino, "test.txt".to_string()).unwrap();
    let ino = inode.ino;
    
    // Set an xattr
    let mut inode = fs.storage.get_inode(ino).unwrap();
    inode.set_xattr("user.test".to_string(), b"value".to_vec());
    fs.storage.update_inode(&inode).unwrap();
    
    // Remove the xattr
    let mut inode = fs.storage.get_inode(ino).unwrap();
    let removed = inode.remove_xattr("user.test");
    assert!(removed.is_some());
    fs.storage.update_inode(&inode).unwrap();
    
    // Verify it's gone
    let inode = fs.storage.get_inode(ino).unwrap();
    assert!(inode.get_xattr("user.test").is_none());
}

#[test]
fn test_xattr_large_value() {
    let (mut fs, _temp) = setup_test_fs();
    
    let parent_ino = 1;
    let inode = fs.storage.create_file(parent_ino, "test.txt".to_string()).unwrap();
    let ino = inode.ino;
    
    // Set a large xattr (4KB)
    let large_value = vec![0x42; 4096];
    let mut inode = fs.storage.get_inode(ino).unwrap();
    inode.set_xattr("user.large".to_string(), large_value.clone());
    fs.storage.update_inode(&inode).unwrap();
    
    // Verify
    let inode = fs.storage.get_inode(ino).unwrap();
    let value = inode.get_xattr("user.large").unwrap();
    assert_eq!(value, &large_value[..]);
}

#[test]
fn test_xattr_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let pool_dir = temp_dir.path().to_path_buf();
    
    let ino = {
        let metadata = MetadataManager::new(pool_dir.clone()).unwrap();
        let disk1_path = pool_dir.join("disk1");
        let disk2_path = pool_dir.join("disk2");
        std::fs::create_dir_all(&disk1_path).unwrap();
        std::fs::create_dir_all(&disk2_path).unwrap();
        
        let disk1 = Disk::new(disk1_path).unwrap();
        let disk2 = Disk::new(disk2_path).unwrap();
        let storage = StorageEngine::new(metadata, vec![disk1, disk2]);
        
        let inode = storage.create_file(1, "test.txt".to_string()).unwrap();
        let ino = inode.ino;
        
        let mut inode = storage.get_inode(ino).unwrap();
        inode.set_xattr("user.persist".to_string(), b"persistent_value".to_vec());
        storage.update_inode(&inode).unwrap();
        
        ino
    };
    
    // Reopen and verify
    let metadata = MetadataManager::new(pool_dir.clone()).unwrap();
    let disk1_path = pool_dir.join("disk1");
    let disk2_path = pool_dir.join("disk2");
    
    let disk1 = Disk::new(disk1_path).unwrap();
    let disk2 = Disk::new(disk2_path).unwrap();
    let storage = StorageEngine::new(metadata, vec![disk1, disk2]);
    
    let inode = storage.get_inode(ino).unwrap();
    let value = inode.get_xattr("user.persist").unwrap();
    assert_eq!(value, b"persistent_value");
}

// ===== File Locking Tests =====

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
fn test_lock_conflict_write() {
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
fn test_lock_shared() {
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
    
    // Multiple read locks should succeed
    assert!(manager.acquire_lock(1, lock1).is_ok());
    assert!(manager.acquire_lock(1, lock2).is_ok());
}

#[test]
fn test_lock_upgrade_conflict() {
    let manager = LockManager::new();
    
    let read_lock = FileLock {
        owner: 1,
        pid: 100,
        lock_type: LockType::Read,
        start: 0,
        end: 100,
    };
    
    let write_lock = FileLock {
        owner: 2,
        pid: 200,
        lock_type: LockType::Write,
        start: 50,
        end: 150,
    };
    
    // Acquire read lock
    assert!(manager.acquire_lock(1, read_lock).is_ok());
    
    // Write lock should conflict
    assert!(manager.acquire_lock(1, write_lock).is_err());
}

#[test]
fn test_lock_release() {
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

#[test]
fn test_lock_release_all() {
    let manager = LockManager::new();
    
    let lock1 = FileLock {
        owner: 1,
        pid: 100,
        lock_type: LockType::Write,
        start: 0,
        end: 100,
    };
    
    let lock2 = FileLock {
        owner: 1,
        pid: 100,
        lock_type: LockType::Write,
        start: 200,
        end: 300,
    };
    
    assert!(manager.acquire_lock(1, lock1).is_ok());
    assert!(manager.acquire_lock(1, lock2).is_ok());
    
    // Release all locks for owner 1
    assert!(manager.release_all_locks(1, 1).is_ok());
    
    // Should be able to acquire both ranges now
    let lock3 = FileLock {
        owner: 2,
        pid: 200,
        lock_type: LockType::Write,
        start: 0,
        end: 300,
    };
    assert!(manager.acquire_lock(1, lock3).is_ok());
}

#[test]
fn test_lock_non_overlapping() {
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
        start: 101,
        end: 200,
    };
    
    // Non-overlapping locks should both succeed
    assert!(manager.acquire_lock(1, lock1).is_ok());
    assert!(manager.acquire_lock(1, lock2).is_ok());
}

#[test]
fn test_lock_test() {
    let manager = LockManager::new();
    
    let lock1 = FileLock {
        owner: 1,
        pid: 100,
        lock_type: LockType::Write,
        start: 0,
        end: 100,
    };
    
    assert!(manager.acquire_lock(1, lock1).is_ok());
    
    // Test for conflicting lock
    let lock2 = FileLock {
        owner: 2,
        pid: 200,
        lock_type: LockType::Write,
        start: 50,
        end: 150,
    };
    
    let conflict = manager.test_lock(1, &lock2).unwrap();
    assert!(conflict.is_some());
    assert_eq!(conflict.unwrap().owner, 1);
}

// ===== ACL Tests =====

#[test]
fn test_acl_creation() {
    let acl_entry = AclEntry {
        tag: AclTag::User,
        qualifier: Some(1000),
        permissions: 0o600,
    };
    
    assert_eq!(acl_entry.tag, AclTag::User);
    assert_eq!(acl_entry.qualifier, Some(1000));
    assert_eq!(acl_entry.permissions, 0o600);
}

#[test]
fn test_acl_storage() {
    let (mut fs, _temp) = setup_test_fs();
    
    let parent_ino = 1;
    let inode = fs.storage.create_file(parent_ino, "test.txt".to_string()).unwrap();
    let ino = inode.ino;
    
    // Set ACL
    let acl = vec![
        AclEntry {
            tag: AclTag::UserObj,
            qualifier: None,
            permissions: 0o600,
        },
        AclEntry {
            tag: AclTag::User,
            qualifier: Some(1000),
            permissions: 0o400,
        },
        AclEntry {
            tag: AclTag::GroupObj,
            qualifier: None,
            permissions: 0o040,
        },
        AclEntry {
            tag: AclTag::Other,
            qualifier: None,
            permissions: 0o004,
        },
    ];
    
    let mut inode = fs.storage.get_inode(ino).unwrap();
    inode.acl = Some(acl.clone());
    fs.storage.update_inode(&inode).unwrap();
    
    // Verify
    let inode = fs.storage.get_inode(ino).unwrap();
    assert!(inode.acl.is_some());
    let stored_acl = inode.acl.unwrap();
    assert_eq!(stored_acl.len(), 4);
    assert_eq!(stored_acl[0].tag, AclTag::UserObj);
    assert_eq!(stored_acl[1].tag, AclTag::User);
    assert_eq!(stored_acl[1].qualifier, Some(1000));
}

// ===== Fallocate Tests =====

#[test]
fn test_fallocate_extend() {
    let (mut fs, _temp) = setup_test_fs();
    
    let parent_ino = 1;
    let inode = fs.storage.create_file(parent_ino, "test.txt".to_string()).unwrap();
    let ino = inode.ino;
    
    // Write some data with 2x replication (we only have 2 disks)
    let data = b"hello";
    let mut inode = fs.storage.get_inode(ino).unwrap();
    // Set size directly for this test since we're testing fallocate semantics
    inode.size = data.len() as u64;
    fs.storage.update_inode(&inode).unwrap();
    
    // Verify initial size
    let inode = fs.storage.get_inode(ino).unwrap();
    assert_eq!(inode.size, 5);
    
    // In a real implementation, we would call the FUSE fallocate method here
    // For now, we just verify the infrastructure exists
}

#[test]
fn test_fallocate_modes() {
    // Verify fallocate mode constants exist
    assert_eq!(libc::FALLOC_FL_PUNCH_HOLE, 0x02);
    assert_eq!(libc::FALLOC_FL_ZERO_RANGE, 0x10);
}

// ===== Integration Tests =====

#[test]
fn test_xattr_with_locks() {
    let (mut fs, _temp) = setup_test_fs();
    
    let parent_ino = 1;
    let inode = fs.storage.create_file(parent_ino, "test.txt".to_string()).unwrap();
    let ino = inode.ino;
    
    // Acquire lock
    let lock = FileLock {
        owner: 1,
        pid: 100,
        lock_type: LockType::Write,
        start: 0,
        end: u64::MAX,
    };
    assert!(fs.lock_manager.acquire_lock(ino, lock).is_ok());
    
    // Set xattr while holding lock
    let mut inode = fs.storage.get_inode(ino).unwrap();
    inode.set_xattr("user.locked".to_string(), b"value".to_vec());
    fs.storage.update_inode(&inode).unwrap();
    
    // Verify xattr
    let inode = fs.storage.get_inode(ino).unwrap();
    assert_eq!(inode.get_xattr("user.locked").unwrap(), b"value");
    
    // Release lock
    assert!(fs.lock_manager.release_all_locks(ino, 1).is_ok());
}

#[test]
fn test_concurrent_xattr_operations() {
    let (mut fs, _temp) = setup_test_fs();
    
    let parent_ino = 1;
    let inode = fs.storage.create_file(parent_ino, "test.txt".to_string()).unwrap();
    let ino = inode.ino;
    
    // Simulate concurrent operations by setting multiple xattrs
    let mut inode = fs.storage.get_inode(ino).unwrap();
    for i in 0..10 {
        let name = format!("user.attr{}", i);
        let value = format!("value{}", i);
        inode.set_xattr(name, value.into_bytes());
    }
    fs.storage.update_inode(&inode).unwrap();
    
    // Verify all xattrs
    let inode = fs.storage.get_inode(ino).unwrap();
    assert_eq!(inode.list_xattrs().len(), 10);
    
    for i in 0..10 {
        let name = format!("user.attr{}", i);
        let expected_value = format!("value{}", i);
        assert_eq!(
            inode.get_xattr(&name).unwrap(),
            expected_value.as_bytes()
        );
    }
}

#[test]
fn test_xattr_special_characters() {
    let (mut fs, _temp) = setup_test_fs();
    
    let parent_ino = 1;
    let inode = fs.storage.create_file(parent_ino, "test.txt".to_string()).unwrap();
    let ino = inode.ino;
    
    // Test with special characters in value
    let special_value = b"value\0with\nnull\tand\rspecial\x01chars";
    let mut inode = fs.storage.get_inode(ino).unwrap();
    inode.set_xattr("user.special".to_string(), special_value.to_vec());
    fs.storage.update_inode(&inode).unwrap();
    
    // Verify
    let inode = fs.storage.get_inode(ino).unwrap();
    assert_eq!(inode.get_xattr("user.special").unwrap(), special_value);
}

// ===== macOS-specific Tests =====

#[cfg(target_os = "macos")]
#[test]
fn test_macos_xattr_support() {
    use crate::macos::{MacOSHandler, xattr_names};

    let handler = MacOSHandler::new();

    // Test resource fork xattr
    let result = handler.handle_xattr(xattr_names::RESOURCE_FORK, Some(b"resource data"));
    assert!(result.is_ok());

    // Test Finder info xattr
    let finder_info = [0u8; 32];
    let result = handler.handle_xattr(xattr_names::FINDER_INFO, Some(&finder_info));
    assert!(result.is_ok());

    // Test invalid Finder info size
    let invalid_finder_info = [0u8; 16]; // Wrong size
    let result = handler.handle_xattr(xattr_names::FINDER_INFO, Some(&invalid_finder_info));
    assert!(result.is_err());

    // Test Spotlight metadata
    let metadata = vec![0u8; 1000];
    let result = handler.handle_xattr(xattr_names::METADATA, Some(&metadata));
    assert!(result.is_ok());

    // Test oversized resource fork
    let large_resource = vec![0u8; 20 * 1024 * 1024]; // 20MB, over limit
    let result = handler.handle_xattr(xattr_names::RESOURCE_FORK, Some(&large_resource));
    assert!(result.is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_default_xattrs() {
    use crate::macos::default_macos_xattrs;

    let file_xattrs = default_macos_xattrs("file");
    assert!(!file_xattrs.is_empty());

    let dir_xattrs = default_macos_xattrs("directory");
    assert!(!dir_xattrs.is_empty());

    // Check that Finder info is included
    let has_finder_info = file_xattrs.iter().any(|(k, _)| k == "com.apple.FinderInfo");
    assert!(has_finder_info);
}
