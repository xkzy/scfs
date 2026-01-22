use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Metadata root with versioning for atomic commits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataRoot {
    /// Monotonically increasing version number
    pub version: u64,
    
    /// Timestamp of this version (seconds since epoch)
    pub timestamp: i64,
    
    /// Next available inode number
    pub next_ino: u64,
    
    /// Checksum of entire metadata state (BLAKE3)
    pub state_checksum: String,
    
    /// Number of inodes in this version
    pub inode_count: u64,
    
    /// Number of extents in this version
    pub extent_count: u64,
    
    /// Total data size (bytes)
    pub total_size: u64,
    
    /// State: "committed" | "pending"
    pub state: String,
}

impl MetadataRoot {
    pub fn new(next_ino: u64) -> Self {
        // Initial root needs a placeholder checksum
        let initial_checksum = blake3::hash(b"initial_root_v1").to_hex().to_string();
        
        MetadataRoot {
            version: 1,
            timestamp: chrono::Utc::now().timestamp(),
            next_ino,
            state_checksum: initial_checksum,
            inode_count: 1, // Root directory
            extent_count: 0,
            total_size: 0,
            state: "committed".to_string(),
        }
    }
    
    /// Create next version from current
    pub fn next_version(&self) -> Self {
        MetadataRoot {
            version: self.version + 1,
            timestamp: chrono::Utc::now().timestamp(),
            next_ino: self.next_ino,
            state_checksum: String::new(),
            inode_count: self.inode_count,
            extent_count: self.extent_count,
            total_size: self.total_size,
            state: "pending".to_string(),
        }
    }
    
    /// Mark this version as committed
    pub fn commit(&mut self, state_checksum: String) {
        self.state = "committed".to_string();
        self.state_checksum = state_checksum;
        self.timestamp = chrono::Utc::now().timestamp();
    }
    
    /// Check if this root is valid
    pub fn is_valid(&self) -> bool {
        self.version > 0 
            && self.state == "committed"
            && !self.state_checksum.is_empty()
            && self.next_ino >= 2  // At least root inode
    }
}

/// Transaction coordinator for atomic metadata updates
pub struct MetadataTransaction {
    pool_dir: PathBuf,
    current_root: MetadataRoot,
    pending_root: Option<MetadataRoot>,
    committed: bool,
}

impl MetadataTransaction {
    /// Begin a new transaction from current root
    pub fn begin(pool_dir: &Path, current_root: MetadataRoot) -> Self {
        let pending_root = current_root.next_version();
        
        MetadataTransaction {
            pool_dir: pool_dir.to_path_buf(),
            current_root,
            pending_root: Some(pending_root),
            committed: false,
        }
    }
    
    /// Get the pending root (mutable)
    pub fn pending_root_mut(&mut self) -> Result<&mut MetadataRoot> {
        self.pending_root.as_mut()
            .ok_or_else(|| anyhow!("No pending transaction"))
    }
    
    /// Commit the transaction
    pub fn commit(mut self, state_checksum: String) -> Result<MetadataRoot> {
        let mut pending = self.pending_root
            .take()
            .ok_or_else(|| anyhow!("No pending transaction"))?;
        
        // Mark as committed
        pending.commit(state_checksum);
        
        // Write to disk atomically
        self.write_root(&pending)?;
        
        // Fsync to ensure durability
        self.fsync_root_dir()?;
        
        self.committed = true;
        Ok(pending)
    }
    
    /// Abort the transaction (automatic on drop)
    pub fn abort(self) {
        // Just drop, pending root never written
    }
    
    /// Write root to disk atomically
    fn write_root(&self, root: &MetadataRoot) -> Result<()> {
        let root_dir = self.pool_dir.join("metadata").join("roots");
        fs::create_dir_all(&root_dir)?;
        
        // Write versioned root
        let root_path = root_dir.join(format!("root.{}", root.version));
        let temp_path = root_path.with_extension("tmp");
        
        let contents = serde_json::to_string_pretty(root)?;
        fs::write(&temp_path, &contents)?;
        
        // Atomic rename
        fs::rename(&temp_path, &root_path)?;
        
        // Update "current" symlink
        let current_link = root_dir.join("current");
        let temp_link = root_dir.join("current.tmp");
        
        // Create symlink to latest root
        let target = format!("root.{}", root.version);
        
        // Remove old temp link if exists
        let _ = fs::remove_file(&temp_link);
        
        // Create new symlink
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &temp_link)?;
        
        // Atomic rename of symlink
        fs::rename(&temp_link, &current_link)?;
        
        Ok(())
    }
    
    /// Fsync the root directory for durability
    fn fsync_root_dir(&self) -> Result<()> {
        let root_dir = self.pool_dir.join("metadata").join("roots");
        
        // Open directory and fsync (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let dir = fs::File::open(&root_dir)?;
            unsafe {
                libc::fsync(dir.as_raw_fd());
            }
        }
        
        Ok(())
    }
}

impl Drop for MetadataTransaction {
    fn drop(&mut self) {
        if !self.committed && self.pending_root.is_some() {
            log::warn!("Transaction dropped without commit - aborting");
            // Pending root automatically discarded
        }
    }
}

/// Metadata root manager
pub struct MetadataRootManager {
    pool_dir: PathBuf,
    current_root: Arc<Mutex<MetadataRoot>>,
}

impl MetadataRootManager {
    /// Create new root manager
    pub fn new(pool_dir: PathBuf) -> Result<Self> {
        let root_dir = pool_dir.join("metadata").join("roots");
        fs::create_dir_all(&root_dir)?;
        
        // Load or create initial root
        let current_root = Self::load_latest_root(&root_dir)
            .unwrap_or_else(|| {
                log::info!("Creating initial metadata root");
                MetadataRoot::new(2) // Start at ino 2 (1 is root dir)
            });
        
        // Verify root is valid
        if !current_root.is_valid() {
            return Err(anyhow!("Invalid metadata root: {:?}", current_root));
        }
        
        log::info!("Loaded metadata root version {}", current_root.version);
        
        Ok(MetadataRootManager {
            pool_dir,
            current_root: Arc::new(Mutex::new(current_root)),
        })
    }
    
    /// Load the latest valid root
    fn load_latest_root(root_dir: &Path) -> Option<MetadataRoot> {
        // Try to read "current" symlink first
        let current_link = root_dir.join("current");
        if current_link.exists() {
            if let Ok(root) = Self::read_root_file(&current_link) {
                if root.is_valid() {
                    return Some(root);
                }
            }
        }
        
        // Fall back to scanning for highest version
        let mut roots = Vec::new();
        
        if let Ok(entries) = fs::read_dir(root_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("root.") && !name.ends_with(".tmp") {
                        if let Ok(root) = Self::read_root_file(&entry.path()) {
                            if root.is_valid() {
                                roots.push(root);
                            }
                        }
                    }
                }
            }
        }
        
        // Return highest version
        roots.into_iter()
            .max_by_key(|r| r.version)
    }
    
    /// Read a root file
    fn read_root_file(path: &Path) -> Result<MetadataRoot> {
        let contents = fs::read_to_string(path)?;
        let root: MetadataRoot = serde_json::from_str(&contents)?;
        Ok(root)
    }
    
    /// Get current root (snapshot)
    pub fn current_root(&self) -> MetadataRoot {
        self.current_root.lock().unwrap().clone()
    }
    
    /// Begin a new transaction
    pub fn begin_transaction(&self) -> MetadataTransaction {
        let current = self.current_root.lock().unwrap().clone();
        MetadataTransaction::begin(&self.pool_dir, current)
    }
    
    /// Commit a transaction and update current root
    pub fn commit_transaction(&self, tx: MetadataTransaction, state_checksum: String) -> Result<()> {
        let new_root = tx.commit(state_checksum)?;
        
        // Update current root
        let mut current = self.current_root.lock().unwrap();
        *current = new_root;
        
        log::info!("Committed metadata root version {}", current.version);
        
        Ok(())
    }
    
    /// Clean up old root versions (keep last N)
    pub fn gc_old_roots(&self, keep_count: usize) -> Result<usize> {
        let root_dir = self.pool_dir.join("metadata").join("roots");
        let mut roots = Vec::new();
        
        // Scan for all roots
        if let Ok(entries) = fs::read_dir(&root_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("root.") && !name.ends_with(".tmp") {
                        if let Ok(root) = Self::read_root_file(&entry.path()) {
                            roots.push((root.version, entry.path()));
                        }
                    }
                }
            }
        }
        
        // Sort by version
        roots.sort_by_key(|(v, _)| *v);
        
        // Keep newest N, delete the rest
        let delete_count = roots.len().saturating_sub(keep_count);
        let mut deleted = 0;
        
        for (_, path) in roots.iter().take(delete_count) {
            if fs::remove_file(path).is_ok() {
                deleted += 1;
                log::debug!("Deleted old root: {:?}", path);
            }
        }
        
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
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
}
