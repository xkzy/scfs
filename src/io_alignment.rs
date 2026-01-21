#![cfg(unix)]

use anyhow::{Context, Result};
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::path::Path;
use std::ptr;

/// Simple RAII wrapper for aligned memory allocated via posix_memalign
pub struct AlignedBuf {
    ptr: *mut u8,
    size: usize,
    alignment: usize,
}

unsafe impl Send for AlignedBuf {}
unsafe impl Sync for AlignedBuf {}

impl AlignedBuf {
    /// Allocate an aligned buffer of `size` bytes with `alignment`.
    pub fn new(size: usize, alignment: usize) -> Result<Self> {
        if alignment == 0 || !alignment.is_power_of_two() {
            anyhow::bail!("alignment must be a non-zero power of two");
        }
        if size == 0 {
            anyhow::bail!("size must be non-zero");
        }

        let mut ptr: *mut c_void = ptr::null_mut();
        let ret = unsafe { libc::posix_memalign(&mut ptr, alignment, size) };
        if ret != 0 {
            return Err(std::io::Error::from_raw_os_error(ret).into());
        }
        if ptr.is_null() {
            anyhow::bail!("posix_memalign returned null");
        }

        Ok(AlignedBuf {
            ptr: ptr as *mut u8,
            size,
            alignment,
        })
    }

    /// Mutable slice view to the buffer
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) }
    }

    /// Immutable slice view
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.size) }
    }

    /// Raw pointer
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr as *const u8
    }

    /// Raw mutable pointer
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    /// Size in bytes
    pub fn len(&self) -> usize {
        self.size
    }

    /// Alignment
    pub fn alignment(&self) -> usize {
        self.alignment
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe {
            libc::free(self.ptr as *mut c_void);
            self.ptr = ptr::null_mut();
        }
    }
}

/// Open a file with O_DIRECT and O_SYNC set (if supported on the filesystem)
pub fn open_with_o_direct(path: &Path) -> Result<std::fs::File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .custom_flags(libc::O_DIRECT | libc::O_SYNC)
        .open(path)
        .with_context(|| format!("Failed to open {:?} with O_DIRECT", path))?;
    Ok(file)
}

/// Return true if the file descriptor has O_DIRECT flag set
pub fn fd_has_o_direct(fd: RawFd) -> Result<bool> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok((flags & libc::O_DIRECT) != 0)
}

/// Convenience test helper to open a path with O_DIRECT and return whether the flag is present
pub fn open_and_check_o_direct(path: &Path) -> Result<bool> {
    let f = open_with_o_direct(path)?;
    let fd = f.as_raw_fd();
    fd_has_o_direct(fd)
}
