// moved from src/security.rs
use super::*;

    #[test]
    fn test_validate_path() {
        assert!(SecurityValidator::validate_path(Path::new("/valid/path")).is_ok());
        
        // Path traversal should fail
        assert!(SecurityValidator::validate_path(Path::new("../etc/passwd")).is_err());
    }

    #[test]
    fn test_validate_size() {
        assert!(SecurityValidator::validate_size(1024, 2048).is_ok());
        
        // Size exceeding max should fail
        assert!(SecurityValidator::validate_size(3000, 2048).is_err());
        
        // Zero size should fail
        assert!(SecurityValidator::validate_size(0, 2048).is_err());
    }

    #[test]
    fn test_validate_redundancy() {
        assert!(SecurityValidator::validate_redundancy(4, 2).is_ok());
        
        // Invalid shards
        assert!(SecurityValidator::validate_redundancy(0, 2).is_err());
        assert!(SecurityValidator::validate_redundancy(300, 300).is_err());
    }

    #[test]
    fn test_validate_uuid() {
        let uuid = Uuid::new_v4();
        assert!(SecurityValidator::validate_uuid(uuid).is_ok());
        
        // Nil UUID should fail
        assert!(SecurityValidator::validate_uuid(Uuid::nil()).is_err());
    }

    #[test]
    fn test_validate_inode() {
        assert!(SecurityValidator::validate_inode(1).is_ok());
        assert!(SecurityValidator::validate_inode(1000).is_ok());
        
        // Inode 0 should fail
        assert!(SecurityValidator::validate_inode(0).is_err());
    }

    #[test]
    fn test_fuse_mount_policy() {
        let secure = FuseMountPolicy::secure();
        assert!(!secure.allow_other);
        
        let default = FuseMountPolicy::default();
        assert_eq!(default.entry_timeout, 60);
        
        let args = secure.to_fuse_args();
        assert!(args.iter().any(|a| a.contains("ro")));
    }

    #[test]
    fn test_audit_log() {
        let mut log = AuditLog::new(10);
        
        log.log(
            AuditEventType::AccessDenied,
            "Access denied to file".to_string(),
            AuditSeverity::Warning,
        );
        
        let events = log.get_events(AuditSeverity::Warning);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_capability_manager() {
        let mgr = CapabilityManager::for_fuse_mount();
        
        assert!(mgr.verify_capability(Capability::ReadFiles).is_ok());
        assert!(mgr.verify_capability(Capability::AdministerDisk).is_err());
    }

    #[test]
    fn test_validate_checksum() {
        let valid_checksum = [1u8; 32];
        assert!(SecurityValidator::validate_checksum(&valid_checksum).is_ok());
        
        let invalid_checksum = [0u8; 32];
        assert!(SecurityValidator::validate_checksum(&invalid_checksum).is_err());
    }

    #[test]
    fn test_validate_fragment_index() {
        assert!(SecurityValidator::validate_fragment_index(0, 10).is_ok());
        assert!(SecurityValidator::validate_fragment_index(9, 10).is_ok());
        
        // Index >= max should fail
        assert!(SecurityValidator::validate_fragment_index(10, 10).is_err());
        assert!(SecurityValidator::validate_fragment_index(100, 10).is_err());
    }
