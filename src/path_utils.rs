//! Platform-Independent Path Utilities
//!
//! This module provides path manipulation utilities that work consistently across
//! all supported platforms (Linux, macOS, Windows). It handles platform-specific
//! differences like path separators and drive letters transparently.
//!
//! ## Design Philosophy
//!
//! Paths are normalized to use forward slashes (`/`) internally, regardless of
//! the host platform. This provides a consistent internal representation while
//! allowing easy conversion to/from platform-specific paths at the boundaries.
//!
//! ## Examples
//!
//! ```rust
//! use dynamicfs::path_utils;
//! use std::path::Path;
//!
//! // Normalize paths from different platforms
//! let unix_path = Path::new("/home/user/file.txt");
//! let windows_path = Path::new("C:\\Users\\user\\file.txt");
//!
//! // Both normalize to forward slashes
//! let norm1 = path_utils::normalize_path(unix_path);
//! // On Windows: "/C/Users/user/file.txt"
//! // On Unix: "/home/user/file.txt"
//!
//! // Join paths in a platform-independent way
//! let joined = path_utils::join_path("/base", "subdir");
//! assert_eq!(joined, "/base/subdir");
//!
//! // Extract components
//! assert_eq!(path_utils::parent_path("/a/b/c"), Some("/a/b"));
//! assert_eq!(path_utils::file_name("/a/b/c"), Some("c"));
//! ```

use std::path::{Path, PathBuf};

/// Normalize path separators to forward slashes for internal representation
///
/// Handles Windows drive letters and UNC paths by converting them to a
/// normalized form that uses forward slashes.
///
/// # Platform-Specific Behavior
///
/// ## Windows
/// - `C:\path\to\file` → `/C/path/to/file`
/// - `\\server\share\file` → `//server/share/file`
///
/// ## Unix (Linux/macOS)
/// - `/path/to/file` → `/path/to/file` (unchanged)
///
/// # Examples
///
/// ```rust
/// use std::path::Path;
/// use dynamicfs::path_utils;
///
/// let path = Path::new("a\\b\\c");
/// let normalized = path_utils::normalize_path(path);
/// assert_eq!(normalized, "a/b/c");
/// ```
pub fn normalize_path(path: &Path) -> String {
    let path_str = path.to_string_lossy();

    // Handle Windows drive letters (C:\ -> /C/)
    #[cfg(target_os = "windows")]
    {
        if path_str.len() >= 3 
            && path_str.chars().nth(1) == Some(':') 
            && path_str.chars().nth(2) == Some('\\') 
        {
            let drive = path_str.chars().next().unwrap().to_ascii_uppercase();
            let rest = &path_str[3..];
            return format!("/{}{}", drive, rest.replace('\\', "/"));
        }
        
        // Handle UNC paths (\\server\share -> //server/share)
        if path_str.starts_with("\\\\") {
            return path_str.replace('\\', "/");
        }
    }

    // For Unix or relative Windows paths, just normalize separators
    path_str.replace('\\', "/")
}

/// Convert normalized path back to platform-specific path
///
/// Reverses the normalization performed by `normalize_path`, converting
/// internal forward-slash paths back to platform-native format.
///
/// # Platform-Specific Behavior
///
/// ## Windows
/// - `/C/path/to/file` → `C:\path\to\file`
/// - `//server/share/file` → `\\server\share\file`
///
/// ## Unix (Linux/macOS)
/// - `/path/to/file` → `/path/to/file` (unchanged)
///
/// # Examples
///
/// ```rust
/// use dynamicfs::path_utils;
///
/// let denormalized = path_utils::denormalize_path("a/b/c");
/// // On Windows: PathBuf::from("a\\b\\c")
/// // On Unix: PathBuf::from("a/b/c")
/// ```
pub fn denormalize_path(normalized: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        // Handle normalized Windows drive paths (/C/path -> C:\path)
        if normalized.len() >= 3 
            && normalized.starts_with('/') 
            && normalized.chars().nth(2) == Some('/') 
        {
            let drive = normalized.chars().nth(1).unwrap();
            let rest = &normalized[3..];
            return PathBuf::from(format!("{}:\\{}", drive, rest.replace('/', "\\")));
        }
        
        // Handle UNC paths (//server/share -> \\server\share)
        if normalized.starts_with("//") {
            return PathBuf::from(normalized.replace('/', "\\"));
        }
        
        // Relative paths
        return PathBuf::from(normalized.replace('/', "\\"));
    }

    #[cfg(not(target_os = "windows"))]
    {
        PathBuf::from(normalized)
    }
}

/// Join path components in a platform-independent way
///
/// Joins a base path with a component using forward slashes, ensuring
/// no double slashes are created.
///
/// # Examples
///
/// ```rust
/// use dynamicfs::path_utils;
///
/// assert_eq!(path_utils::join_path("/base", "file"), "/base/file");
/// assert_eq!(path_utils::join_path("/base/", "file"), "/base/file");
/// ```
pub fn join_path(base: &str, component: &str) -> String {
    if base.ends_with('/') {
        format!("{}{}", base, component)
    } else {
        format!("{}/{}", base, component)
    }
}

/// Get parent path
///
/// Returns the parent directory of a path, or `None` if the path
/// has no parent (e.g., root directory or single component).
///
/// # Examples
///
/// ```rust
/// use dynamicfs::path_utils;
///
/// assert_eq!(path_utils::parent_path("/a/b/c"), Some("/a/b"));
/// assert_eq!(path_utils::parent_path("/a"), Some("/"));
/// assert_eq!(path_utils::parent_path("/"), Some("/"));
/// assert_eq!(path_utils::parent_path("file"), None);
/// ```
pub fn parent_path(path: &str) -> Option<&str> {
    if let Some(last_slash) = path.rfind('/') {
        if last_slash == 0 {
            Some("/") // Root directory
        } else {
            Some(&path[..last_slash])
        }
    } else {
        None // No parent (relative single component)
    }
}

/// Get filename from path
///
/// Extracts the final component (filename or directory name) from a path.
///
/// # Examples
///
/// ```rust
/// use dynamicfs::path_utils;
///
/// assert_eq!(path_utils::file_name("/a/b/c"), Some("c"));
/// assert_eq!(path_utils::file_name("/a/b/"), Some("b"));
/// assert_eq!(path_utils::file_name("file.txt"), Some("file.txt"));
/// assert_eq!(path_utils::file_name("/"), None);
/// ```
pub fn file_name(path: &str) -> Option<&str> {
    path.rsplit('/').find(|s| !s.is_empty())
}

/// Check if path is absolute
///
/// Determines whether a path is absolute or relative, handling
/// platform-specific differences.
///
/// # Platform-Specific Behavior
///
/// ## Windows
/// - Absolute if starts with `/` (normalized), `C:\` (drive letter), or `\\` (UNC)
///
/// ## Unix (Linux/macOS)
/// - Absolute if starts with `/`
///
/// # Examples
///
/// ```rust
/// use dynamicfs::path_utils;
///
/// assert!(path_utils::is_absolute("/absolute/path"));
/// assert!(!path_utils::is_absolute("relative/path"));
/// ```
pub fn is_absolute(path: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        // Windows: C:\ or /C/ (normalized) or UNC \\server\share
        path.starts_with('/') 
            || path.starts_with('\\')
            || (path.len() >= 3 && path.chars().nth(1) == Some(':'))
    }
    
    #[cfg(not(target_os = "windows"))]
    {
        path.starts_with('/')
    }
}

/// Get path separator for current platform
///
/// Returns the platform-specific path separator as a string slice.
///
/// # Returns
///
/// - Windows: `"\"`
/// - Unix: `"/"`
pub fn separator() -> &'static str {
    std::path::MAIN_SEPARATOR_STR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        let path = Path::new("a\\b\\c");
        let normalized = normalize_path(path);
        assert_eq!(normalized, "a/b/c");

        let path2 = Path::new("a/b/c");
        let normalized2 = normalize_path(path2);
        assert_eq!(normalized2, "a/b/c");
    }

    #[test]
    fn test_denormalize_path() {
        let denormalized = denormalize_path("a/b/c");
        
        #[cfg(target_os = "windows")]
        assert_eq!(denormalized, PathBuf::from("a\\b\\c"));
        
        #[cfg(not(target_os = "windows"))]
        assert_eq!(denormalized, PathBuf::from("a/b/c"));
    }

    #[test]
    fn test_join_path() {
        assert_eq!(join_path("a/b", "c"), "a/b/c");
        assert_eq!(join_path("a/b/", "c"), "a/b/c");
        assert_eq!(join_path("/", "file"), "/file");
    }

    #[test]
    fn test_parent_path() {
        assert_eq!(parent_path("a/b/c"), Some("a/b"));
        assert_eq!(parent_path("a/b"), Some("a"));
        assert_eq!(parent_path("a"), None);
        assert_eq!(parent_path("/a/b"), Some("/a"));
        assert_eq!(parent_path("/a"), Some("/"));
        assert_eq!(parent_path("/"), Some("/"));
    }

    #[test]
    fn test_file_name() {
        assert_eq!(file_name("a/b/c"), Some("c"));
        assert_eq!(file_name("a/b/"), Some("b"));
        assert_eq!(file_name("file.txt"), Some("file.txt"));
        assert_eq!(file_name("/a/b/c.txt"), Some("c.txt"));
        assert_eq!(file_name("/"), None);
        assert_eq!(file_name("///"), None);
    }

    #[test]
    fn test_is_absolute() {
        assert!(is_absolute("/absolute/path"));
        assert!(!is_absolute("relative/path"));
        assert!(!is_absolute("./relative"));
        assert!(!is_absolute("../relative"));
        
        #[cfg(target_os = "windows")]
        {
            assert!(is_absolute("C:\\path"));
            assert!(is_absolute("\\\\server\\share"));
            assert!(is_absolute("/C/path")); // normalized
        }
    }

    #[test]
    fn test_separator() {
        let sep = separator();
        
        #[cfg(target_os = "windows")]
        assert_eq!(sep, "\\");
        
        #[cfg(not(target_os = "windows"))]
        assert_eq!(sep, "/");
    }

    #[test]
    fn test_normalize_denormalize_roundtrip() {
        let original = "a/b/c";
        let path = Path::new(original);
        let normalized = normalize_path(path);
        let denormalized = denormalize_path(&normalized);
        
        // Should be able to convert back
        assert_eq!(normalize_path(&denormalized), original);
    }

    #[test]
    fn test_windows_drive_paths() {
        #[cfg(target_os = "windows")]
        {
            let path = Path::new("C:\\Users\\test\\file.txt");
            let normalized = normalize_path(path);
            assert_eq!(normalized, "/C/Users/test/file.txt");
            
            let denormalized = denormalize_path(&normalized);
            assert_eq!(denormalized, PathBuf::from("C:\\Users\\test\\file.txt"));
        }
    }

    #[test]
    fn test_unix_absolute_paths() {
        #[cfg(not(target_os = "windows"))]
        {
            let path = Path::new("/home/user/file.txt");
            let normalized = normalize_path(path);
            assert_eq!(normalized, "/home/user/file.txt");
            
            let denormalized = denormalize_path(&normalized);
            assert_eq!(denormalized, PathBuf::from("/home/user/file.txt"));
        }
    }
}
