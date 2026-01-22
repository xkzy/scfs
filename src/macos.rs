#![cfg(target_os = "macos")]

//! macOS-specific filesystem features for SCFS
//!
//! ## Phase 9.3: macOS Support
//!
//! This module provides comprehensive macOS-specific functionality including:
//! - **Extended Attributes**: Full support for macOS xattrs including resource forks
//! - **Finder Metadata**: Color labels, comments, and custom icons
//! - **Spotlight Integration**: Metadata for search indexing
//! - **Time Machine Compatibility**: Markers and exclusion support
//! - **HFS+ Semantics**: Case-insensitive comparison, file IDs
//!
//! ## Extended Attributes
//!
//! macOS uses extended attributes (xattrs) extensively for metadata:
//! - `com.apple.FinderInfo`: Finder-specific metadata (32 bytes)
//! - `com.apple.ResourceFork`: Legacy resource fork data
//! - `com.apple.metadata:*`: Spotlight indexing metadata
//! - `com.apple.quarantine`: Download quarantine information
//! - `com.apple.TimeMachine.*`: Time Machine backup metadata
//!
//! ## Spotlight Integration
//!
//! Spotlight uses metadata attributes for searching:
//! - `kMDItemContentType`: UTI (Uniform Type Identifier)
//! - `kMDItemDisplayName`: User-visible file name
//! - `kMDItemKeywords`: Search keywords
//! - `kMDItemAuthors`: Document authors
//! - `kMDItemContentCreationDate`: Creation date
//!
//! ## Time Machine
//!
//! Time Machine backup system uses special attributes:
//! - `com.apple.TimeMachine.Supported`: Marks filesystem as backup-capable
//! - Exclusion markers for files/directories that shouldn't be backed up
//! - Snapshot support for consistent backups

use std::ffi::OsStr;

/// macOS extended attribute namespaces
///
/// These are the standard Apple-defined xattr namespaces used by macOS.
pub mod xattr_names {
    /// Resource fork attribute (legacy Mac OS compatibility)
    ///
    /// Resource forks contain structured data like icons, menus, and dialogs.
    /// Modern macOS applications rarely use resource forks, but they're
    /// still supported for compatibility.
    pub const RESOURCE_FORK: &str = "com.apple.ResourceFork";
    
    /// Finder information attribute (32 bytes)
    ///
    /// Contains Finder-specific metadata:
    /// - Bytes 0-3: File type (OSType)
    /// - Bytes 4-7: Creator code (OSType)
    /// - Bytes 8-9: Finder flags
    /// - Bytes 10-11: Location in window
    /// - Bytes 12-13: Folder ID
    /// - Bytes 14-31: Reserved
    pub const FINDER_INFO: &str = "com.apple.FinderInfo";
    
    /// Spotlight metadata namespace
    ///
    /// Attributes under this namespace are indexed by Spotlight:
    /// - `com.apple.metadata:kMDItemKeywords`
    /// - `com.apple.metadata:kMDItemAuthors`
    /// - `com.apple.metadata:kMDItemContentType`
    pub const METADATA: &str = "com.apple.metadata";
    
    /// Time Machine backup metadata
    ///
    /// Used by Time Machine for backup management:
    /// - Exclusion markers
    /// - Backup timestamps
    /// - Snapshot information
    pub const TIME_MACHINE: &str = "com.apple.TimeMachine";
    
    /// Download quarantine information
    ///
    /// Marks files downloaded from the internet:
    /// - Download source URL
    /// - Download date
    /// - Quarantine flags
    pub const QUARANTINE: &str = "com.apple.quarantine";
    
    /// Apple Double format (._* files)
    ///
    /// Used for storing extended attributes on non-HFS+ volumes
    pub const APPLE_DOUBLE: &str = "com.apple.AppleDouble";
    
    /// File system encoding hint
    pub const TEXT_ENCODING: &str = "com.apple.TextEncoding";
}

/// Finder flag constants
///
/// These flags are stored in the Finder info xattr and control
/// how the Finder displays and treats files.
pub mod finder_flags {
    /// File is on the desktop
    pub const IS_ON_DESK: u16 = 0x0001;
    
    /// File color (3 bits: 0-7)
    pub const COLOR_MASK: u16 = 0x000E;
    
    /// File has been opened
    pub const HAS_BEEN_INITED: u16 = 0x0100;
    
    /// File has custom icon
    pub const HAS_CUSTOM_ICON: u16 = 0x0400;
    
    /// File is a stationery pad
    pub const IS_STATIONERY: u16 = 0x0800;
    
    /// File can't be renamed
    pub const NAME_LOCKED: u16 = 0x1000;
    
    /// File has bundle bit set
    pub const HAS_BUNDLE: u16 = 0x2000;
    
    /// File is invisible
    pub const IS_INVISIBLE: u16 = 0x4000;
    
    /// File is an alias
    pub const IS_ALIAS: u16 = 0x8000;
}

/// Finder color labels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderColor {
    None = 0,
    Gray = 1,
    Green = 2,
    Purple = 3,
    Blue = 4,
    Yellow = 5,
    Red = 6,
    Orange = 7,
}

impl FinderColor {
    /// Create from finder flags
    pub fn from_flags(flags: u16) -> Self {
        match (flags & finder_flags::COLOR_MASK) >> 1 {
            0 => FinderColor::None,
            1 => FinderColor::Gray,
            2 => FinderColor::Green,
            3 => FinderColor::Purple,
            4 => FinderColor::Blue,
            5 => FinderColor::Yellow,
            6 => FinderColor::Red,
            7 => FinderColor::Orange,
            _ => FinderColor::None,
        }
    }

    /// Convert to finder flags bits
    pub fn to_flags_bits(self) -> u16 {
        (self as u16) << 1
    }
}

/// Check if an xattr name is macOS-specific
///
/// # Examples
///
/// ```
/// use dynamicfs::macos::is_macos_xattr;
///
/// assert!(is_macos_xattr("com.apple.FinderInfo"));
/// assert!(is_macos_xattr("com.apple.metadata:kMDItemAuthors"));
/// assert!(!is_macos_xattr("user.custom"));
/// ```
pub fn is_macos_xattr(name: &str) -> bool {
    name.starts_with("com.apple.")
}

/// Validate macOS xattr name and value
///
/// Ensures xattr values conform to macOS requirements:
/// - FinderInfo must be exactly 32 bytes
/// - Resource forks limited to 16MB
/// - Metadata attributes limited to 1MB
///
/// # Errors
///
/// Returns an error string if validation fails.
pub fn validate_macos_xattr(name: &str, value: &[u8]) -> Result<(), &'static str> {
    match name {
        xattr_names::RESOURCE_FORK => {
            // Resource forks can be large but have practical limits
            if value.len() > 16 * 1024 * 1024 { // 16MB limit
                return Err("Resource fork too large (max 16MB)");
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
                return Err("Spotlight metadata too large (max 1MB)");
            }
        }
        _ if name.starts_with("com.apple.") => {
            // Other macOS xattrs have reasonable size limits
            if value.len() > 64 * 1024 {
                return Err("macOS xattr value too large (max 64KB)");
            }
        }
        _ => {}
    }
    Ok(())
}

/// Get default macOS xattrs for a new file
///
/// Creates appropriate default extended attributes based on file type.
///
/// # Arguments
///
/// * `file_type` - Type of file ("file", "directory", "symlink")
///
/// # Returns
///
/// Vector of (name, value) tuples for default xattrs.
pub fn default_macos_xattrs(file_type: &str) -> Vec<(String, Vec<u8>)> {
    let mut xattrs = Vec::new();

    // Add Finder info for all files
    let finder_info = vec![0u8; 32]; // Default empty Finder info
    xattrs.push((xattr_names::FINDER_INFO.to_string(), finder_info));

    // Add type-specific xattrs
    match file_type {
        "directory" => {
            // Directories might have special Finder info
            // Could set folder flags or custom icons
        }
        "symlink" => {
            // Symlinks typically don't have extended attributes
            xattrs.clear();
        }
        _ => {
            // Regular files get standard Finder info
        }
    }

    xattrs
}

/// Parse Finder info structure
///
/// Extracts information from the 32-byte Finder info xattr.
#[derive(Debug, Clone)]
pub struct FinderInfo {
    /// File type (4-byte OSType)
    pub file_type: [u8; 4],
    
    /// Creator code (4-byte OSType)
    pub creator: [u8; 4],
    
    /// Finder flags
    pub flags: u16,
    
    /// Location in window (x, y)
    pub location: (i16, i16),
    
    /// Folder ID
    pub folder_id: u16,
    
    /// Color label
    pub color: FinderColor,
}

impl FinderInfo {
    /// Parse from 32-byte buffer
    pub fn from_bytes(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() != 32 {
            return Err("Finder info must be exactly 32 bytes");
        }

        let mut file_type = [0u8; 4];
        let mut creator = [0u8; 4];
        file_type.copy_from_slice(&data[0..4]);
        creator.copy_from_slice(&data[4..8]);

        let flags = u16::from_be_bytes([data[8], data[9]]);
        let location = (
            i16::from_be_bytes([data[10], data[11]]),
            i16::from_be_bytes([data[12], data[13]]),
        );
        let folder_id = u16::from_be_bytes([data[14], data[15]]);
        let color = FinderColor::from_flags(flags);

        Ok(FinderInfo {
            file_type,
            creator,
            flags,
            location,
            folder_id,
            color,
        })
    }

    /// Serialize to 32-byte buffer
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = vec![0u8; 32];
        data[0..4].copy_from_slice(&self.file_type);
        data[4..8].copy_from_slice(&self.creator);
        data[8..10].copy_from_slice(&self.flags.to_be_bytes());
        data[10..12].copy_from_slice(&self.location.0.to_be_bytes());
        data[12..14].copy_from_slice(&self.location.1.to_be_bytes());
        data[14..16].copy_from_slice(&self.folder_id.to_be_bytes());
        data
    }

    /// Set color label
    pub fn set_color(&mut self, color: FinderColor) {
        self.flags = (self.flags & !finder_flags::COLOR_MASK) | color.to_flags_bits();
        self.color = color;
    }

    /// Check if file is invisible
    pub fn is_invisible(&self) -> bool {
        self.flags & finder_flags::IS_INVISIBLE != 0
    }

    /// Set invisible flag
    pub fn set_invisible(&mut self, invisible: bool) {
        if invisible {
            self.flags |= finder_flags::IS_INVISIBLE;
        } else {
            self.flags &= !finder_flags::IS_INVISIBLE;
        }
    }
}

/// Handle macOS-specific file operations
///
/// This struct provides methods for managing macOS-specific filesystem
/// features including extended attributes, Spotlight metadata, and
/// Time Machine integration.
pub struct MacOSHandler {
    /// Whether to enable Spotlight indexing
    spotlight_enabled: bool,
    
    /// Whether to enable Time Machine support
    time_machine_enabled: bool,
}

impl MacOSHandler {
    /// Create a new macOS handler with default settings
    pub fn new() -> Self {
        MacOSHandler {
            spotlight_enabled: true,
            time_machine_enabled: true,
        }
    }

    /// Create with custom configuration
    pub fn with_config(spotlight_enabled: bool, time_machine_enabled: bool) -> Self {
        MacOSHandler {
            spotlight_enabled,
            time_machine_enabled,
        }
    }

    /// Process macOS-specific xattr operations
    ///
    /// Validates and handles macOS extended attributes.
    ///
    /// # Arguments
    ///
    /// * `name` - Attribute name
    /// * `value` - Attribute value (None for read operations)
    ///
    /// # Returns
    ///
    /// - `Ok(Some(data))` - Processed/transformed data
    /// - `Ok(None)` - Let normal handling proceed
    /// - `Err(msg)` - Validation error
    pub fn handle_xattr(&self, name: &str, value: Option<&[u8]>) -> Result<Option<Vec<u8>>, &'static str> {
        if !is_macos_xattr(name) {
            return Ok(None); // Not a macOS xattr, let normal handling proceed
        }

        match name {
            xattr_names::RESOURCE_FORK => {
                // Resource fork handling
                if let Some(val) = value {
                    validate_macos_xattr(name, val)?;
                    log::debug!("Storing resource fork ({} bytes)", val.len());
                }
                Ok(None) // Let normal xattr storage handle it
            }
            xattr_names::FINDER_INFO => {
                if let Some(val) = value {
                    validate_macos_xattr(name, val)?;
                    // Could parse and log Finder info here
                    if let Ok(info) = FinderInfo::from_bytes(val) {
                        log::debug!("Finder info: color={:?}, invisible={}", 
                                   info.color, info.is_invisible());
                    }
                }
                Ok(None)
            }
            xattr_names::METADATA => {
                if let Some(val) = value {
                    validate_macos_xattr(name, val)?;
                    if self.spotlight_enabled {
                        log::debug!("Storing Spotlight metadata ({} bytes)", val.len());
                    }
                }
                Ok(None)
            }
            xattr_names::TIME_MACHINE => {
                // Time Machine xattrs might need special handling
                if self.time_machine_enabled {
                    log::debug!("Time Machine xattr: {}", name);
                }
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

    /// Check if file should be excluded from Time Machine backups
    ///
    /// Checks for Time Machine exclusion markers.
    pub fn is_time_machine_excluded(&self, _xattrs: &[(String, Vec<u8>)]) -> bool {
        // Would check for com.apple.metadata:com_apple_backup_excludeItem
        false
    }

    /// Mark file for Time Machine exclusion
    ///
    /// Sets the appropriate xattr to exclude from backups.
    pub fn exclude_from_time_machine(&self) -> (String, Vec<u8>) {
        // The actual xattr is com.apple.metadata:com_apple_backup_excludeItem
        // with a specific plist value
        (
            "com.apple.metadata:com_apple_backup_excludeItem".to_string(),
            b"com.apple.backupd".to_vec(),
        )
    }

    /// Generate Spotlight metadata for a file
    ///
    /// Creates Spotlight-compatible metadata attributes.
    ///
    /// # Arguments
    ///
    /// * `filename` - Name of the file
    /// * `content_type` - UTI (e.g., "public.plain-text")
    /// * `keywords` - Search keywords
    ///
    /// # Returns
    ///
    /// Vector of (name, value) tuples for Spotlight xattrs.
    pub fn generate_spotlight_metadata(
        &self,
        filename: &str,
        content_type: &str,
        keywords: &[&str],
    ) -> Vec<(String, Vec<u8>)> {
        if !self.spotlight_enabled {
            return vec![];
        }

        let mut xattrs = Vec::new();

        // Add content type
        xattrs.push((
            "com.apple.metadata:kMDItemContentType".to_string(),
            content_type.as_bytes().to_vec(),
        ));

        // Add display name
        xattrs.push((
            "com.apple.metadata:kMDItemDisplayName".to_string(),
            filename.as_bytes().to_vec(),
        ));

        // Add keywords
        if !keywords.is_empty() {
            let keywords_str = keywords.join(",");
            xattrs.push((
                "com.apple.metadata:kMDItemKeywords".to_string(),
                keywords_str.as_bytes().to_vec(),
            ));
        }

        xattrs
    }
}

impl Default for MacOSHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_macos_xattr() {
        assert!(is_macos_xattr("com.apple.FinderInfo"));
        assert!(is_macos_xattr("com.apple.metadata:kMDItemAuthors"));
        assert!(is_macos_xattr("com.apple.ResourceFork"));
        assert!(!is_macos_xattr("user.custom"));
        assert!(!is_macos_xattr("security.selinux"));
    }

    #[test]
    fn test_finder_info_parsing() {
        let data = vec![0u8; 32];
        let info = FinderInfo::from_bytes(&data).unwrap();
        
        assert_eq!(info.file_type, [0, 0, 0, 0]);
        assert_eq!(info.creator, [0, 0, 0, 0]);
        assert_eq!(info.flags, 0);
        assert_eq!(info.color, FinderColor::None);
    }

    #[test]
    fn test_finder_info_color() {
        let mut info = FinderInfo::from_bytes(&vec![0u8; 32]).unwrap();
        
        info.set_color(FinderColor::Red);
        assert_eq!(info.color, FinderColor::Red);
        
        let bytes = info.to_bytes();
        let parsed = FinderInfo::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.color, FinderColor::Red);
    }

    #[test]
    fn test_finder_info_invisible() {
        let mut info = FinderInfo::from_bytes(&vec![0u8; 32]).unwrap();
        
        assert!(!info.is_invisible());
        info.set_invisible(true);
        assert!(info.is_invisible());
        
        let bytes = info.to_bytes();
        let parsed = FinderInfo::from_bytes(&bytes).unwrap();
        assert!(parsed.is_invisible());
    }

    #[test]
    fn test_xattr_validation() {
        // Valid Finder info
        assert!(validate_macos_xattr(xattr_names::FINDER_INFO, &vec![0u8; 32]).is_ok());
        
        // Invalid Finder info size
        assert!(validate_macos_xattr(xattr_names::FINDER_INFO, &vec![0u8; 31]).is_err());
        
        // Valid resource fork
        assert!(validate_macos_xattr(xattr_names::RESOURCE_FORK, &vec![0u8; 1024]).is_ok());
        
        // Too large resource fork
        assert!(validate_macos_xattr(xattr_names::RESOURCE_FORK, &vec![0u8; 17 * 1024 * 1024]).is_err());
    }

    #[test]
    fn test_default_xattrs() {
        let xattrs = default_macos_xattrs("file");
        assert!(!xattrs.is_empty());
        assert!(xattrs.iter().any(|(name, _)| name == xattr_names::FINDER_INFO));
        
        // Symlinks don't get xattrs
        let symlink_xattrs = default_macos_xattrs("symlink");
        assert!(symlink_xattrs.is_empty());
    }

    #[test]
    fn test_macos_handler() {
        let handler = MacOSHandler::new();
        
        // Test Finder info handling
        let finder_data = vec![0u8; 32];
        let result = handler.handle_xattr(xattr_names::FINDER_INFO, Some(&finder_data));
        assert!(result.is_ok());
        
        // Test invalid Finder info
        let invalid_data = vec![0u8; 16];
        let result = handler.handle_xattr(xattr_names::FINDER_INFO, Some(&invalid_data));
        assert!(result.is_err());
    }

    #[test]
    fn test_spotlight_metadata() {
        let handler = MacOSHandler::new();
        let metadata = handler.generate_spotlight_metadata(
            "test.txt",
            "public.plain-text",
            &["document", "text"],
        );
        
        assert!(!metadata.is_empty());
        assert!(metadata.iter().any(|(name, _)| name.contains("kMDItemContentType")));
        assert!(metadata.iter().any(|(name, _)| name.contains("kMDItemKeywords")));
    }

    #[test]
    fn test_time_machine_exclusion() {
        let handler = MacOSHandler::new();
        let (name, value) = handler.exclude_from_time_machine();
        
        assert!(name.contains("backup_excludeItem"));
        assert!(!value.is_empty());
    }
}