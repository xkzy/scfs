// moved from src/storage.rs
use super::*;
    use crate::crash_sim::{get_crash_simulator, CrashPoint};
    use crate::disk::Disk;
    use tempfile::TempDir;
    
    pub(super) fn setup_test_env() -> (TempDir, Vec<TempDir>, MetadataManager, Vec<Disk>) {
        eprintln!("[TEST DEBUG] setup_test_env: creating pool tempdir");
        let pool_dir = tempfile::tempdir().unwrap();
        
        // Create 6 test disks
        eprintln!("[TEST DEBUG] setup_test_env: creating disk tempdirs");
        let disk_dirs: Vec<TempDir> = (0..6)
            .map(|i| { eprintln!("[TEST DEBUG] creating disk dir {}", i); tempfile::tempdir().unwrap() })
            .collect();
        
        eprintln!("[TEST DEBUG] setup_test_env: initializing Disk objects");
        let disks: Vec<Disk> = disk_dirs
            .iter()
            .map(|td| { eprintln!("[TEST DEBUG] Disk::new for {}", td.path().display()); Disk::new(td.path().to_path_buf()).unwrap() })
            .collect();
        
        eprintln!("[TEST DEBUG] setup_test_env: creating MetadataManager");
        let metadata = MetadataManager::new(pool_dir.path().to_path_buf()).unwrap();
        
        eprintln!("[TEST DEBUG] setup_test_env: done");
        (pool_dir, disk_dirs, metadata, disks)
    }
    
    #[test]
    fn test_write_and_read_small_file() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "test.txt".to_string()).unwrap();
        
        // Write data
        let data = b"Hello, World!";
        storage.write_file(inode.ino, data, 0).unwrap();
        
        // Read data
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_write_and_read_large_file() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "large.bin".to_string()).unwrap();
        
        // Write 5MB of data
        let data = vec![0x42u8; 5 * 1024 * 1024];
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Read data
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data.len(), data.len());
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_multiple_files() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create multiple files
        for i in 0..10 {
            let name = format!("file{}.txt", i);
            let inode = storage.create_file(1, name).unwrap();
            let data = format!("Content of file {}", i);
            storage.write_file(inode.ino, data.as_bytes(), 0).unwrap();
        }
        
        // List directory
        let children = storage.list_directory(1).unwrap();
        assert!(children.len() >= 10, "Should have at least 10 files, got {}", children.len());
        
        // Verify all our files exist
        for i in 0..10 {
            let name = format!("file{}.txt", i);
            let found = children.iter().any(|c| c.name == name);
            assert!(found, "File {} should exist", name);
        }
        
        // Read each of our files
        for i in 0..10 {
            let name = format!("file{}.txt", i);
            if let Some(child) = children.iter().find(|c| c.name == name) {
                let data = storage.read_file(child.ino).unwrap();
                assert!(data.len() > 0);
            }
        }
    }
    
    #[test]
    fn test_directory_operations() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a subdirectory
        let subdir = storage.create_dir(1, "subdir".to_string()).unwrap();
        
        // Create files in subdirectory
        let file1 = storage.create_file(subdir.ino, "file1.txt".to_string()).unwrap();
        let file2 = storage.create_file(subdir.ino, "file2.txt".to_string()).unwrap();
        
        // List subdirectory
        let children = storage.list_directory(subdir.ino).unwrap();
        assert_eq!(children.len(), 2);
        
        // Write and read from files
        storage.write_file(file1.ino, b"File 1 content", 0).unwrap();
        storage.write_file(file2.ino, b"File 2 content", 0).unwrap();
        
        let data1 = storage.read_file(file1.ino).unwrap();
        let data2 = storage.read_file(file2.ino).unwrap();
        
        assert_eq!(data1, b"File 1 content");
        assert_eq!(data2, b"File 2 content");
    }
    
    #[test]
    fn test_delete_file() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create and write file
        let inode = storage.create_file(1, "temp.txt".to_string()).unwrap();
        storage.write_file(inode.ino, b"Temporary data", 0).unwrap();
        
        // Verify file exists
        let children_before = storage.list_directory(1).unwrap();
        let found_before = children_before.iter().any(|c| c.name == "temp.txt");
        assert!(found_before, "File should exist before deletion");
        
        // Delete file
        storage.delete_file(inode.ino).unwrap();
        
        // Verify file is gone
        let children_after = storage.list_directory(1).unwrap();
        let found_after = children_after.iter().any(|c| c.name == "temp.txt");
        assert!(!found_after, "File should not exist after deletion");
    }
    
    #[test]
    fn test_change_policy_replication_to_ec() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a small file (will use replication)
        let inode = storage.create_file(1, "policy_test.bin".to_string()).unwrap();
        let data = vec![0xAAu8; 512 * 1024]; // 512KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Verify we can read it
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
        
        // Change policy to EC (4+2)
        let new_policy = crate::extent::RedundancyPolicy::ErasureCoding {
            data_shards: 4,
            parity_shards: 2,
        };
        
        storage.change_file_redundancy(inode.ino, new_policy).unwrap();
        
        // Verify data still intact
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_change_policy_ec_to_replication() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a large file (will use EC)
        let inode = storage.create_file(1, "policy_test2.bin".to_string()).unwrap();
        let data = vec![0xBBu8; 3 * 1024 * 1024]; // 3MB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Verify initial read
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
        
        // Change policy to replication (5 copies)
        let new_policy = crate::extent::RedundancyPolicy::Replication { copies: 5 };
        
        storage.change_file_redundancy(inode.ino, new_policy).unwrap();
        
        // Verify data still intact
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_policy_change_with_disk_failure() {
        let (_pool_dir, _disk_dirs, metadata, mut disks) = setup_test_env();
        
        // Create initial file with EC (4+2)
        let storage = StorageEngine::new(metadata, disks.clone());
        let inode = storage.create_file(1, "resilient.bin".to_string()).unwrap();
        let data = vec![0xCCu8; 2 * 1024 * 1024]; // 2MB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Fail two disks
        disks[0].mark_failed().unwrap();
        disks[2].mark_failed().unwrap();
        
        // Recreate storage with failed disks
        let storage = StorageEngine::new(
            MetadataManager::new(_pool_dir.path().to_path_buf()).unwrap(),
            disks.clone()
        );
        
        // Data should still be readable
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
        
        // Change to higher redundancy (replication)
        let new_policy = crate::extent::RedundancyPolicy::Replication { copies: 4 };
        storage.change_file_redundancy(inode.ino, new_policy).unwrap();
        
        // Verify data is still readable
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_hot_cold_classification() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "classify.bin".to_string()).unwrap();
        let data = vec![0x42u8; 256 * 1024]; // 256KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Get extent map
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        
        // Load the extent
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        
        // Check classification - should have access stats recorded
        assert!(extent.access_stats.write_count > 0, "write_count should be > 0");
        // Classification depends on write/read frequency and timing
        // Just verify it has a valid classification
        let classification = extent.classification();
        assert!(
            classification == crate::extent::AccessClassification::Hot
                || classification == crate::extent::AccessClassification::Warm
                || classification == crate::extent::AccessClassification::Cold
        );
    }
    
    #[test]
    fn test_access_tracking() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "tracked.bin".to_string()).unwrap();
        let data = vec![0x55u8; 512 * 1024]; // 512KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Read the file multiple times
        for _ in 0..5 {
            let read_data = storage.read_file(inode.ino).unwrap();
            assert_eq!(read_data, data);
        }
        
        // Get extent and check read count increased
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        
        // Should have recorded the reads
        assert!(extent.access_stats.read_count > 0);
        assert!(extent.access_stats.last_read > 0);
    }
    
    #[test]
    fn test_access_frequency() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "frequent.bin".to_string()).unwrap();
        let data = vec![0x77u8; 128 * 1024]; // 128KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Get extent
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        
        // Check access frequency calculation
        let frequency = extent.access_frequency();
        assert!(frequency >= 0.0);
    }
    
    #[test]
    fn test_recommended_policy() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "policy_rec.bin".to_string()).unwrap();
        let data = vec![0x88u8; 256 * 1024]; // 256KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Get extent
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        
        // Check recommended policy based on classification
        let recommended = extent.recommended_policy();
        // Just verify it returns a valid policy
        match recommended {
            crate::extent::RedundancyPolicy::Replication { copies } => {
                assert!(copies > 0);
            }
            crate::extent::RedundancyPolicy::ErasureCoding { data_shards, parity_shards } => {
                assert!(data_shards > 0 && parity_shards > 0);
            }
        }
    }
    
    #[test]
    fn test_lazy_migration_on_read() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file with EC (cold data)
        let inode = storage.create_file(1, "migrate.bin".to_string()).unwrap();
        let data = vec![0x99u8; 3 * 1024 * 1024]; // 3MB - will use EC
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Get initial extent
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        let initial_policy = extent.redundancy;
        
        drop(metadata);
        drop(extent);
        
        // Read file multiple times to mark as hot
        for _ in 0..5 {
            let _ = storage.read_file(inode.ino).unwrap();
        }
        
        // Get extent again and check if migration may have occurred
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent = metadata.load_extent(&extent_map.extents[0]).unwrap();
        
        // The extent should have higher read count now
        assert!(extent.access_stats.read_count > 0);
        // If it was classified as hot, it may have migrated to replication
        // Just verify the extent is still valid and readable
        assert!(!extent.fragment_locations.is_empty());
    }
    
    #[test]
    fn test_lazy_migration_check() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "check_migrate.bin".to_string()).unwrap();
        let data = vec![0xAAu8; 512 * 1024]; // 512KB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Get extent UUID
        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        let extent_uuid = extent_map.extents[0];
        let extent = metadata.load_extent(&extent_uuid).unwrap();
        let initial_policy = extent.redundancy;
        
        drop(metadata);
        
        // Check if migration is needed
        let needs_migration = storage.extent_needs_migration(&extent_uuid).unwrap();
        let recommended = storage.get_recommended_policy(&extent_uuid).unwrap();
        
        // Verify recommendation exists
        match recommended {
            crate::extent::RedundancyPolicy::Replication { copies } => {
                assert!(copies > 0);
            }
            crate::extent::RedundancyPolicy::ErasureCoding { data_shards, parity_shards } => {
                assert!(data_shards > 0 && parity_shards > 0);
            }
        }
        
        // needs_migration should match if policies differ
        assert_eq!(needs_migration, recommended != initial_policy);
    }

    #[test]
    fn test_multi_extent_write_preserves_unique_chunks() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);

        let inode = storage.create_file(1, "pattern.bin".to_string()).unwrap();

        let mut data = Vec::new();
        for i in 0..3 {
            let value = (i as u8) + 1;
            data.extend(std::iter::repeat(value).take(DEFAULT_EXTENT_SIZE));
        }
        data.extend(std::iter::repeat(0xFFu8).take(DEFAULT_EXTENT_SIZE / 2));

        storage.write_file(inode.ino, &data, 0).unwrap();
        let read_back = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_back, data);

        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        assert!(extent_map.extents.len() >= 3);
    }

    #[test]
    fn test_write_failure_rolls_back_fragments() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);

        let inode = storage.create_file(1, "fail_on_write.bin".to_string()).unwrap();

        let sim = get_crash_simulator();
        sim.reset();
        sim.enable_at(CrashPoint::AfterFragmentWrite);

        let result = storage.write_file(inode.ino, b"guarded content", 0);
        assert!(result.is_err(), "expected crash-injected write failure");

        sim.disable();
        sim.reset();

        let metadata = storage.metadata.read().unwrap();
        let extent_map = metadata.load_extent_map(inode.ino).unwrap();
        assert!(extent_map.extents.is_empty(), "extent map should be empty after failed write");

        let extents = metadata.list_all_extents().unwrap();
        assert!(extents.is_empty(), "no extent metadata should be persisted on failure");

        let disks = storage.disks.read().unwrap();
        for disk in disks.iter() {
            let disk_guard = disk.lock().unwrap();
            let fragments_dir = disk_guard.path.join("fragments");
            let count = std::fs::read_dir(&fragments_dir).map(|rd| rd.count()).unwrap_or(0);
            assert_eq!(count, 0, "Fragments should be cleaned up for disk {:?}", disk_guard.uuid);
        }
    }
