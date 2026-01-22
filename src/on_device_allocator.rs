use anyhow::{Context, Result};
use blake3::Hasher;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use crc32fast;
use serde::{Serialize, Deserialize};

use crate::free_extent::FreeExtentIndex;

#[cfg(test)]
use crate::crash_sim::{check_crash_point, CrashPoint};

#[cfg(target_os = "linux")]
const BLKDISCARD: u64 = 0x12 << 8 | 119;

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
#[derive(Debug, Clone)]
pub struct OnDeviceAllocator {
    device_path: PathBuf,
    superblock_offset: u64,
    pub unit_size: u64,
    pub total_units: u64,
    /// In-memory bitmap (LSB first per byte)
    bitmap: Vec<u8>,
    /// Free extent index for efficient contiguous allocation
    free_extents: FreeExtentIndex,
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

/// Placement details for on-device fragment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnDevicePlacement {
    pub start_unit: u64,
    pub unit_count: u64,
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

        // Initialize free extent index
        let mut free_extents = FreeExtentIndex::new(None)?;
        for unit in 0..total_units {
            let byte = bitmap[(unit / 8) as usize];
            let bit = 1u8 << (unit % 8);
            if (byte & bit) == 0 {
                // Free unit - add to free extents
                free_extents.insert_run(unit, 1)?;
            }
        }

        let mut oda = OnDeviceAllocator {
            device_path: path.to_path_buf(),
            superblock_offset: 0,
            unit_size,
            total_units,
            bitmap,
            free_extents,
            allocator_offset: sb.allocator_offset,
        };

        // Run quick reconciliation to ensure bitmap reflects present fragments
        let changed = oda.reconcile_and_persist()?;
        if changed {
            log::info!("On-device allocator reconciled and persisted changes for {}", path.display());
        }

        Ok(oda)
    }

    /// Persist bitmap to device and bump superblock seq atomically
    pub fn persist(&mut self) -> Result<()> {
        let mut f = OpenOptions::new().read(true).write(true).open(&self.device_path).context("Failed to open device for persist")?;
        f.seek(SeekFrom::Start(self.allocator_offset))?;
        f.write_all(&self.bitmap)?;
        f.sync_all()?;

        // Atomic superblock update: write to temporary location first
        let temp_sb_offset = SUPERBLOCK_SIZE as u64; // Use second superblock slot as temp
        let mut buf = vec![0u8; SUPERBLOCK_SIZE];
        f.seek(SeekFrom::Start(0))?;
        f.read_exact(&mut buf)?;
        let mut sb = Superblock::from_bytes(&buf)?;
        sb.seq += 1;
        let sbbuf = sb.to_bytes();

        // Write to temp location
        f.seek(SeekFrom::Start(temp_sb_offset))?;
        f.write_all(&sbbuf)?;
        f.sync_all()?;

        // Atomically copy to primary location (in practice, this would use filesystem atomic rename,
        // but for block devices we ensure the write is durable)
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&sbbuf)?;
        f.sync_all()?;

        Ok(())
    }

    /// Attempt to allocate n contiguous units using free extent index
    pub fn allocate_contiguous(&mut self, n: u64) -> Option<u64> {
        if n == 0 || n > self.total_units {
            return None;
        }

        // Use free extent index for efficient allocation
        if let Some(start) = self.free_extents.allocate_best_fit(n) {
            // Mark units as allocated in bitmap
            for u in start..(start + n) {
                let idx = (u / 8) as usize;
                let b = 1u8 << (u % 8);
                self.bitmap[idx] |= b;
            }
            Some(start)
        } else {
            None
        }
    }

    /// Get a reference to the allocation bitmap
    pub fn bitmap(&self) -> &[u8] {
        &self.bitmap
    }

    /// Compute where fragment data region starts (right after allocator region, aligned to unit_size)
    pub fn data_region_base(&self) -> u64 {
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
    pub fn write_fragment_at(&mut self, start_unit: u64, data: &[u8], hdr: &FragmentHeader) -> Result<OnDevicePlacement> {
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

        #[cfg(test)]
        check_crash_point(CrashPoint::BeforeFragmentDataWrite)?;

        f.seek(SeekFrom::Start(offset))?;
        f.write_all(&payload)?;

        #[cfg(test)]
        check_crash_point(CrashPoint::AfterFragmentDataWrite)?;

        // ensure data durability
        let fd = f.as_raw_fd();
        unsafe { libc::fdatasync(fd) };

        #[cfg(test)]
        check_crash_point(CrashPoint::AfterFragmentFsync)?;

        // Persist allocator bitmap and superblock
        self.persist()?;

        Ok(OnDevicePlacement { start_unit, unit_count: n_units })
    }

    /// Read a fragment from start_unit and verify header checksum and data checksum
    pub fn read_fragment_at(&self, start_unit: u64) -> Result<(FragmentHeader, Vec<u8>)> {
        let offset = self.data_region_base() + start_unit * self.unit_size;
        let mut f = OpenOptions::new().read(true).open(&self.device_path).context("Failed to open device for fragment read")?;
        // read header first
        let mut hbuf = vec![0u8; 16+4+8+32+4];
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
        // Update free extent index
        self.free_extents.insert_run(start, n)?;
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

    /// Perform TRIM (discard) operation on freed units to reclaim space on SSDs
    /// This is a no-op on devices that don't support TRIM
    pub fn trim_freed_units(&self, start_unit: u64, unit_count: u64) -> Result<()> {
        if unit_count == 0 {
            return Ok(());
        }

        let offset = self.data_region_base() + start_unit * self.unit_size;
        let length = unit_count * self.unit_size;

        #[cfg(target_os = "linux")]
        {
            match OpenOptions::new().write(true).open(&self.device_path) {
                Ok(file) => {
                    let fd = file.as_raw_fd();
                    let range = [offset, length];
                    // BLKDISCARD ioctl - ignore errors as TRIM may not be supported
                    unsafe {
                        let _ = libc::ioctl(fd, BLKDISCARD, &range);
                    }
                }
                Err(_) => {
                    // If we can't open for writing, skip TRIM
                }
            }
        }

        Ok(())
    }

    /// Enhanced free_contiguous that also performs TRIM operation
    pub fn free_and_trim(&mut self, start: u64, n: u64) -> Result<()> {
        self.free_contiguous(start, n)?;
        self.trim_freed_units(start, n)?;
        Ok(())
    }

    /// Defragmentation: consolidate fragmented free space by moving allocated units
    /// Returns the number of units moved
    pub fn defragment(&mut self) -> Result<u64> {
        let mut units_moved = 0u64;
        let mut write_pos = 0u64;

        // Simple defrag algorithm: move allocated units to the beginning
        for read_pos in 0..self.total_units {
            let read_idx = (read_pos / 8) as usize;
            let read_bit = 1u8 << (read_pos % 8);

            if (self.bitmap[read_idx] & read_bit) != 0 {
                // Unit is allocated
                if read_pos != write_pos {
                    // Move the fragment from read_pos to write_pos
                    self.move_fragment(read_pos, write_pos)?;
                    units_moved += 1;
                }
                write_pos += 1;
            }
        }

        // Clear the bitmap for the now-free units at the end and rebuild free extents
        self.free_extents = FreeExtentIndex::new(None)?;
        for unit in write_pos..self.total_units {
            let idx = (unit / 8) as usize;
            let bit = 1u8 << (unit % 8);
            self.bitmap[idx] &= !bit;
            self.free_extents.insert_run(unit, 1)?;
        }

        if units_moved > 0 {
            self.persist()?;
        }

        Ok(units_moved)
    }

    /// Move a fragment from one unit position to another
    fn move_fragment(&mut self, from_unit: u64, to_unit: u64) -> Result<()> {
        // Read the fragment at from_unit
        let (header, data) = self.read_fragment_at(from_unit)?;

        // Calculate how many units this fragment occupies
        let fragment_size = ((header.total_length + (16 + 4 + 8 + 32 + 4) as u64) + self.unit_size - 1) / self.unit_size;

        // Free the old location
        self.free_contiguous(from_unit, fragment_size)?;

        // Write to new location
        let placement = self.write_fragment_at(to_unit, &data, &header)?;

        // Update bitmap for the new location
        for unit in to_unit..(to_unit + fragment_size) {
            let idx = (unit / 8) as usize;
            let bit = 1u8 << (unit % 8);
            self.bitmap[idx] |= bit;
        }

        Ok(())
    }

    /// Scan data region for valid fragment headers and ensure bitmap marks used units.
    /// Returns true if bitmap was changed and persisted.
    pub fn reconcile_and_persist(&mut self) -> Result<bool> {
        let mut changed = false;
        let base = self.data_region_base();
        let mut f = OpenOptions::new().read(true).open(&self.device_path).context("Failed to open device during reconcile")?;
        for unit in 0..self.total_units {
            let offset = base + unit * self.unit_size;
            let mut hbuf = vec![0u8; 16+4+8+32+4];
            if let Ok(_) = f.seek(SeekFrom::Start(offset)).and_then(|_| f.read_exact(&mut hbuf)) {
                if let Ok(hdr) = FragmentHeader::from_bytes(&hbuf) {
                    // header valid; check data checksum by reading exact data length
                    let data_len = hdr.total_length as usize;
                    // avoid reading huge data in reconcile; ensure within unit bounds
                    if data_len > (self.unit_size as usize) * 1024 { // safety cap
                        continue;
                    }
                    let mut data = vec![0u8; data_len];
                    if f.read_exact(&mut data).is_ok() {
                        let ch = blake3::hash(&data);
                        if ch.as_bytes() == &hdr.data_checksum {
                            // header + data valid - mark the units that correspond to this fragment
                            let fragment_total = ((data_len + (16+4+8+32+4) + self.unit_size as usize -1) / self.unit_size as usize) as u64;
                            for u in unit..(unit + fragment_total) {
                                let idx = (u / 8) as usize;
                                let b = 1u8 << (u % 8);
                                if (self.bitmap[idx] & b) == 0 {
                                    self.bitmap[idx] |= b;
                                    changed = true;
                                    // Remove from free extents if it was marked free
                                    let _ = self.free_extents.consume_range(u, 1);
                                }
                            }
                        }
                    }
                }
            }
        }

        if changed {
            self.persist()?;
            return Ok(true);
        }
        Ok(false)
    }
}

