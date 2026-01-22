use anyhow::Result;
use tempfile::tempdir;
use crate::metadata_btree::PersistedBTree;
use uuid::Uuid;

#[test]
fn inode_table_integration() -> Result<()> {
    let td = tempdir()?;
    let p = td.path().join("inode_table.bin");
    type Inode = (u64, String); // (size, name)
    let mut table: PersistedBTree<u64, Inode> = PersistedBTree::new(Some(p.clone()))?;
    table.insert(42, (1234, "file.txt".to_string()))?;
    assert!(table.contains_key(&42));
    let table2: PersistedBTree<u64, Inode> = PersistedBTree::new(Some(p.clone()))?;
    assert!(table2.contains_key(&42));
    Ok(())
}

#[test]
fn extent_mapping_integration() -> Result<()> {
    let td = tempdir()?;
    let p = td.path().join("extent_map.bin");
    type Placement = Vec<(Uuid, u64, u64)>;
    let mut map: PersistedBTree<Uuid, Placement> = PersistedBTree::new(Some(p.clone()))?;
    let ext = Uuid::new_v4();
    let disk = Uuid::new_v4();
    map.insert(ext, vec![(disk, 0, 1)])?;
    assert!(map.contains_key(&ext));
    let map2: PersistedBTree<Uuid, Placement> = PersistedBTree::new(Some(p.clone()))?;
    assert!(map2.contains_key(&ext));
    Ok(())
}