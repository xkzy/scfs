use dynamicfs::test_utils::setup_test_env;
use dynamicfs::disk::Disk;

#[test]
fn test_disk_drain_triggers_migration_and_removal() {
    // Setup environment
    let (pool_dir, _disk_dirs, metadata, mut disks) = setup_test_env();

    // Mark first disk as draining (before creating storage so state is captured)
    disks[0].mark_draining().unwrap();

    // Create storage with current disk states
    let storage = dynamicfs::storage::StorageEngine::new(metadata, disks.clone());

    // Write a file so fragments are placed
    let inode = storage.create_file(1, "drain_test.bin".to_string()).unwrap();
    let data = vec![0x55u8; 1024];
    storage.write_file(inode.ino, &data, 0).unwrap();

    // Perform mount-time rebuild which should migrate fragments off draining disk
    let res = storage.perform_mount_rebuild();
    assert!(res.is_ok(), "perform_mount_rebuild should succeed");

    // After rebuild, ensure no fragment references point to the drained disk
    let metadata_mgr = storage.metadata();
    let metadata = metadata_mgr.read().unwrap();
    let extents = metadata.list_all_extents().unwrap();
    for extent in extents {
        for loc in extent.fragment_locations {
            assert_ne!(loc.disk_uuid, disks[0].uuid, "Fragments should be migrated off drained disk");
        }
    }
}
