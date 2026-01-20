use anyhow::{anyhow, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Generic persisted B-tree map using bincode serialization with interior mutability.
pub struct PersistedBTree<K, V> {
    inner: Mutex<PersistedBTreeInner<K, V>>,
}

#[derive(Clone)]
struct PersistedBTreeInner<K, V> {
    map: BTreeMap<K, V>,
    persist_path: Option<PathBuf>,
}

impl<K, V> PersistedBTree<K, V>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Send + 'static,
    V: Clone + Serialize + DeserializeOwned + Send + 'static,
{
    pub fn new(persist_path: Option<PathBuf>) -> Result<Self> {
        let mut inner = PersistedBTreeInner {
            map: BTreeMap::new(),
            persist_path: persist_path.clone(),
        };
        if let Some(path) = &persist_path {
            if path.exists() {
                // Try to load existing btree file. If it fails (corrupt/truncated), remove it and start fresh.
                match fs::read(path) {
                    Ok(contents) => match bincode::deserialize(&contents) {
                        Ok(map) => inner.map = map,
                        Err(e) => {
                            // remove the corrupt file and continue with an empty map
                            eprintln!("Warning: failed to deserialize btree file {:?}: {}. Removing corrupted file.", path, e);
                            let _ = fs::remove_file(path);
                        }
                    },
                    Err(e) => {
                        eprintln!("Warning: failed to read btree file {:?}: {}. Ignoring.", path, e);
                    }
                }
            }
        }
        Ok(PersistedBTree {
            inner: Mutex::new(inner),
        })
    }

    pub fn insert(&self, k: K, v: V) -> Result<()> {
        let mut guard = self.inner.lock().unwrap();
        guard.map.insert(k, v);
        // persist
        if let Some(path) = &guard.persist_path {
            let tmp = path.with_extension("bt.tmp");
            let encoded = bincode::serialize(&guard.map).context("Failed to serialize btree map")?;
            fs::write(&tmp, encoded).context("Failed to write tmp btree file")?;
            fs::rename(&tmp, path).context("Failed to atomically persist btree file")?;
        }
        Ok(())
    }

    pub fn remove(&self, k: &K) -> Result<Option<V>> {
        let mut guard = self.inner.lock().unwrap();
        let res = guard.map.remove(k);
        if let Some(path) = &guard.persist_path {
            let tmp = path.with_extension("bt.tmp");
            let encoded = bincode::serialize(&guard.map).context("Failed to serialize btree map")?;
            fs::write(&tmp, encoded).context("Failed to write tmp btree file")?;
            fs::rename(&tmp, path).context("Failed to atomically persist btree file")?;
        }
        Ok(res)
    }

    pub fn get(&self, k: &K) -> Option<V> {
        let guard = self.inner.lock().unwrap();
        guard.map.get(k).cloned()
    }

    pub fn contains_key(&self, k: &K) -> bool {
        let guard = self.inner.lock().unwrap();
        guard.map.contains_key(k)
    }

    pub fn list_keys(&self) -> Vec<K> {
        let guard = self.inner.lock().unwrap();
        guard.map.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        let guard = self.inner.lock().unwrap();
        guard.map.len()
    }
}

#[cfg(test)]
mod tests {
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
}
