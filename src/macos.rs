#![cfg(target_os = "macos")]

//! macOS-specific filesystem features for SCFS
//!
//! This module provides macOS-specific functionality including:
//! - Resource fork support
//! - Finder metadata
//! - Spotlight indexing
//! - Time Machine compatibility

use std::ffi::OsStr;

/// macOS extended attribute namespaces
pub mod xattr_names {
    pub const RESOURCE_FORK: &str = "com.apple.ResourceFork";
    pub const FINDER_INFO: &str = "com.apple.FinderInfo";
    pub const METADATA: &str = "com.apple.metadata";
    pub const TIME_MACHINE: &str = "com.apple.TimeMachine";
    pub const QUARANTINE: &str = "com.apple.quarantine";
}

/// Check if an xattr name is macOS-specific
pub fn is_macos_xattr(name: &str) -> bool {
    name.starts_with("com.apple.")
}

/// Validate macOS xattr name and value
pub fn validate_macos_xattr(name: &str, value: &[u8]) -> Result<(), &'static str> {
    match name {
        xattr_names::RESOURCE_FORK => {
            // Resource forks can be large
            if value.len() > 16 * 1024 * 1024 { // 16MB limit
                return Err("Resource fork too large");
            }
        }
        xattr_names::FINDER_INFO => {
            // Finder info is exactly 32 bytes
            if value.len() != 32 {
                return Err("Finder info must be exactly 32 bytes");
            }
        }
        xattr_names::METADATA => {
            // Spotlight metadata can be up to 1MB
            if value.len() > 1024 * 1024 {
                return Err("Spotlight metadata too large");
            }
        }
        _ => {
            // Other macOS xattrs have reasonable size limits
            if value.len() > 64 * 1024 {
                return Err("macOS xattr value too large");
            }
        }
    }
    Ok(())
}

/// Get default macOS xattrs for a new file
pub fn default_macos_xattrs(file_type: &str) -> Vec<(String, Vec<u8>)> {
    let mut xattrs = Vec::new();

    // Add Finder info for all files
    let finder_info = vec![0u8; 32]; // Default empty Finder info
    xattrs.push((xattr_names::FINDER_INFO.to_string(), finder_info));

    // Add type-specific xattrs
    match file_type {
        "directory" => {
            // Directories might have special Finder info
        }
        _ => {
            // Regular files
        }
    }

    xattrs
}

/// Handle macOS-specific file operations
pub struct MacOSHandler;

impl MacOSHandler {
    pub fn new() -> Self {
        MacOSHandler
    }

    /// Process macOS-specific xattr operations
    pub fn handle_xattr(&self, name: &str, value: Option<&[u8]>) -> Result<Option<Vec<u8>>, &'static str> {
        if !is_macos_xattr(name) {
            return Ok(None); // Not a macOS xattr, let normal handling proceed
        }

        match name {
            xattr_names::RESOURCE_FORK => {
                // Resource fork handling
                if let Some(val) = value {
                    validate_macos_xattr(name, val)?;
                }
                Ok(None) // Let normal xattr storage handle it
            }
            xattr_names::FINDER_INFO => {
                if let Some(val) = value {
                    validate_macos_xattr(name, val)?;
                }
                Ok(None)
            }
            xattr_names::METADATA => {
                if let Some(val) = value {
                    validate_macos_xattr(name, val)?;
                }
                Ok(None)
            }
            xattr_names::TIME_MACHINE => {
                // Time Machine xattrs might need special handling
                Ok(None)
            }
            _ => {
                // Other macOS xattrs
                if let Some(val) = value {
                    validate_macos_xattr(name, val)?;
                }
                Ok(None)
            }
        }
    }
}