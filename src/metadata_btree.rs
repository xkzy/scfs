use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize, Deserialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
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

