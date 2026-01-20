use anyhow::{anyhow, Context, Result};
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::PathBuf;

pub struct Device {
    pub path: PathBuf,
    fd: RawFd,
    pub block_size: usize,
    pub supports_direct: bool,
}

impl Device {
    /// Try opening a device/path with O_DIRECT; fall back to non-direct with O_SYNC.
    pub fn open(path: PathBuf, read_only: bool) -> Result<Self> {
        use libc;

        // Try O_DIRECT first
        let mut flags = if read_only { libc::O_RDONLY } else { libc::O_RDWR };
        flags |= libc::O_CLOEXEC;
        let mut supports_direct = true;

        // attempt open with O_DIRECT
        let fd = OpenOptions::new()
            .read(read_only || true)
            .write(!read_only)
            .custom_flags(flags | libc::O_DIRECT)
            .open(&path);

        let fd = match fd {
            Ok(f) => f,
            Err(e) => {
                // Fallback: open without O_DIRECT but use O_SYNC for durability
                supports_direct = false;
                let fallback = OpenOptions::new()
                    .read(true)
                    .write(!read_only)
                    .custom_flags(libc::O_CLOEXEC | libc::O_SYNC)
                    .open(&path)
                    .context("Failed to open device with fallback flags")?;
                fallback
            }
        };

        let fd_raw = fd.as_raw_fd();

        // Probe block size; if ioctl fails, use sensible default (4096)
        let block_size = match Self::probe_block_size_raw(fd_raw) {
            Ok(sz) => sz,
            Err(_) => 4096usize,
        };

        // Prevent fd from closing by forgetting File (we manage raw fd only)
        std::mem::forget(fd);

        Ok(Device {
            path,
            fd: fd_raw,
            block_size,
            supports_direct,
        })
    }

    /// Probe block size using BLKSSZGET; accepts a raw fd
    fn probe_block_size_raw(fd: RawFd) -> Result<usize> {
        use libc;
        const BLKSSZGET: libc::c_ulong = 0x1268;

        let mut blksz: libc::c_uint = 0;
        let ret = unsafe { libc::ioctl(fd, BLKSSZGET as _, &mut blksz) };
        if ret != 0 {
            return Err(anyhow!("BLKSSZGET ioctl failed"));
        }
        Ok(blksz as usize)
    }

    /// Probe block size by opening the path and querying
    pub fn probe_block_size(path: &PathBuf) -> Result<usize> {
        use std::os::unix::fs::OpenOptionsExt;

        let f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(path)
            .context("Failed to open path for block size probe")?;
        let fd = f.as_raw_fd();
        let result = Self::probe_block_size_raw(fd);
        Ok(result.unwrap_or(4096usize))
    }

    /// Align value up to multiple of `align`
    pub fn align_up(value: usize, align: usize) -> usize {
        if align == 0 {
            return value;
        }
        ((value + align - 1) / align) * align
    }

    /// Check whether a pointer (address) is aligned
    pub fn is_aligned(addr: usize, align: usize) -> bool {
        if align == 0 {
            return true;
        }
        addr % align == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use anyhow::Result;

    #[test]
    fn align_up_works() {
        assert_eq!(Device::align_up(1000, 4096), 4096);
        assert_eq!(Device::align_up(4096, 4096), 4096);
        assert_eq!(Device::align_up(4097, 4096), 8192);
        assert_eq!(Device::align_up(0, 4096), 0);
    }

    #[test]
    fn is_aligned_works() {
        assert!(Device::is_aligned(0, 512));
        assert!(Device::is_aligned(4096, 4096));
        assert!(!Device::is_aligned(1000, 4096));
    }

    #[test]
    fn probe_block_size_default_on_regular_file() -> Result<()> {
        let tmp = NamedTempFile::new()?;
        let path = tmp.path().to_path_buf();
        // Regular files won't support BLKSSZGET; method should return default
        let sz = Device::probe_block_size(&path)?;
        assert!(sz >= 512);
        Ok(())
    }

    #[test]
    fn open_fallback_without_direct() -> Result<()> {
        let tmp = NamedTempFile::new()?;
        let path = tmp.path().to_path_buf();

        let dev = Device::open(path, true)?;
        // On regular files, O_DIRECT likely failed; ensure fallback worked
        assert!(dev.block_size >= 512);
        Ok(())
    }
}
