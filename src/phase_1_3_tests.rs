use anyhow::Result;
use std::fs;
use uuid::Uuid;

use crate::disk::Disk;
use crate::extent::{Extent, RedundancyPolicy, FragmentLocation};
use crate::gc::{GarbageCollector};
use crate::metadata::{MetadataManager, Inode, ExtentMap};

#[test]
fn test_inode_checksum_verification() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let pool_dir = temp_dir.path().to_path_buf();
    
    let metadata = MetadataManager::new(pool_dir.clone())?;
    
    // Create and save an inode
    let inode = Inode::new_file(42, 1, "test.txt".to_string());
    metadata.save_inode(&inode)?;
    
    // Load it back - should succeed with valid checksum
    let loaded = metadata.load_inode(42)?;
    assert_eq!(loaded.ino, 42);
    assert_eq!(loaded.name, "test.txt");
    assert!(loaded.checksum.is_some(), "Checksum should be present");
    
    // Manually corrupt the saved inode
    let inode_path = pool_dir.join("inodes").join("42");
    let mut contents: serde_json::Value = serde_json::from_str(&fs::read_to_string(&inode_path)?)?;
    contents["size"] = serde_json::json!(999); // Change data but not checksum
    fs::write(&inode_path, serde_json::to_string_pretty(&contents)?)?;
    
    // Loading should now fail due to checksum mismatch
    let result = metadata.load_inode(42);
    assert!(result.is_err(), "Should fail with corrupted checksum");
    let err_msg = result.unwrap_err().to_string();
    eprintln!("Error message: {}", err_msg);
    assert!(err_msg.contains("checksum") || err_msg.contains("Corrupted"), 
        "Error should mention checksum or corruption, got: {}", err_msg);
    
    Ok(())
}

#[test]
fn test_extent_map_checksum_verification() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let pool_dir = temp_dir.path().to_path_buf();
    
    let metadata = MetadataManager::new(pool_dir.clone())?;
    
    // Create and save an extent map
    let extent_map = ExtentMap {
        ino: 42,
        extents: vec![Uuid::new_v4(), Uuid::new_v4()],
        checksum: None,
    };
    metadata.save_extent_map(&extent_map)?;
    
    // Load it back - should succeed with valid checksum
    let loaded = metadata.load_extent_map(42)?;
    assert_eq!(loaded.ino, 42);
    assert_eq!(loaded.extents.len(), 2);
    assert!(loaded.checksum.is_some(), "Checksum should be present");
    
    // Manually corrupt the saved extent map
    let map_path = pool_dir.join("extent_maps").join("42");
    let mut contents: serde_json::Value = serde_json::from_str(&fs::read_to_string(&map_path)?)?;
    contents["extents"] = serde_json::json!([]); // Remove extents but not checksum
    fs::write(&map_path, serde_json::to_string_pretty(&contents)?)?;
    
    // Loading should now fail due to checksum mismatch
    let result = metadata.load_extent_map(42);
    assert!(result.is_err(), "Should fail with corrupted checksum");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("checksum") || err_msg.contains("Corrupted"), 
        "Error should mention checksum or corruption, got: {}", err_msg);
    
    Ok(())
}

#[test]
fn test_orphan_detection() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let pool_dir = temp_dir.path().to_path_buf();
    
    // Set up metadata
    let metadata = MetadataManager::new(pool_dir.clone())?;
    
    // Create a disk
    let disk_dir = temp_dir.path().join("disk1");
    fs::create_dir_all(&disk_dir)?;
    let mut disk = Disk::new(disk_dir.clone())?;
    disk.save()?; // Save disk metadata
    
    // Create some fragment files
    let fragments_dir = disk_dir.join("fragments");
    fs::create_dir_all(&fragments_dir)?;
    
    let extent_uuid1 = Uuid::new_v4();
    let extent_uuid2 = Uuid::new_v4();
    
    // Create fragments on disk
    fs::write(fragments_dir.join(format!("{}_0", extent_uuid1)), b"data1")?;
    fs::write(fragments_dir.join(format!("{}_1", extent_uuid1)), b"data2")?;
    fs::write(fragments_dir.join(format!("{}_0", extent_uuid2)), b"data3")?; // orphan
    
    // Create extent metadata only for extent_uuid1
    let now = chrono::Utc::now().timestamp();
    let extent1 = Extent {
        uuid: extent_uuid1,
        size: 5,
        checksum: [0u8; 32], // BLAKE3 checksum
        redundancy: RedundancyPolicy::Replication { copies: 2 },
        fragment_locations: vec![
            FragmentLocation {
                disk_uuid: disk.uuid,
                fragment_index: 0,
            },
            FragmentLocation {
                disk_uuid: disk.uuid,
                fragment_index: 1,
            },
        ],
        access_stats: crate::extent::AccessStats {
            read_count: 0,
            write_count: 0,
            last_read: now,
            last_write: now,
            created_at: now,
            classification: crate::extent::AccessClassification::Cold,
            hmm_classifier: None,
        },
        previous_policy: None,
        policy_transitions: Vec::new(),
        last_policy_change: None,
    };
    metadata.save_extent(&extent1)?;
    
    // extent_uuid2 has no metadata, so it's an orphan
    
    // Run orphan detection
    let gc = GarbageCollector::new(pool_dir, vec![disk]);
    let orphans = gc.detect_orphans()?;
    
    // Should find exactly one orphan (extent_uuid2)
    assert_eq!(orphans.len(), 1, "Should find exactly one orphan");
    assert_eq!(orphans[0].extent_uuid, extent_uuid2);
    assert_eq!(orphans[0].fragment_index, 0);
    
    Ok(())
}

#[test]
fn test_orphan_cleanup() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let pool_dir = temp_dir.path().to_path_buf();
    
    // Set up metadata
    MetadataManager::new(pool_dir.clone())?;
    
    // Create a disk
    let disk_dir = temp_dir.path().join("disk1");    fs::create_dir_all(&disk_dir)?;    fs::create_dir_all(&disk_dir)?;
    let disk = Disk::new(disk_dir.clone())?;
    
    // Create orphaned fragments
    let fragments_dir = disk_dir.join("fragments");
    fs::create_dir_all(&fragments_dir)?;
    
    let orphan_uuid = Uuid::new_v4();
    let orphan_path = fragments_dir.join(format!("{}_0", orphan_uuid));
    fs::write(&orphan_path, b"orphan data")?;
    
    // Verify orphan exists
    assert!(orphan_path.exists());
    
    // Run cleanup (0 seconds min age = clean everything)
    let gc = GarbageCollector::new(pool_dir, vec![disk]);
    let cleaned = gc.cleanup_orphans(0, false)?;
    
    // Should have cleaned the orphan
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].extent_uuid, orphan_uuid);
    
    // Verify orphan was deleted
    assert!(!orphan_path.exists(), "Orphan should be deleted");
    
    Ok(())
}

#[test]
fn test_orphan_cleanup_age_filter() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let pool_dir = temp_dir.path().to_path_buf();
    
    // Set up metadata
    MetadataManager::new(pool_dir.clone())?;
    
    // Create a disk
    let disk_dir = temp_dir.path().join("disk1");
    fs::create_dir_all(&disk_dir)?;
    let disk = Disk::new(disk_dir.clone())?;
    
    // Create orphaned fragments
    let fragments_dir = disk_dir.join("fragments");
    fs::create_dir_all(&fragments_dir)?;
    
    let orphan_uuid = Uuid::new_v4();
    let orphan_path = fragments_dir.join(format!("{}_0", orphan_uuid));
    fs::write(&orphan_path, b"orphan data")?;
    
    // Run cleanup with very high min age (fragments are too new)
    let gc = GarbageCollector::new(pool_dir, vec![disk]);
    let cleaned = gc.cleanup_orphans(999999, false)?; // ~11.5 days
    
    // Should NOT have cleaned the orphan (too new)
    assert_eq!(cleaned.len(), 0);
    
    // Verify orphan still exists
    assert!(orphan_path.exists(), "Orphan should still exist");
    
    Ok(())
}

#[test]
fn test_orphan_cleanup_dry_run() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let pool_dir = temp_dir.path().to_path_buf();
    
    // Set up metadata
    MetadataManager::new(pool_dir.clone())?;
    
    // Create a disk
    let disk_dir = temp_dir.path().join("disk1");
    fs::create_dir_all(&disk_dir)?;
    let disk = Disk::new(disk_dir.clone())?;
    
    // Create orphaned fragments
    let fragments_dir = disk_dir.join("fragments");
    fs::create_dir_all(&fragments_dir)?;
    
    let orphan_uuid = Uuid::new_v4();
    let orphan_path = fragments_dir.join(format!("{}_0", orphan_uuid));
    fs::write(&orphan_path, b"orphan data")?;
    
    // Run cleanup in dry-run mode
    let gc = GarbageCollector::new(pool_dir, vec![disk]);
    let cleaned = gc.cleanup_orphans(0, true)?; // dry_run = true
    
    // Should report what would be cleaned
    assert_eq!(cleaned.len(), 1);
    
    // But orphan should still exist
    assert!(orphan_path.exists(), "Orphan should still exist in dry-run");
    
    Ok(())
}

#[test]
fn test_orphan_stats() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let pool_dir = temp_dir.path().to_path_buf();
    
    // Set up metadata
    MetadataManager::new(pool_dir.clone())?;
    
    // Create a disk
    let disk_dir = temp_dir.path().join("disk1");
    fs::create_dir_all(&disk_dir)?;
    let disk = Disk::new(disk_dir.clone())?;
    
    // Create orphaned fragments of various sizes
    let fragments_dir = disk_dir.join("fragments");
    fs::create_dir_all(&fragments_dir)?;
    
    fs::write(fragments_dir.join(format!("{}_0", Uuid::new_v4())), vec![0u8; 1000])?;
    fs::write(fragments_dir.join(format!("{}_0", Uuid::new_v4())), vec![0u8; 2000])?;
    fs::write(fragments_dir.join(format!("{}_0", Uuid::new_v4())), vec![0u8; 3000])?;
    
    // Get stats
    let gc = GarbageCollector::new(pool_dir, vec![disk]);
    let stats = gc.get_orphan_stats()?;
    
    // Should report all orphans
    assert_eq!(stats.total_count, 3);
    assert_eq!(stats.total_bytes, 6000);
    
    // All should be recent (< 24 hours)
    assert_eq!(stats.old_count, 0);
    assert_eq!(stats.old_bytes, 0);
    
    Ok(())
}
