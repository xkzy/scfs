use anyhow::{anyhow, Context, Result};
use blake3::Hasher;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use crc32fast;
use nix::fcntl::{flock, FlockArg};

const SUPERBLOCK_MAGIC: &[u8; 8] = b"DFSBLOCK";
const SUPERBLOCK_VERSION: u32 = 1;
pub const SUPERBLOCK_SIZE: usize = 4096;

/// Simple on-device superblock
#[derive(Debug, Clone)]
pub struct Superblock {
    pub magic: [u8; 8],
    pub version: u32,
    pub device_uuid: Uuid,
    pub seq: u64,
    pub allocator_offset: u64,
    pub allocator_len: u64,
    pub checksum: u64,
}

impl Superblock {
    pub fn new(device_uuid: Uuid, seq: u64, allocator_offset: u64, allocator_len: u64) -> Self {
        Superblock {
            magic: *SUPERBLOCK_MAGIC,
            version: SUPERBLOCK_VERSION,
            device_uuid,
            seq,
            allocator_offset,
            allocator_len,
            checksum: 0,
        }
    }

    /// Encode into a SUPERBLOCK_SIZE buffer and compute checksum
    pub fn to_bytes(&mut self) -> [u8; SUPERBLOCK_SIZE] {
        let mut buf = [0u8; SUPERBLOCK_SIZE];
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..12].copy_from_slice(&self.version.to_le_bytes());
        buf[12..28].copy_from_slice(self.device_uuid.as_bytes());
        buf[28..36].copy_from_slice(&self.seq.to_le_bytes());
        buf[36..44].copy_from_slice(&self.allocator_offset.to_le_bytes());
        buf[44..52].copy_from_slice(&self.allocator_len.to_le_bytes());
        // checksum placeholder at 52..60 (8 bytes)

        // compute blake3 over everything except checksum field
        let mut hasher = Hasher::new();
        hasher.update(&buf[0..52]);
        hasher.update(&buf[60..SUPERBLOCK_SIZE]);
        let hash = hasher.finalize();
        let cs = u64::from_le_bytes(hash.as_bytes()[0..8].try_into().unwrap());
        self.checksum = cs;
        buf[52..60].copy_from_slice(&cs.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        if buf.len() < SUPERBLOCK_SIZE {
            anyhow::bail!("buffer too small for superblock");
        }
        if &buf[0..8] != SUPERBLOCK_MAGIC {
            anyhow::bail!("superblock magic mismatch");
        }
        let version = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        if version != SUPERBLOCK_VERSION {
            anyhow::bail!("unsupported superblock version");
        }
        let uuid = Uuid::from_bytes(buf[12..28].try_into().unwrap());
        let seq = u64::from_le_bytes(buf[28..36].try_into().unwrap());
        let allocator_offset = u64::from_le_bytes(buf[36..44].try_into().unwrap());
        let allocator_len = u64::from_le_bytes(buf[44..52].try_into().unwrap());
        let cs = u64::from_le_bytes(buf[52..60].try_into().unwrap());
        // verify checksum
        let mut hasher = Hasher::new();
        hasher.update(&buf[0..52]);
        hasher.update(&buf[60..SUPERBLOCK_SIZE]);
        let hash = hasher.finalize();
        let expected = u64::from_le_bytes(hash.as_bytes()[0..8].try_into().unwrap());
        if cs != expected {
            anyhow::bail!("superblock checksum mismatch");
        }

        Ok(Superblock {
            magic: *SUPERBLOCK_MAGIC,
            version,
            device_uuid: uuid,
            seq,
            allocator_offset,
            allocator_len,
            checksum: cs,
        })
    }
}

/// Minimal on-device allocator scaffold.
pub struct OnDeviceAllocator {
    device_path: PathBuf,
    superblock_offset: u64,
    pub unit_size: u64,
    pub total_units: u64,
    /// In-memory bitmap (LSB first per byte)
    bitmap: Vec<u8>,
    allocator_offset: u64,
}

/// Guard that holds an exclusive lock on a device. Releases the lock when dropped.
pub struct DeviceLock {
    file: File,
}

impl Drop for DeviceLock {
    fn drop(&mut self) {
        use nix::fcntl::{flock, FlockArg};
        let _ = flock(self.file.as_raw_fd(), FlockArg::Unlock);
    }
}

/// Fragment header stored before each fragment on-device
pub struct FragmentHeader {
    pub extent_uuid: Uuid,
    pub fragment_index: u32,
    pub total_length: u64,
    pub data_checksum: [u8; 32],
}

impl FragmentHeader {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(16 + 4 + 8 + 32 + 4);
        buf.extend_from_slice(self.extent_uuid.as_bytes());
        buf.extend_from_slice(&self.fragment_index.to_le_bytes());
        buf.extend_from_slice(&self.total_length.to_le_bytes());
        buf.extend_from_slice(&self.data_checksum);
        // header checksum (crc32 little-endian)
        let crc = crc32fast::hash(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        if buf.len() < (16 + 4 + 8 + 32 + 4) {
            anyhow::bail!("buffer too small for fragment header");
        }
        let uuid = Uuid::from_bytes(buf[0..16].try_into().unwrap());
        let fragment_index = u32::from_le_bytes(buf[16..20].try_into().unwrap());
        let total_length = u64::from_le_bytes(buf[20..28].try_into().unwrap());
        let mut data_checksum = [0u8; 32];
        data_checksum.copy_from_slice(&buf[28..60]);
        let stored_crc = u32::from_le_bytes(buf[60..64].try_into().unwrap());
        let crc = crc32fast::hash(&buf[0..60]);
        if crc != stored_crc {
            anyhow::bail!("fragment header checksum mismatch");
        }
        Ok(FragmentHeader { extent_uuid: uuid, fragment_index, total_length, data_checksum })
    }
}

impl OnDeviceAllocator {
    /// Check whether a valid superblock exists on `path` (useful to detect pre-existing device data)
    pub fn has_superblock(path: &Path) -> bool {
        if let Ok(mut f) = OpenOptions::new().read(true).open(path) {
            let mut buf = vec![0u8; SUPERBLOCK_SIZE];
            if f.read_exact(&mut buf).is_ok() {
                return &buf[0..8] == SUPERBLOCK_MAGIC;
            }
        }
        false
    }

    /// Acquire exclusive flock on device path. Returns a guard that releases the lock on Drop.
    pub fn acquire_device_lock(path: &Path) -> Result<DeviceLock> {
        use std::os::unix::io::AsRawFd;
        use nix::fcntl::{flock, FlockArg};

        let file = OpenOptions::new().read(true).write(true).open(path).context("Failed to open device for locking")?;
        let fd = file.as_raw_fd();
        flock(fd, FlockArg::LockExclusiveNonblock).context("Failed to acquire exclusive lock on device (is it in use?)")?;
        Ok(DeviceLock { file })
    }

    /// Format the device with a superblock and empty bitmap allocator.
    /// `device_size` is the total size to ensure the file/device is large enough when testing with files.
    pub fn format_device(path: &Path, device_uuid: Uuid, device_size: u64, unit_size: u64, total_units: u64) -> Result<()> {
        let allocator_bytes = ((total_units + 7) / 8) as usize;
        let allocator_offset = 64 * 1024u64; // place allocator after 64KB
        let allocator_len = allocator_bytes as u64;

        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .context("Failed to open device file for formatting")?;
        // ensure size
        f.set_len(device_size).context("Failed to set device size")?;

        // write empty bitmap
        f.seek(SeekFrom::Start(allocator_offset))?;
        let zeros = vec![0u8; allocator_bytes];
        f.write_all(&zeros)?;
        f.sync_all()?;

        // write superblock
        let mut sb = Superblock::new(device_uuid, 1, allocator_offset, allocator_len);
        let buf = sb.to_bytes();
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&buf)?;
        f.sync_all()?;
        Ok(())
    }

    /// Load allocator from a device path using superblock at offset 0
    pub fn load_from_device(path: &Path) -> Result<Self> {
        let mut f = OpenOptions::new().read(true).write(true).open(path).context("Failed to open device file")?;
        let mut buf = vec![0u8; SUPERBLOCK_SIZE];
        f.seek(SeekFrom::Start(0))?;
        f.read_exact(&mut buf)?;
        let sb = Superblock::from_bytes(&buf)?;
        // read bitmap
        let mut bitmap = vec![0u8; sb.allocator_len as usize];
        f.seek(SeekFrom::Start(sb.allocator_offset))?;
        f.read_exact(&mut bitmap)?;

        // derive unit_size from caller expectations; for now keep a convention
        let unit_size = 1024 * 1024; // placeholder default 1MiB
        let total_units = (sb.allocator_len as u64) * 8;

        Ok(OnDeviceAllocator {
            device_path: path.to_path_buf(),
            superblock_offset: 0,
            unit_size,
            total_units,
            bitmap,
            allocator_offset: sb.allocator_offset,
        })
    }

    /// Persist bitmap to device and bump superblock seq
    pub fn persist(&mut self) -> Result<()> {
        let mut f = OpenOptions::new().read(true).write(true).open(&self.device_path).context("Failed to open device for persist")?;
        f.seek(SeekFrom::Start(self.allocator_offset))?;
        f.write_all(&self.bitmap)?;
        f.sync_all()?;

        // update superblock seq
        let mut buf = vec![0u8; SUPERBLOCK_SIZE];
        f.seek(SeekFrom::Start(0))?;
        f.read_exact(&mut buf)?;
        let mut sb = Superblock::from_bytes(&buf)?;
        sb.seq += 1;
        let sbbuf = sb.to_bytes();
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&sbbuf)?;
        f.sync_all()?;
        Ok(())
    }

    /// Attempt to allocate n contiguous units by simple scan
    pub fn allocate_contiguous(&mut self, n: u64) -> Option<u64> {
        if n == 0 || n > self.total_units {
            return None;
        }
        let mut run = 0u64;
        let mut start = 0u64;
        for unit in 0..self.total_units {
            let byte = self.bitmap[(unit / 8) as usize];
            let bit = 1u8 << (unit % 8);
            if (byte & bit) == 0 {
                if run == 0 { start = unit; }
                run += 1;
                if run == n {
                    // mark
                    for u in start..(start + n) {
                        let idx = (u / 8) as usize;
                        let b = 1u8 << (u % 8);
                        self.bitmap[idx] |= b;
                    }
                    return Some(start);
                }
            } else {
                run = 0;
            }
        }
        None
    }

    /// Compute where fragment data region starts (right after allocator region, aligned to unit_size)
    fn data_region_base(&self) -> u64 {
        let end = self.allocator_offset + (self.bitmap.len() as u64);
        let mask = self.unit_size - 1;
        if end & mask == 0 {
            end
        } else {
            (end + self.unit_size) & !mask
        }
    }

    /// Write a fragment (header + data) into the allocated units starting at `start_unit`.
    /// Steps: write header+data (padded to units), fdatasync, persist bitmap & bump superblock.
    pub fn write_fragment_at(&mut self, start_unit: u64, data: &[u8], hdr: &FragmentHeader) -> Result<()> {
        let n_units = ((hdr.total_length + (16 + 4 + 8 + 32 + 4) as u64) + self.unit_size - 1) / self.unit_size;
        if start_unit + n_units > self.total_units {
            anyhow::bail!("allocation out of range");
        }
        let offset = self.data_region_base() + start_unit * self.unit_size;
        let mut f = OpenOptions::new().read(true).write(true).open(&self.device_path).context("Failed to open device for fragment write")?;
        // build payload: header + data, pad to multiple of unit_size
        let mut payload = hdr.to_bytes();
        payload.extend_from_slice(&data);
        let payload_len = payload.len() as u64;
        let total_write = ((payload_len + self.unit_size - 1) / self.unit_size) * self.unit_size;
        payload.resize(total_write as usize, 0u8);

        f.seek(SeekFrom::Start(offset))?;
        f.write_all(&payload)?;
        // ensure data durability
        let fd = f.as_raw_fd();
        unsafe { libc::fdatasync(fd) };

        // Persist allocator bitmap and superblock
        self.persist()?;
        Ok(())
    }

    /// Read a fragment from start_unit and verify header checksum and data checksum
    pub fn read_fragment_at(&self, start_unit: u64) -> Result<(FragmentHeader, Vec<u8>)> {
        let offset = self.data_region_base() + start_unit * self.unit_size;
        let mut f = OpenOptions::new().read(true).open(&self.device_path).context("Failed to open device for fragment read")?;
        // read header first
        let mut hbuf = vec![0u8; (16+4+8+32+4)];
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(&mut hbuf)?;
        let hdr = FragmentHeader::from_bytes(&hbuf)?;
        let data_len = hdr.total_length as usize;
        let mut data = vec![0u8; data_len];
        f.read_exact(&mut data)?;
        // verify data checksum
        let ch = blake3::hash(&data);
        if ch.as_bytes() != &hdr.data_checksum {
            anyhow::bail!("data checksum mismatch");
        }
        Ok((hdr, data))
    }

    pub fn free_contiguous(&mut self, start: u64, n: u64) -> Result<()> {
        if start + n > self.total_units { anyhow::bail!("out of range"); }
        for u in start..(start+n) {
            let idx = (u / 8) as usize;
            let b = 1u8 << (u % 8);
            self.bitmap[idx] &= !b;
        }
        Ok(())
    }

    pub fn free_count(&self) -> u64 {
        let mut c = 0u64;
        for unit in 0..self.total_units {
            let byte = self.bitmap[(unit / 8) as usize];
            let bit = 1u8 << (unit % 8);
            if (byte & bit) == 0 { c += 1; }
        }
        c
    }
}
