// moved from src/backup_evolution.rs
use super::*;

    #[test]
    fn test_create_full_backup() {
        let mut manager = BackupManager::new();
        let extents = vec![
            (Uuid::new_v4(), 1024u64),
            (Uuid::new_v4(), 2048u64),
        ];

        let backup = manager.create_full_backup(&extents);
        
        assert_eq!(backup.backup_type, BackupType::Full);
        assert_eq!(backup.total_size, 3072);
        assert_eq!(backup.extents.len(), 2);
    }

    #[test]
    fn test_create_incremental_backup() {
        let mut manager = BackupManager::new();
        let base_extents = vec![(Uuid::new_v4(), 1024u64)];
        let base_backup = manager.create_full_backup(&base_extents);

        let changes = vec![
            (Uuid::new_v4(), ChangeType::Created, 512u64),
            (Uuid::new_v4(), ChangeType::Modified, 256u64),
        ];

        let incremental = manager.create_incremental_backup(
            base_backup.id,
            &changes,
        ).unwrap();

        assert_eq!(incremental.backup_type, BackupType::Incremental);
        assert_eq!(incremental.base_backup, Some(base_backup.id));
    }

    #[test]
    fn test_backup_completion() {
        let mut manager = BackupManager::new();
        let extents = vec![(Uuid::new_v4(), 1024u64)];
        let backup = manager.create_full_backup(&extents);

        manager.complete_backup(backup.id).unwrap();
        
        let completed = manager.get_backup(backup.id).unwrap();
        assert_eq!(completed.status, BackupStatus::Completed);
        assert!(completed.completed_at.is_some());
    }

    #[test]
    fn test_format_version() {
        let v1 = FormatVersion::current();
        let v2 = FormatVersion {
            major: 1,
            minor: 1,
            patch: 0,
            features: vec![],
        };

        assert!(v2.is_compatible(&v1)); // 1.1 is compatible with 1.0
        
        let v3 = FormatVersion {
            major: 2,
            minor: 0,
            patch: 0,
            features: vec![],
        };
        
        assert!(!v3.is_compatible(&v1)); // 2.0 is NOT compatible with 1.0
    }

    #[test]
    fn test_upgrade_operation() {
        let from = FormatVersion::current();
        let to = FormatVersion {
            major: 1,
            minor: 1,
            patch: 0,
            features: vec![],
        };

        let mut upgrade = UpgradeOperation::new(from, to);
        assert_eq!(upgrade.status, UpgradeStatus::NotStarted);

        upgrade.mark_in_progress();
        assert_eq!(upgrade.status, UpgradeStatus::InProgress);

        upgrade.mark_completed();
        assert_eq!(upgrade.status, UpgradeStatus::Completed);
        assert_eq!(upgrade.progress_percent, 100);
    }

    #[test]
    fn test_backup_list() {
        let mut manager = BackupManager::new();
        
        let backup1 = manager.create_full_backup(&[(Uuid::new_v4(), 1024)]);
        let backup2 = manager.create_full_backup(&[(Uuid::new_v4(), 2048)]);

        let backups = manager.list_backups();
        assert_eq!(backups.len(), 2);
        
        // Should be sorted by recency
        assert_eq!(backups[0].id, backup2.id);
        assert_eq!(backups[1].id, backup1.id);
    }
