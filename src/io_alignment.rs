#![cfg(unix)]

use anyhow::{Context, Result};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::path::Path;
use std::ptr;
use nix::sys::statvfs;

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

/// Perform a direct pwrite using an aligned buffer. Ensures buffer pointer and size and offset
/// are alignment multiples required for O_DIRECT-friendly I/O. Retries on EINTR.
pub fn pwrite_direct(fd: RawFd, buf: &AlignedBuf, offset: u64) -> Result<usize> {
    // Ensure alignment requirements
    if (buf.as_ptr() as usize) % buf.alignment() != 0 {
        anyhow::bail!("buffer pointer not aligned");
    }
    if buf.len() % buf.alignment() != 0 {
        anyhow::bail!("buffer length must be a multiple of alignment");
    }
    if (offset as usize) % buf.alignment() != 0 {
        anyhow::bail!("offset must be a multiple of alignment");
    }

    let mut written = 0usize;
    let mut to_write = buf.len();
    // Use immutable pointer for pwrite
    let mut ptr = buf.as_ptr() as *const libc::c_void;
    let mut off = offset as libc::off_t;

    while to_write > 0 {
        let ret = unsafe { libc::pwrite(fd, ptr, to_write, off) };
        if ret < 0 {
            let eno = io::Error::last_os_error();
            if eno.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(eno.into());
        }
        let wrote = ret as usize;
        written += wrote;
        if wrote == 0 {
            break;
        }
        to_write -= wrote;
        // advance pointer and offset
        ptr = unsafe { (ptr as *const u8).add(wrote) as *const libc::c_void };
        off += wrote as libc::off_t;
    }

    Ok(written)
}

/// Perform a direct pread into an aligned buffer. Ensures buffer pointer and size and offset
/// are alignment multiples required for O_DIRECT-friendly I/O. Retries on EINTR.
pub fn pread_direct(fd: RawFd, buf: &mut AlignedBuf, offset: u64) -> Result<usize> {
    // Ensure alignment requirements
    if (buf.as_ptr() as usize) % buf.alignment() != 0 {
        anyhow::bail!("buffer pointer not aligned");
    }
    if buf.len() % buf.alignment() != 0 {
        anyhow::bail!("buffer length must be a multiple of alignment");
    }
    if (offset as usize) % buf.alignment() != 0 {
        anyhow::bail!("offset must be a multiple of alignment");
    }

    let mut read = 0usize;
    let mut to_read = buf.len();
    let mut ptr = buf.as_mut_ptr() as *mut libc::c_void;
    let mut off = offset as libc::off_t;

    while to_read > 0 {
        let ret = unsafe { libc::pread(fd, ptr, to_read, off) };
        if ret < 0 {
            let eno = io::Error::last_os_error();
            if eno.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(eno.into());
        }
        let r = ret as usize;
        read += r;
        if r == 0 {
            break;
        }
        to_read -= r;
        ptr = unsafe { (ptr as *mut u8).add(r) as *mut libc::c_void };
        off += r as libc::off_t;
    }

    Ok(read)
}

/// Detect the required alignment (in bytes) for I/O on `path`.
/// For block devices, use BLKSSZGET (sector size). For regular files, use filesystem block size.
pub fn detect_alignment_from_path(path: &Path) -> Result<usize> {
    // If path exists and is block device, query block device sector size
    if path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            if fs::metadata(path)?.file_type().is_block_device() {
                use std::os::unix::io::AsRawFd;
                let f = File::open(path)?;
                let fd = f.as_raw_fd();
                // BLKSSZGET ioctl
                let mut sector_size: libc::c_uint = 0;
                const BLKSSZGET: libc::c_ulong = 0x1268;
                let ret = unsafe { libc::ioctl(fd, BLKSSZGET as _, &mut sector_size) };
                if ret == 0 && sector_size > 0 {
                    return Ok(sector_size as usize);
                }
            }
        }
    }

    // Fallback: use statvfs block size for the filesystem containing path
    let dir = if path.exists() {
        if path.is_file() { path.parent().unwrap_or(path) } else { path }
    } else {
        path.parent().unwrap_or(path)
    };

    let stat = statvfs::statvfs(dir).context("statvfs failed")?;
    let bsize = stat.block_size();
    Ok(bsize as usize)
}

/// Write `data` to `path` using aligned buffer and O_DIRECT if possible. If `prefer_direct` is true
/// try to open with O_DIRECT and fall back as needed. Pads short writes up to alignment.
pub fn write_aligned_file(path: &Path, data: &[u8], prefer_direct: bool) -> Result<()> {
    let align = detect_alignment_from_path(path)?;
    // Determine write size: round up to multiple of alignment
    let mut write_size = data.len();
    if write_size % align != 0 {
        write_size += align - (write_size % align);
    }

    let mut buf = AlignedBuf::new(write_size, align)?;
    // Copy data and zero the rest
    buf.as_mut_slice()[..data.len()].copy_from_slice(data);
    for b in &mut buf.as_mut_slice()[data.len()..] {
        *b = 0;
    }

    // Try direct open if requested
    let use_direct = prefer_direct && open_with_o_direct(path).is_ok();

    if use_direct {
        let f = open_with_o_direct(path)?;
        let fd = f.as_raw_fd();
        let wrote = pwrite_direct(fd, &buf, 0)?;
        if wrote != write_size {
            anyhow::bail!("short write for direct I/O: {} != {}", wrote, write_size);
        }
        Ok(())
    } else {
        // Fallback to normal write
        let mut f = OpenOptions::new().create(true).write(true).truncate(true).open(path)?;
        f.write_all(&buf.as_slice()[..data.len()])?;
        f.sync_all()?;
        Ok(())
    }
}

/// Read `len` bytes from `path` into a Vec<u8>, using aligned reads when prefer_direct is true.
pub fn read_aligned_file(path: &Path, len: usize, prefer_direct: bool) -> Result<Vec<u8>> {
    let align = detect_alignment_from_path(path)?;
    let mut read_size = len;
    if read_size % align != 0 {
        read_size += align - (read_size % align);
    }

    let mut buf = AlignedBuf::new(read_size, align)?;
    // Try direct if available
    let use_direct = prefer_direct && open_with_o_direct(path).is_ok();

    if use_direct {
        let f = open_with_o_direct(path)?;
        let fd = f.as_raw_fd();
        let read = pread_direct(fd, &mut buf, 0)?;
        let mut out = vec![0u8; len];
        out[..std::cmp::min(len, read)].copy_from_slice(&buf.as_slice()[..std::cmp::min(len, read)]);
        Ok(out)
    } else {
        let mut f = File::open(path)?;
        let mut out = vec![0u8; len];
        f.read_exact(&mut out)?;
        Ok(out)
    }
}
