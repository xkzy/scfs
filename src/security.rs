use uuid::Uuid;
use std::path::Path;

/// Security validator for filesystem operations
pub struct SecurityValidator;

impl SecurityValidator {
    /// Validate path for directory traversal attacks
    pub fn validate_path(path: &Path) -> anyhow::Result<()> {
        // Convert to string for validation
        let path_str = path.to_string_lossy();

        // Check for null bytes
        if path_str.contains('\0') {
            return Err(anyhow::anyhow!("Path contains null bytes"));
        }

        // Check for suspicious patterns
        if path_str.contains("..") && path.is_relative() {
            return Err(anyhow::anyhow!("Path traversal detected"));
        }

        Ok(())
    }

    /// Validate file size bounds
    pub fn validate_size(size: u64, max_size: u64) -> anyhow::Result<()> {
        if size > max_size {
            return Err(anyhow::anyhow!(
                "Size {} exceeds maximum {}",
                size,
                max_size
            ));
        }

        if size == 0 {
            return Err(anyhow::anyhow!("Size cannot be zero"));
        }

        Ok(())
    }

    /// Validate extent count
    pub fn validate_extent_count(count: usize, max_extents: usize) -> anyhow::Result<()> {
        if count > max_extents {
            return Err(anyhow::anyhow!(
                "Extent count {} exceeds maximum {}",
                count,
                max_extents
            ));
        }

        Ok(())
    }

    /// Validate redundancy parameters
    pub fn validate_redundancy(data_shards: usize, parity_shards: usize) -> anyhow::Result<()> {
        if data_shards == 0 || data_shards > 256 {
            return Err(anyhow::anyhow!("Invalid data shards: {}", data_shards));
        }

        if parity_shards == 0 || parity_shards > 256 {
            return Err(anyhow::anyhow!("Invalid parity shards: {}", parity_shards));
        }

        if data_shards + parity_shards > 256 {
            return Err(anyhow::anyhow!(
                "Total shards {} exceeds maximum",
                data_shards + parity_shards
            ));
        }

        Ok(())
    }

    /// Validate UUID format
    pub fn validate_uuid(uuid: Uuid) -> anyhow::Result<()> {
        // UUIDs are already validated by the UUID type, so just check not nil
        if uuid == Uuid::nil() {
            return Err(anyhow::anyhow!("UUID cannot be nil"));
        }

        Ok(())
    }

    /// Validate inode number
    pub fn validate_inode(ino: u64) -> anyhow::Result<()> {
        if ino == 0 {
            return Err(anyhow::anyhow!("Invalid inode: 0"));
        }

        if ino > u64::MAX / 2 {
            return Err(anyhow::anyhow!("Inode number too large"));
        }

        Ok(())
    }

    /// Validate checksum
    pub fn validate_checksum(checksum: &[u8; 32]) -> anyhow::Result<()> {
        // Check not all zeros (which would indicate missing checksum)
        if checksum.iter().all(|&b| b == 0) {
            return Err(anyhow::anyhow!("Checksum is all zeros"));
        }

        Ok(())
    }

    /// Validate fragment index
    pub fn validate_fragment_index(index: usize, max_fragments: usize) -> anyhow::Result<()> {
        if index >= max_fragments {
            return Err(anyhow::anyhow!(
                "Fragment index {} out of range: max {}",
                index,
                max_fragments
            ));
        }

        Ok(())
    }
}

/// Security policies for FUSE mount
pub struct FuseMountPolicy {
    /// Allow_other for multi-user access
    pub allow_other: bool,
    /// Default permissions mode
    pub entry_timeout: u64,
    pub attr_timeout: u64,
    /// Maximum file size
    pub max_file_size: u64,
    /// Maximum number of open files
    pub max_open_files: usize,
}

impl FuseMountPolicy {
    pub fn secure() -> Self {
        FuseMountPolicy {
            allow_other: false,
            entry_timeout: 0,    // No cache for security
            attr_timeout: 0,     // No cache for security
            max_file_size: 1024 * 1024 * 1024, // 1GB
            max_open_files: 1024,
        }
    }

    pub fn default() -> Self {
        FuseMountPolicy {
            allow_other: false,
            entry_timeout: 60,
            attr_timeout: 60,
            max_file_size: 10 * 1024 * 1024 * 1024, // 10GB
            max_open_files: 4096,
        }
    }

    pub fn to_fuse_args(&self) -> Vec<String> {
        let mut args = vec![];

        if self.allow_other {
            args.push("allow_other".to_string());
        }

        args.push(format!("entry_timeout={}", self.entry_timeout));
        args.push(format!("attr_timeout={}", self.attr_timeout));

        // Read-only by default for safety
        args.push("ro".to_string());

        args
    }
}

/// Security audit logger
pub struct AuditLog {
    events: Vec<AuditEvent>,
    max_events: usize,
}

#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub timestamp: i64,
    pub event_type: AuditEventType,
    pub description: String,
    pub severity: AuditSeverity,
}

#[derive(Debug, Clone, Copy)]
pub enum AuditEventType {
    AccessDenied,
    InvalidInput,
    BoundsExceeded,
    PermissionDenied,
    OperationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuditSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

impl AuditLog {
    pub fn new(max_events: usize) -> Self {
        AuditLog {
            events: Vec::new(),
            max_events,
        }
    }

    pub fn log(&mut self, event_type: AuditEventType, description: String, severity: AuditSeverity) {
        let event = AuditEvent {
            timestamp: chrono::Utc::now().timestamp(),
            event_type,
            description,
            severity,
        };

        self.events.push(event);

        // Keep only recent events
        if self.events.len() > self.max_events {
            self.events.remove(0);
        }
    }

    pub fn get_events(&self, min_severity: AuditSeverity) -> Vec<AuditEvent> {
        self.events
            .iter()
            .filter(|e| e.severity >= min_severity)
            .cloned()
            .collect()
    }

    pub fn critical_events(&self) -> Vec<AuditEvent> {
        self.get_events(AuditSeverity::Error)
    }
}

/// Capability manager for privilege dropping
pub struct CapabilityManager {
    required_caps: Vec<Capability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    ReadFiles,
    WriteFiles,
    ManageMetadata,
    MountFS,
    AdministerDisk,
}

impl CapabilityManager {
    pub fn for_fuse_mount() -> Self {
        CapabilityManager {
            required_caps: vec![
                Capability::ReadFiles,
                Capability::WriteFiles,
                Capability::ManageMetadata,
            ],
        }
    }

    pub fn has_capability(&self, cap: Capability) -> bool {
        self.required_caps.contains(&cap)
    }

    pub fn verify_capability(&self, cap: Capability) -> anyhow::Result<()> {
        if self.has_capability(cap) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Missing capability: {:?}", cap))
        }
    }
}

#[cfg(test)]
mod tests {
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
}
