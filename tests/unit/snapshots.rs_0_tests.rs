// moved from src/snapshots.rs
use super::*;
    use crate::extent::{RedundancyPolicy, FragmentLocation};

    fn create_test_extent(size: usize) -> Extent {
        use crate::extent::AccessStats;
        Extent {
            uuid: Uuid::new_v4(),
            size,
            checksum: [0; 32],
            redundancy: RedundancyPolicy::Replication { copies: 3 },
            fragment_locations: vec![
                FragmentLocation {
                    disk_uuid: Uuid::new_v4(),
                    fragment_index: 0,
                    on_device: None,
                },
            ],
            previous_policy: None,
            policy_transitions: Vec::new(),
            last_policy_change: None,
            access_stats: AccessStats {
                read_count: 0,
                write_count: 0,
                last_read: 0,
                last_write: 0,
                created_at: Utc::now().timestamp(),
                classification: crate::extent::AccessClassification::Cold,
                hmm_classifier: None,
            },
            rebuild_in_progress: false,
            rebuild_progress: None,
            generation: 0,
        }
    }

    #[test]
    fn test_create_full_snapshot() {
        let mut manager = SnapshotManager::new();
        let extent = create_test_extent(1024);
        let extents = vec![extent.clone()];
        
        let snapshot = manager.create_full_snapshot(
            "snap1".to_string(),
            Uuid::new_v4(),
            &extents,
            "Test snapshot".to_string(),
        );

        assert_eq!(snapshot.name, "snap1");
        assert_eq!(snapshot.parent_uuid, None);
        assert_eq!(snapshot.file_count, 1);
        assert_eq!(snapshot.total_size, 1024);
        assert_eq!(manager.extent_refcount(&extent.uuid), 1);
    }

    #[test]
    fn test_snapshot_indexing() {
        let mut manager = SnapshotManager::new();
        let extent = create_test_extent(1024);
        
        let snapshot = manager.create_full_snapshot(
            "snap1".to_string(),
            Uuid::new_v4(),
            &[extent],
            "".to_string(),
        );

        assert!(manager.get_snapshot("snap1").is_some());
        assert_eq!(manager.get_snapshot("snap1").unwrap().uuid, snapshot.uuid);
        assert!(manager.get_snapshot("nonexistent").is_none());
    }

    #[test]
    fn test_create_incremental_snapshot() {
        let mut manager = SnapshotManager::new();
        let parent_extent = create_test_extent(1024);
        let new_extent = create_test_extent(512);
        
        let parent = manager.create_full_snapshot(
            "parent".to_string(),
            Uuid::new_v4(),
            &[parent_extent.clone()],
            "".to_string(),
        );

        let incremental = manager.create_incremental_snapshot(
            "child".to_string(),
            parent.uuid,
            Uuid::new_v4(),
            &[new_extent.clone()],
            &[],
            &[],
            "".to_string(),
        ).unwrap();

        assert_eq!(incremental.parent_uuid, Some(parent.uuid));
        assert_eq!(manager.extent_refcount(&new_extent.uuid), 1);
    }

    #[test]
    fn test_restore_operation() {
        let mut restore = RestoreOperation::new(
            Uuid::new_v4(),
            "/mnt/restore".to_string(),
            1000,
        );

        assert_eq!(restore.status, RestoreStatus::InProgress);
        assert_eq!(restore.progress_percent(), 0.0);

        restore.bytes_restored = 500;
        assert_eq!(restore.progress_percent(), 50.0);

        restore.mark_completed();
        assert_eq!(restore.status, RestoreStatus::Completed);
    }

    #[test]
    fn test_cow_savings_estimation() {
        let mut manager = SnapshotManager::new();
        let extent1 = create_test_extent(1024);
        let extent2 = create_test_extent(512);
        
        manager.create_full_snapshot(
            "snap1".to_string(),
            Uuid::new_v4(),
            &[extent1.clone(), extent2.clone()],
            "".to_string(),
        );

        // Create second snapshot with same extents
        manager.create_full_snapshot(
            "snap2".to_string(),
            Uuid::new_v4(),
            &[extent1.clone()],
            "".to_string(),
        );

        // extent1 has refcount 2, so we save 1x its size
        assert!(manager.estimate_cow_savings() > 0);
    }
