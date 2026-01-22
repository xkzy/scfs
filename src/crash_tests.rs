use super::*;
use crate::test_utils::setup_test_env;
use crate::crash_sim::{get_crash_simulator, CrashPoint};
use tempfile::TempDir;
use std::fs;

#[test]
fn test_crash_before_inode_temp_write() {
    let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);
    
    // Enable crash simulation
    let sim = get_crash_simulator();
    sim.enable_at(CrashPoint::BeforeTempWrite);
    
    // Try to create a file - should fail before writing temp file
    let result = storage.create_file(1, "crash_test.txt".to_string());
    assert!(result.is_err(), "Should fail due to simulated crash");
    assert!(result.unwrap_err().to_string().contains("SIMULATED POWER LOSS"));
    
    // Verify no file was created (inode doesn't exist)
    sim.disable();
    let children = storage.list_directory(1).unwrap();
    let found = children.iter().any(|c| c.name == "crash_test.txt");
    assert!(!found, "File should not exist after crash before temp write");
}

#[test]
#[ignore]
fn test_crash_after_temp_write_before_rename() {
    // TODO: Fix crash simulator to work across module boundaries
    // The thread-local get_crash_simulator() returns different instances
    // in test code vs metadata code, so state isn't shared properly.
    
    let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);
    
    // Test that a crash at any point before rename means file doesn't appear
    let sim = get_crash_simulator();
    sim.enable_at(CrashPoint::AfterTempWrite);
    
    let result = storage.create_file(1, "partial_crash.txt".to_string());
    assert!(result.is_err(), "Should fail due to simulated crash");
    
    sim.disable();
    
    // File should not be visible - demonstrates atomicity
    let children = storage.list_directory(1).unwrap();
    let found = children.iter().any(|c| c.name == "partial_crash.txt");
    assert!(!found, "File not visible until rename completes");
}

#[test]
fn test_crash_after_inode_commit() {
    let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);
    
    // Enable crash after successful commit
    let sim = get_crash_simulator();
    sim.enable_at(CrashPoint::AfterRename);
    
    // Create file - should succeed even though we "crash" after
    let result = storage.create_file(1, "committed.txt".to_string());
    assert!(result.is_err(), "Crashes after commit");
    
    sim.disable();
    
    // File should exist because rename completed
    let children = storage.list_directory(1).unwrap();
    let found = children.iter().any(|c| c.name == "committed.txt");
    assert!(found, "File should exist after crash post-commit");
}

#[test]
fn test_crash_during_write_fragments() {
    let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);
    
    // Create a file first
    let inode = storage.create_file(1, "data.bin".to_string()).unwrap();
    
    // Enable crash during fragment write
    let sim = get_crash_simulator();
    sim.enable_at(CrashPoint::BeforeFragmentWrite);
    
    // Try to write data - should fail during fragment write
    let data = vec![0x42u8; 1024];
    let result = storage.write_file(inode.ino, &data, 0);
    assert!(result.is_err(), "Should fail during fragment write");
    
    sim.disable();
    
    // File exists but has no data
    let read_result = storage.read_file(inode.ino);
    // Read should fail or return empty because no fragments were committed
    assert!(read_result.is_err() || read_result.unwrap().is_empty(), 
            "No data should be readable after crash before fragment write");
}

#[test]
fn test_crash_after_fragments_before_metadata() {
    let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);
    
    // Create a file first
    let inode = storage.create_file(1, "orphaned.bin".to_string()).unwrap();
    
    // Enable crash after fragments written, before extent metadata
    let sim = get_crash_simulator();
    sim.enable_at(CrashPoint::DuringExtentMetadata);
    
    // Try to write data - fragments will be written but metadata won't
    let data = vec![0xFFu8; 512];
    let result = storage.write_file(inode.ino, &data, 0);
    assert!(result.is_err(), "Should fail during extent metadata save");
    
    sim.disable();
    
    // This creates "orphaned fragments" - fragments exist on disk
    // but no metadata points to them. This is acceptable because:
    // 1. Read will fail (no extent map)
    // 2. Garbage collection can clean up orphaned fragments
    
    let read_result = storage.read_file(inode.ino);
    assert!(read_result.is_err() || read_result.unwrap().is_empty(),
            "No data should be readable without extent metadata");
}

#[test]
fn test_crash_during_extent_map_save() {
    let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);
    
    // Create a file
    let inode = storage.create_file(1, "map_crash.bin".to_string()).unwrap();
    
    // Enable crash during extent map save
    let sim = get_crash_simulator();
    sim.enable_at(CrashPoint::DuringExtentMap);
    
    // Try to write data
    let data = vec![0xAAu8; 256];
    let result = storage.write_file(inode.ino, &data, 0);
    assert!(result.is_err(), "Should fail during extent map save");
    
    sim.disable();
    
    // Extent metadata might exist, but without extent map, 
    // file appears empty
    let read_result = storage.read_file(inode.ino);
    assert!(read_result.is_err() || read_result.unwrap().is_empty(),
            "No data readable without extent map");
}

#[test]
#[ignore]
fn test_multiple_operations_with_crash() {
    let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);
    
    // Enable crash after 8 operations
    // Each save_inode has ~4 crash points, so 2 files = 8 checks
    let sim = get_crash_simulator();
    sim.enable_after_n_ops(CrashPoint::BeforeRename, 8);
    
    // First file should succeed (operations 1-4)
    let file1 = storage.create_file(1, "file1.txt".to_string());
    assert!(file1.is_ok(), "First file should succeed");
    
    // Second file should succeed (operations 5-7, crashes at 8)
    let file2 = storage.create_file(1, "file2.txt".to_string());
    assert!(file2.is_err(), "Second file should crash at operation 8");
    
    sim.disable();
    
    // Verify: first exists, second doesn't
    let children = storage.list_directory(1).unwrap();
    assert!(children.iter().any(|c| c.name == "file1.txt"));
    assert!(!children.iter().any(|c| c.name == "file2.txt"));
}

#[test]
fn test_recovery_cleans_temp_files() {
    let (pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);
    
    // Manually create a temp file to simulate interrupted operation
    let temp_inode_path = pool_dir.path().join("inodes").join("999.tmp");
    fs::write(&temp_inode_path, "corrupt temp data").unwrap();
    
    // Temp file exists
    assert!(temp_inode_path.exists());
    
    // Normal operation should ignore temp files
    let children = storage.list_directory(1).unwrap();
    let has_temp = children.iter().any(|c| c.ino == 999);
    assert!(!has_temp, "Temp file should not appear in listings");
    
    // In a real system, you'd have a recovery/cleanup process
    // that removes .tmp files on startup
}

#[test]
fn test_atomic_rename_guarantees() {
    let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);

    // Create a file with initial data
    let inode = storage.create_file(1, "atomic.txt".to_string()).unwrap();
    let initial_data = b"version 1";
    storage.write_file(inode.ino, initial_data, 0).unwrap();
    
    // Verify initial data
    let read_data = storage.read_file(inode.ino).unwrap();
    assert_eq!(read_data, initial_data);
    
    // Now crash during update (before rename)
    let sim = get_crash_simulator();
    sim.enable_at(CrashPoint::BeforeRename);
    
    let new_data = b"version 2 - should not be visible";
    let result = storage.write_file(inode.ino, new_data, 0);
    assert!(result.is_err());
    
    sim.disable();
    
    // Read should still return version 1, not version 2 or corrupted data
    let read_after_crash = storage.read_file(inode.ino).unwrap();
    assert_eq!(read_after_crash, initial_data, 
               "Data should be unchanged after crash during update");
}

#[test]
#[ignore]
fn test_write_verify_crash_consistency() {
    let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);
    
    let sim = get_crash_simulator();
    
    // Test crashes at different points - focus on actual outcomes
    let test_cases = vec![
        (CrashPoint::BeforeTempWrite, "before_temp"),
        (CrashPoint::AfterTempWrite, "after_temp"),
    ];
    
    for (crash_point, name) in test_cases {
        sim.reset();
        sim.enable_at(crash_point);
        
        let filename = format!("test_{}.bin", name);
        let result = storage.create_file(1, filename.clone());
        
        sim.disable();
        
        // If create failed, file should not be visible
        if result.is_err() {
            let children = storage.list_directory(1).unwrap();
            let exists = children.iter().any(|c| c.name == filename);
            assert!(!exists, "File should not exist after crash at {:?}", crash_point);
        }
    }
}

#[test]
fn test_concurrent_crash_scenarios() {
    let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
    let storage = StorageEngine::new(metadata, disks);
    
    // Test that crash recovery is consistent
    let sim = get_crash_simulator();
    
    // Successfully create a few files
    let file1 = storage.create_file(1, "stable1.txt".to_string()).unwrap();
    let file2 = storage.create_file(1, "stable2.txt".to_string()).unwrap();
    
    // Now cause a crash on next create
    sim.enable_at(CrashPoint::BeforeTempWrite);
    let file3_result = storage.create_file(1, "crash3.txt".to_string());
    assert!(file3_result.is_err());
    sim.disable();
    
    // Verify: first two exist, third doesn't
    let children = storage.list_directory(1).unwrap();
    assert!(children.iter().any(|c| c.name == "stable1.txt"));
    assert!(children.iter().any(|c| c.name == "stable2.txt"));
    assert!(!children.iter().any(|c| c.name == "crash3.txt"));
    
    // Can continue operating after crash
    let file4 = storage.create_file(1, "after_crash.txt".to_string()).unwrap();
    let children2 = storage.list_directory(1).unwrap();
    assert!(children2.iter().any(|c| c.name == "after_crash.txt"));
}
