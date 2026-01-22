// moved from src/metadata_btree.rs
use super::*;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
    struct Inode {
        id: u64,
        size: u64,
        name: String,
    }

    #[test]
    fn persisted_btree_basic() -> Result<()> {
        let td = tempdir()?;
        let p = td.path().join("inode.btree");
        let mut tb: PersistedBTree<u64, Inode> = PersistedBTree::new(Some(p.clone()))?;
        let inode = Inode { id: 1, size: 1024, name: "foo".to_string() };
        tb.insert(1, inode.clone())?;
        assert_eq!(tb.len(), 1);
        assert!(tb.contains_key(&1));
        // reload
        let tb2: PersistedBTree<u64, Inode> = PersistedBTree::new(Some(p.clone()))?;
        assert_eq!(tb2.len(), 1);
        assert_eq!(tb2.get(&1).unwrap().name, "foo");
        Ok(())
    }

    #[test]
    fn extent_fragment_mapping() -> Result<()> {
        // Map extent UUID -> Vec<(disk_uuid, unit_idx, len)>
        type Placement = Vec<(Uuid, u64, u64)>;
        let td = tempdir()?;
        let p = td.path().join("extent_map.bin");
        let mut idx: PersistedBTree<Uuid, Placement> = PersistedBTree::new(Some(p.clone()))?;
        let extent = Uuid::new_v4();
        let disk = Uuid::new_v4();
        idx.insert(extent, vec![(disk, 0, 2)])?;
        assert!(idx.contains_key(&extent));
        let idx2: PersistedBTree<Uuid, Placement> = PersistedBTree::new(Some(p.clone()))?;
        assert!(idx2.contains_key(&extent));
        Ok(())
    }
