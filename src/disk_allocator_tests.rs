use anyhow::Result;
use tempfile::tempdir;
use std::fs;
use crate::disk::Disk;

#[test]
fn disk_allocator_and_index_persist() -> Result<()> {
    let td = tempdir()?;
    let disk_dir = td.path().join("disk1");
    fs::create_dir_all(&disk_dir)?;

    let mut disk = Disk::new(disk_dir.clone())?;
    // allocator and index should be present
    assert!(disk.allocator.is_some());
    assert!(disk.free_index.is_some());

    // allocate 2 units
    if let Some(idx) = &mut disk.free_index {
        let start = idx.allocate_best_fit(2).expect("alloc 2");
        // persist happened
    }

    // reload disk
    let disk2 = Disk::load(&disk_dir)?;
    assert!(disk2.allocator.is_some());
    assert!(disk2.free_index.is_some());
    Ok(())
}