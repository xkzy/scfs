//! Cross-Platform Filesystem Interface
//!
//! This module defines the core abstraction layer that enables pluggable storage backends
//! and cross-platform compatibility for the DynamicFS filesystem.
//!
//! ## Overview
//!
//! The `FilesystemInterface` trait provides a platform-independent API for filesystem
//! operations, allowing the storage engine to be completely decoupled from OS-specific
//! mounting mechanisms (FUSE on Linux/macOS, WinFsp on Windows).
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │   OS-Specific Mounting Layer            │
//! │  (FUSE/WinFsp/macFUSE)                  │
//! ├─────────────────────────────────────────┤
//! │   FilesystemInterface Trait             │  ← This module
//! │  (Platform-independent API)             │
//! ├─────────────────────────────────────────┤
//! │   Storage Engine Implementation         │
//! │  (Pure Rust, no OS dependencies)        │
//! └─────────────────────────────────────────┘
//! ```
//!
//! ## Design Principles
//!
//! 1. **OS Independence**: No direct dependencies on OS-specific APIs or types
//! 2. **Pluggability**: Multiple storage backends can implement this trait
//! 3. **Simplicity**: Clean, minimal API surface area
//! 4. **Safety**: All operations return Result for proper error handling
//!
//! ## Usage
//!
//! Implement the `FilesystemInterface` trait to create a storage backend:
//!
//! ```rust,ignore
//! use anyhow::Result;
//! use dynamicfs::fs_interface::{FilesystemInterface, FilesystemStats};
//! use dynamicfs::metadata::Inode;
//!
//! struct MyStorageBackend {
//!     // Your storage implementation
//! }
//!
//! impl FilesystemInterface for MyStorageBackend {
//!     fn read_file(&self, ino: u64) -> Result<Vec<u8>> {
//!         // Implementation
//!         todo!()
//!     }
//!
//!     fn write_file(&self, ino: u64, data: &[u8], offset: u64) -> Result<()> {
//!         // Implementation
//!         todo!()
//!     }
//!
//!     // ... implement other methods
//! }
//! ```

use anyhow::Result;

/// Cross-platform filesystem interface trait
///
/// This trait abstracts filesystem operations to enable pluggable storage backends
/// and cross-platform compatibility. Any storage engine implementing this trait
/// can be mounted using platform-specific mounting mechanisms.
///
/// # Thread Safety
///
/// Implementations must be thread-safe (Send + Sync) to support concurrent
/// access from multiple threads in the FUSE/WinFsp layer.
///
/// # Error Handling
///
/// All methods return `Result<T>` to allow proper error propagation. Implementations
/// should return descriptive errors using `anyhow::Error`.
pub trait FilesystemInterface {
    /// Read entire file content
    ///
    /// # Arguments
    ///
    /// * `ino` - Inode number of the file to read
    ///
    /// # Returns
    ///
    /// The complete file content as a byte vector
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The inode does not exist
    /// - The inode is not a regular file
    /// - There are I/O errors reading the data
    fn read_file(&self, ino: u64) -> Result<Vec<u8>>;

    /// Write data to file at specified offset
    ///
    /// # Arguments
    ///
    /// * `ino` - Inode number of the file to write to
    /// * `data` - Data to write
    /// * `offset` - Byte offset in the file where writing should begin
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The inode does not exist
    /// - The inode is not a regular file
    /// - There are I/O errors writing the data
    /// - The offset is invalid
    fn write_file(&self, ino: u64, data: &[u8], offset: u64) -> Result<()>;

    /// Create a new file
    ///
    /// # Arguments
    ///
    /// * `parent_ino` - Inode number of the parent directory
    /// * `name` - Name of the new file
    ///
    /// # Returns
    ///
    /// The newly created file's inode metadata
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The parent inode does not exist
    /// - The parent is not a directory
    /// - A file with the same name already exists in the parent
    /// - There are I/O errors creating the file
    fn create_file(&self, parent_ino: u64, name: String) -> Result<crate::metadata::Inode>;

    /// Create a new directory
    ///
    /// # Arguments
    ///
    /// * `parent_ino` - Inode number of the parent directory
    /// * `name` - Name of the new directory
    ///
    /// # Returns
    ///
    /// The newly created directory's inode metadata
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The parent inode does not exist
    /// - The parent is not a directory
    /// - A directory with the same name already exists in the parent
    /// - There are I/O errors creating the directory
    fn create_dir(&self, parent_ino: u64, name: String) -> Result<crate::metadata::Inode>;

    /// Delete a file
    ///
    /// # Arguments
    ///
    /// * `ino` - Inode number of the file to delete
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The inode does not exist
    /// - The inode is not a regular file
    /// - There are I/O errors deleting the file
    fn delete_file(&self, ino: u64) -> Result<()>;

    /// Delete a directory
    ///
    /// # Arguments
    ///
    /// * `ino` - Inode number of the directory to delete
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The inode does not exist
    /// - The inode is not a directory
    /// - The directory is not empty
    /// - There are I/O errors deleting the directory
    fn delete_dir(&self, ino: u64) -> Result<()>;

    /// Get inode information
    ///
    /// # Arguments
    ///
    /// * `ino` - Inode number to look up
    ///
    /// # Returns
    ///
    /// The inode's metadata
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The inode does not exist
    /// - There are I/O errors reading the metadata
    fn get_inode(&self, ino: u64) -> Result<crate::metadata::Inode>;

    /// List directory contents
    ///
    /// # Arguments
    ///
    /// * `parent_ino` - Inode number of the directory to list
    ///
    /// # Returns
    ///
    /// A vector of inode metadata for all entries in the directory
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The inode does not exist
    /// - The inode is not a directory
    /// - There are I/O errors reading the directory
    fn list_directory(&self, parent_ino: u64) -> Result<Vec<crate::metadata::Inode>>;

    /// Find child by name in directory
    ///
    /// # Arguments
    ///
    /// * `parent_ino` - Inode number of the directory to search
    /// * `name` - Name of the child to find
    ///
    /// # Returns
    ///
    /// * `Some(inode)` if a child with the given name exists
    /// * `None` if no child with the given name exists
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The parent inode does not exist
    /// - The parent is not a directory
    /// - There are I/O errors searching the directory
    fn find_child(&self, parent_ino: u64, name: &str) -> Result<Option<crate::metadata::Inode>>;

    /// Update inode metadata
    ///
    /// # Arguments
    ///
    /// * `inode` - The inode with updated metadata
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The inode does not exist
    /// - There are I/O errors writing the metadata
    fn update_inode(&self, inode: &crate::metadata::Inode) -> Result<()>;

    /// Get filesystem statistics
    ///
    /// # Returns
    ///
    /// Current filesystem statistics including file counts, space usage, etc.
    ///
    /// # Errors
    ///
    /// Returns an error if there are I/O errors collecting the statistics
    fn stat(&self) -> Result<FilesystemStats>;
}

/// Filesystem statistics
///
/// Provides an overview of the filesystem's current state including
/// file/directory counts and space utilization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemStats {
    /// Total number of regular files
    pub total_files: u64,
    
    /// Total number of directories
    pub total_dirs: u64,
    
    /// Total size of all files in bytes
    pub total_size: u64,
    
    /// Used storage space in bytes
    pub used_space: u64,
    
    /// Free storage space in bytes
    pub free_space: u64,
}

impl FilesystemStats {
    /// Calculate the percentage of space used
    ///
    /// # Returns
    ///
    /// Percentage of total capacity that is used (0.0 to 100.0)
    pub fn usage_percentage(&self) -> f64 {
        let total = self.used_space + self.free_space;
        if total == 0 {
            0.0
        } else {
            (self.used_space as f64 / total as f64) * 100.0
        }
    }

    /// Get total capacity in bytes
    ///
    /// # Returns
    ///
    /// Total storage capacity (used + free)
    pub fn total_capacity(&self) -> u64 {
        self.used_space + self.free_space
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filesystem_stats_usage_percentage() {
        let stats = FilesystemStats {
            total_files: 10,
            total_dirs: 5,
            total_size: 1000,
            used_space: 250,
            free_space: 750,
        };

        assert_eq!(stats.usage_percentage(), 25.0);
        assert_eq!(stats.total_capacity(), 1000);
    }

    #[test]
    fn test_filesystem_stats_empty() {
        let stats = FilesystemStats {
            total_files: 0,
            total_dirs: 1,
            total_size: 0,
            used_space: 0,
            free_space: 1000,
        };

        assert_eq!(stats.usage_percentage(), 0.0);
        assert_eq!(stats.total_capacity(), 1000);
    }

    #[test]
    fn test_filesystem_stats_full() {
        let stats = FilesystemStats {
            total_files: 100,
            total_dirs: 10,
            total_size: 10000,
            used_space: 1000,
            free_space: 0,
        };

        assert_eq!(stats.usage_percentage(), 100.0);
        assert_eq!(stats.total_capacity(), 1000);
    }
}
