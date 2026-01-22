//! OS-Specific Filesystem Mounting
//!
//! This module provides platform-specific filesystem mounting implementations,
//! separating the mounting logic from the core storage engine. This allows the
//! storage engine to remain completely OS-agnostic while still supporting
//! native mounting on each platform.
//!
//! ## Supported Platforms
//!
//! - **Linux**: FUSE (Filesystem in Userspace) via `fuser` crate
//! - **macOS**: macFUSE or FUSE-T via `fuser` crate
//! - **Windows**: WinFsp (Windows Filesystem Proxy) integration (planned)
//!
//! ## Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────┐
//! │              Application                       │
//! └───────────────────┬────────────────────────────┘
//!                     │
//!                     v
//! ┌────────────────────────────────────────────────┐
//! │         mount::mount_filesystem()              │  ← This module
//! └───────────────────┬────────────────────────────┘
//!                     │
//!       ┌─────────────┼─────────────┐
//!       v             v             v
//! ┌──────────┐  ┌──────────┐  ┌──────────┐
//! │  Linux   │  │  macOS   │  │ Windows  │
//! │  FUSE    │  │ macFUSE  │  │  WinFsp  │
//! └──────────┘  └──────────┘  └──────────┘
//!       │             │             │
//!       └─────────────┼─────────────┘
//!                     │
//!                     v
//! ┌────────────────────────────────────────────────┐
//! │        FilesystemInterface Trait               │
//! │      (Platform-independent storage)            │
//! └────────────────────────────────────────────────┘
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use std::path::Path;
//! use dynamicfs::mount;
//! use dynamicfs::storage::StorageEngine;
//!
//! // Create your storage backend
//! let storage = Box::new(StorageEngine::new(metadata, disks));
//!
//! // Mount it at a platform-specific location
//! let mountpoint = Path::new("/mnt/myfs");
//! mount::mount_filesystem(storage, mountpoint)?;
//! ```

use anyhow::Result;
use std::path::Path;
use crate::fs_interface::FilesystemInterface;

/// Mount the filesystem at the specified mountpoint
///
/// This function handles OS-specific mounting logic, automatically selecting
/// the appropriate mounting mechanism for the current platform.
///
/// # Arguments
///
/// * `fs` - The filesystem implementation to mount (must be Send + Sync)
/// * `mountpoint` - Path where the filesystem should be mounted
///
/// # Platform-Specific Behavior
///
/// ## Linux
/// Uses FUSE with standard mount options:
/// - FSName: "dynamicfs"
/// - AllowOther: Allows other users to access
/// - DefaultPermissions: Enable kernel permission checking
///
/// ## macOS
/// Uses macFUSE/FUSE-T with macOS-optimized options:
/// - All Linux options, plus:
/// - AutoUnmount: Automatically unmount on process exit
/// - AllowRoot: Allow root access
///
/// ## Windows
/// Uses WinFsp integration (requires WinFsp to be installed)
///
/// # Errors
///
/// Returns an error if:
/// - The mountpoint doesn't exist or isn't accessible
/// - The platform-specific mounting mechanism isn't available
/// - Permission is denied
/// - The mountpoint is already in use
///
/// # Examples
///
/// ```rust,ignore
/// use std::path::Path;
/// use dynamicfs::mount;
///
/// let mountpoint = Path::new("/mnt/myfs");
/// mount::mount_filesystem(storage, mountpoint)?;
/// ```
pub fn mount_filesystem(
    fs: Box<dyn FilesystemInterface + Send + Sync>,
    mountpoint: &Path,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        mount_linux(fs, mountpoint)
    }

    #[cfg(target_os = "macos")]
    {
        mount_macos(fs, mountpoint)
    }

    #[cfg(target_os = "windows")]
    {
        mount_windows(fs, mountpoint)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(anyhow::anyhow!("Unsupported operating system for filesystem mounting"))
    }
}

/// Mount filesystem on Linux using FUSE
#[cfg(target_os = "linux")]
fn mount_linux(
    fs: Box<dyn FilesystemInterface + Send + Sync>,
    mountpoint: &Path,
) -> Result<()> {
    use crate::fuse_impl::DynamicFS;
    use fuser::MountOption;

    let options = vec![
        MountOption::FSName("dynamicfs".to_string()),
        MountOption::AllowOther,
        MountOption::DefaultPermissions,
    ];

    let dynamic_fs = DynamicFS::new(fs);

    fuser::mount2(dynamic_fs, mountpoint, &options)
        .map_err(|e| anyhow::anyhow!("Failed to mount filesystem: {}", e))
}

/// Mount filesystem on macOS using macFUSE/FUSE-T
#[cfg(target_os = "macos")]
fn mount_macos(
    fs: Box<dyn FilesystemInterface + Send + Sync>,
    mountpoint: &Path,
) -> Result<()> {
    use crate::fuse_impl::DynamicFS;
    use fuser::MountOption;

    let options = vec![
        MountOption::FSName("dynamicfs".to_string()),
        MountOption::AllowOther,
        MountOption::DefaultPermissions,
        // macOS-specific options for better integration
        MountOption::AutoUnmount,
        MountOption::AllowRoot,
    ];

    let dynamic_fs = DynamicFS::new(fs);

    fuser::mount2(dynamic_fs, mountpoint, &options)
        .map_err(|e| anyhow::anyhow!("Failed to mount filesystem: {}", e))
}

/// Mount filesystem on Windows using WinFsp
#[cfg(target_os = "windows")]
fn mount_windows(
    fs: Box<dyn FilesystemInterface + Send + Sync>,
    mountpoint: &Path,
) -> Result<()> {
    use crate::windows_fs::WindowsFS;

    let windows_fs = WindowsFS::new(fs);
    windows_fs.mount(mountpoint)
}

/// Unmount the filesystem from the specified mountpoint
///
/// Cleanly unmounts a previously mounted filesystem.
///
/// # Arguments
///
/// * `mountpoint` - Path where the filesystem is mounted
///
/// # Errors
///
/// Returns an error if:
/// - The mountpoint is not mounted
/// - Permission is denied
/// - The filesystem is busy (files are open)
///
/// # Examples
///
/// ```rust,ignore
/// use std::path::Path;
/// use dynamicfs::mount;
///
/// let mountpoint = Path::new("/mnt/myfs");
/// mount::unmount_filesystem(mountpoint)?;
/// ```
pub fn unmount_filesystem(mountpoint: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        unmount_linux(mountpoint)
    }

    #[cfg(target_os = "macos")]
    {
        unmount_macos(mountpoint)
    }

    #[cfg(target_os = "windows")]
    {
        unmount_windows(mountpoint)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(anyhow::anyhow!("Unsupported operating system for filesystem unmounting"))
    }
}

/// Unmount filesystem on Linux
#[cfg(target_os = "linux")]
fn unmount_linux(mountpoint: &Path) -> Result<()> {
    use std::process::Command;

    let output = Command::new("fusermount")
        .args(&["-u", &mountpoint.to_string_lossy()])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run fusermount: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(anyhow::anyhow!("fusermount failed: {}", stderr))
    }
}

/// Unmount filesystem on macOS
#[cfg(target_os = "macos")]
fn unmount_macos(mountpoint: &Path) -> Result<()> {
    use std::process::Command;

    let output = Command::new("umount")
        .arg(&mountpoint.to_string_lossy())
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run umount: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(anyhow::anyhow!("umount failed: {}", stderr))
    }
}

/// Unmount filesystem on Windows
#[cfg(target_os = "windows")]
fn unmount_windows(_mountpoint: &Path) -> Result<()> {
    // For unmounting on Windows, we would need to keep track of mounted filesystems
    // and call the appropriate WinFsp unmount function.
    // This is a placeholder that would be implemented with full WinFsp support.
    Err(anyhow::anyhow!("Windows unmounting requires full WinFsp integration"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests are mostly for documentation and type checking.
    // Actual mounting tests require root/admin privileges and are better
    // suited for integration tests.

    #[test]
    fn test_mount_api_exists() {
        // This test just verifies the API exists and can be referenced
        // Actual functionality testing requires elevated privileges
        
        // The function signature should accept the correct types
        fn _type_check() {
            use crate::storage::StorageEngine;
            use crate::metadata::MetadataManager;
            use crate::disk::Disk;
            use tempfile::TempDir;
            
            let _f: fn(Box<dyn FilesystemInterface + Send + Sync>, &Path) -> Result<()> 
                = mount_filesystem;
        }
    }

    #[test]
    fn test_unmount_api_exists() {
        // Verify the unmount API signature
        let _f: fn(&Path) -> Result<()> = unmount_filesystem;
    }
}
