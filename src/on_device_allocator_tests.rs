use anyhow::Result;
use tempfile::NamedTempFile;
use uuid::Uuid;
use std::fs::OpenOptions;
use std::io::Write;

use crate::on_device_allocator::{OnDeviceAllocator, Superblock, SUPERBLOCK_SIZE};

#[test]
fn test_superblock_roundtrip() -> Result<()> {
    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();

    let uuid = Uuid::new_v4();
    // format with 1MB device size and small allocator (16 units)
    OnDeviceAllocator::format_device(&path, uuid, 2 * 1024 * 1024, 1024 * 1024, 16)?;

    // Read back superblock bytes
    let mut f = OpenOptions::new().read(true).open(&path)?;
    let mut buf = vec![0u8; SUPERBLOCK_SIZE];
    f.read_exact(&mut buf)?;
    let sb = Superblock::from_bytes(&buf)?;
    assert_eq!(sb.device_uuid, uuid);
    assert!(sb.allocator_len > 0);
    Ok(())
}

#[test]
fn test_ondevice_allocator_alloc_persist() -> Result<()> {
    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();
    let uuid = Uuid::new_v4();
    // 4KB device with 64KB size
    let device_size = 256 * 1024u64;
    let unit_size = 4096u64; // placeholder
    let total_units = 64u64;

    OnDeviceAllocator::format_device(&path, uuid, device_size, unit_size, total_units)?;

    let mut alloc = OnDeviceAllocator::load_from_device(&path)?;
    assert_eq!(alloc.free_count(), total_units);

    // allocate 3 units
    let start = alloc.allocate_contiguous(3).expect("alloc 3 units");
    assert!(alloc.free_count() <= total_units - 3);
    alloc.persist()?;

    // reload and ensure allocation persisted
    let alloc2 = OnDeviceAllocator::load_from_device(&path)?;
    assert_eq!(alloc2.free_count(), alloc.free_count());
    Ok(())
}
