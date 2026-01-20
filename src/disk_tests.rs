use tempfile::tempdir;
use std::path::PathBuf;
use uuid::Uuid;
use anyhow::Result;

use crate::disk::{Disk, DiskKind};

#[test]
fn write_fragment_refused_on_block_device() -> Result<()> {
    let td = tempdir()?;
    let path = td.path().to_path_buf();

    let mut disk = Disk {
        uuid: Uuid::new_v4(),
        path: path.clone(),
        capacity_bytes: 1024 * 1024,
        used_bytes: 0,
        health: crate::disk::DiskHealth::Healthy,
        kind: DiskKind::BlockDevice,
        allocator: None,
        free_index: None,
    };

    let res = disk.write_fragment(&Uuid::new_v4(), 0, b"hello");
    assert!(res.is_err());
    Ok(())
}

#[test]
fn disk_kind_default_directory() -> Result<()> {
    let td = tempdir()?;
    let path = td.path().to_path_buf();

    let disk = Disk::new(path.clone())?;
    assert_eq!(disk.kind, DiskKind::Directory);
    Ok(())
}