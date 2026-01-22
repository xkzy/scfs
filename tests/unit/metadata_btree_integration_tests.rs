use anyhow::Result;
use tempfile::tempdir;
use crate::metadata::MetadataManager;

#[test]
fn metadata_btree_roundtrip() -> Result<()> {
    let td = tempdir()?;
    let pool = td.path().to_path_buf();
    let mut mgr = MetadataManager::new(pool.clone())?;
    let ino = mgr.allocate_ino();
    let inode = crate::metadata::Inode::new_file(ino, 1, "f1".to_string());
    mgr.save_inode(&inode)?;
    // reload manager
    let mgr2 = MetadataManager::new(pool.clone())?;
    let loaded = mgr2.load_inode(ino)?;
    assert_eq!(loaded.name, "f1");
    Ok(())
}