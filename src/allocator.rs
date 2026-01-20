use anyhow::{anyhow, Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAGIC: &[u8; 6] = b"DFSBM\0"; // DynamicFS Bitmap
const VERSION: u32 = 1;

#[derive(Debug)]
pub struct BitmapAllocator {
    /// Allocation unit size in bytes
    pub unit_size: u64,
    /// Total units managed
    pub total_units: u64,
    /// Bitmap (packed bits, LSB first per byte)
    bitmap: Mutex<Vec<u8>>,
    /// Whether allocator is persisted on-disk (directory-backed disks)
    persist_path: Option<PathBuf>,
}

impl Clone for BitmapAllocator {
    fn clone(&self) -> Self {
        let bitmap = self.bitmap.lock().unwrap().clone();
        BitmapAllocator {
            unit_size: self.unit_size,
            total_units: self.total_units,
            bitmap: Mutex::new(bitmap),
            persist_path: self.persist_path.clone(),
        }
    }
}

impl BitmapAllocator {
    /// Create a new allocator and persist if a path is provided
    pub fn new(unit_size: u64, total_units: u64, persist_path: Option<PathBuf>) -> Result<Self> {
        if unit_size == 0 {
            return Err(anyhow!("unit_size must be > 0"));
        }
        let bytes = ((total_units + 7) / 8) as usize;
        let mut bitmap = vec![0u8; bytes];

        let allocator = BitmapAllocator {
            unit_size,
            total_units,
            bitmap: Mutex::new(bitmap),
            persist_path,
        };

        if let Some(path) = &allocator.persist_path {
            allocator.save(path)?;
        }

        Ok(allocator)
    }

    /// Load allocator from path (on-disk format)
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let mut f = fs::File::open(path).context("Failed to open allocator file")?;
        let mut header = [0u8; 6 + 4 + 8 + 8];
        f.read_exact(&mut header).context("Failed to read header")?;
        if &header[0..6] != MAGIC {
            return Err(anyhow!("Allocator magic mismatch"));
        }
        let version = u32::from_le_bytes(header[6..10].try_into().unwrap());
        if version != VERSION {
            return Err(anyhow!("Unsupported allocator version"));
        }
        let unit_size = u64::from_le_bytes(header[10..18].try_into().unwrap());
        let total_units = u64::from_le_bytes(header[18..26].try_into().unwrap());
        let bitmap_bytes = ((total_units + 7) / 8) as usize;
        let mut bitmap = vec![0u8; bitmap_bytes];
        f.read_exact(&mut bitmap).context("Failed to read bitmap")?;

        Ok(BitmapAllocator {
            unit_size,
            total_units,
            bitmap: Mutex::new(bitmap),
            persist_path: Some(path.to_path_buf()),
        })
    }

    /// Persist allocator to disk atomically (write to .tmp then rename)
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut tmp = path.with_extension("bin.tmp");
        let mut file = fs::File::create(&tmp).context("Failed to create tmp allocator file")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&self.unit_size.to_le_bytes())?;
        file.write_all(&self.total_units.to_le_bytes())?;
        let bitmap = self.bitmap.lock().unwrap();
        file.write_all(&bitmap)?;
        file.sync_all()?;
        fs::rename(&tmp, path).context("Failed to atomically persist allocator")?;
        Ok(())
    }

    /// Check if a unit is free
    pub fn is_free(&self, unit_idx: u64) -> bool {
        if unit_idx >= self.total_units {
            return false;
        }
        let bitmap = self.bitmap.lock().unwrap();
        let byte = bitmap[(unit_idx / 8) as usize];
        let bit = 1u8 << (unit_idx % 8);
        (byte & bit) == 0
    }

    /// Mark a unit as used (true) or free (false)
    fn set_unit(&self, unit_idx: u64, used: bool) -> Result<()> {
        if unit_idx >= self.total_units {
            return Err(anyhow!("unit_idx out of range"));
        }
        let mut bitmap = self.bitmap.lock().unwrap();
        let idx = (unit_idx / 8) as usize;
        let bit = 1u8 << (unit_idx % 8);
        if used {
            bitmap[idx] |= bit;
        } else {
            bitmap[idx] &= !bit;
        }
        Ok(())
    }

    /// Find a contiguous run of length 'n' units; return its start unit index or None
    pub fn find_contiguous(&self, n: u64) -> Option<u64> {
        if n == 0 || n > self.total_units {
            return None;
        }
        let bitmap = self.bitmap.lock().unwrap();
        let mut run = 0u64;
        let mut start = 0u64;
        for unit in 0..self.total_units {
            let byte = bitmap[(unit / 8) as usize];
            let bit = 1u8 << (unit % 8);
            if (byte & bit) == 0 {
                if run == 0 {
                    start = unit;
                }
                run += 1;
                if run == n {
                    return Some(start);
                }
            } else {
                run = 0;
            }
        }
        None
    }

    /// Allocate n contiguous units (returns start index) or None (bitmap-only scan)
    pub fn allocate_contiguous(&self, n: u64) -> Option<u64> {
        if let Some(start) = self.find_contiguous(n) {
            for u in start..(start + n) {
                let _ = self.set_unit(u, true);
            }
            if let Some(path) = &self.persist_path {
                let _ = self.save(path);
            }
            return Some(start);
        }
        None
    }

    /// Attempt to allocate n contiguous units using a FreeExtentIndex; returns start or None
    pub fn allocate_contiguous_with_index(&self, n: u64, index: &mut crate::free_extent::FreeExtentIndex) -> Option<u64> {
        if n == 0 || n > self.total_units {
            return None;
        }
        if let Some(start) = index.allocate_best_fit(n) {
            // mark bits in bitmap
            for u in start..(start + n) {
                let _ = self.set_unit(u, true);
            }
            if let Some(path) = &self.persist_path {
                let _ = self.save(path);
            }
            return Some(start);
        }
        // fallback to bitmap scan
        self.allocate_contiguous(n)
    }

    /// Free a contiguous allocation
    pub fn free_contiguous(&self, start: u64, n: u64) -> Result<()> {
        if start + n > self.total_units {
            return Err(anyhow!("range out of bounds"));
        }
        for u in start..(start + n) {
            self.set_unit(u, false)?;
        }
        if let Some(path) = &self.persist_path {
            self.save(path)?;
        }
        Ok(())
    }

    /// Count free units (simple scan)
    pub fn free_count(&self) -> u64 {
        let bitmap = self.bitmap.lock().unwrap();
        let mut c = 0u64;
        for unit in 0..self.total_units {
            let byte = bitmap[(unit / 8) as usize];
            let bit = 1u8 << (unit % 8);
            if (byte & bit) == 0 {
                c += 1;
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_and_persist_allocator() -> Result<()> {
        let td = tempdir()?;
        let path = td.path().join("allocator.bin");
        let unit_size = 1024u64 * 1024u64; // 1 MiB
        let total_units = 10u64;
        let alloc = BitmapAllocator::new(unit_size, total_units, Some(path.clone()))?;
        assert_eq!(alloc.free_count(), 10);
        let s = alloc.allocate_contiguous(2).expect("alloc 2 units");
        assert_eq!(alloc.free_count(), 8);
        // reload
        let re = BitmapAllocator::load_from_path(&path)?;
        assert_eq!(re.free_count(), 8);
        re.free_contiguous(s, 2)?;
        assert_eq!(re.free_count(), 10);
        Ok(())
    }

    #[test]
    fn find_contiguous() -> Result<()> {
        let alloc = BitmapAllocator::new(4096, 100, None)?;
        // occupy some units
        let _ = alloc.allocate_contiguous(3).unwrap();
        let _ = alloc.allocate_contiguous(4).unwrap();
        let start = alloc.find_contiguous(10).unwrap();
        assert!(start >= 0);
        Ok(())
    }
}
