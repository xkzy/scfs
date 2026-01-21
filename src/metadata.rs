use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::extent::Extent;

#[cfg(test)]
use crate::crash_sim::{check_crash_point, CrashPoint};

/// POSIX file type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileType {
    RegularFile,
    Directory,
}

/// Extended attributes storage
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtendedAttributes {
    pub attrs: std::collections::BTreeMap<String, Vec<u8>>,  // BTreeMap for deterministic serialization
}

/// ACL entry for access control
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AclEntry {
    pub tag: AclTag,
    pub qualifier: Option<u32>, // uid/gid for USER/GROUP tags
    pub permissions: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AclTag {
    UserObj,     // ACL_USER_OBJ
    User,        // ACL_USER
    GroupObj,    // ACL_GROUP_OBJ
    Group,       // ACL_GROUP
    Mask,        // ACL_MASK
    Other,       // ACL_OTHER
}

/// Inode represents a file or directory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inode {
    pub ino: u64,
    pub parent_ino: u64,
    pub file_type: FileType,
    pub name: String,
    pub size: u64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub xattrs: Option<ExtendedAttributes>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub acl: Option<Vec<AclEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,  // BLAKE3 checksum of serialized inode (excluding this field)
}

impl Inode {
    pub fn new_file(ino: u64, parent_ino: u64, name: String) -> Self {
        let now = chrono::Utc::now().timestamp();
        Inode {
            ino,
            parent_ino,
            file_type: FileType::RegularFile,
            name,
            size: 0,
            atime: now,
            mtime: now,
            ctime: now,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            mode: 0o644,
            xattrs: None,
            acl: None,
            checksum: None,
        }
    }
    
    pub fn new_dir(ino: u64, parent_ino: u64, name: String) -> Self {
        let now = chrono::Utc::now().timestamp();
        Inode {
            ino,
            parent_ino,
            file_type: FileType::Directory,
            name,
            size: 0,
            atime: now,
            mtime: now,
            ctime: now,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            mode: 0o755,
            xattrs: None,
            acl: None,
            checksum: None,
        }
    }
    
    /// Get extended attribute
    pub fn get_xattr(&self, name: &str) -> Option<&[u8]> {
        self.xattrs.as_ref()?.attrs.get(name).map(|v| v.as_slice())
    }
    
    /// Set extended attribute
    pub fn set_xattr(&mut self, name: String, value: Vec<u8>) {
        let xattrs = self.xattrs.get_or_insert_with(Default::default);
        xattrs.attrs.insert(name, value);
    }
    
    /// Remove extended attribute
    pub fn remove_xattr(&mut self, name: &str) -> Option<Vec<u8>> {
        self.xattrs.as_mut()?.attrs.remove(name)
    }
    
    /// List all extended attribute names
    pub fn list_xattrs(&self) -> Vec<String> {
        self.xattrs
            .as_ref()
            .map(|x| x.attrs.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// Maps a file to its extents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtentMap {
    pub ino: u64,
    pub extents: Vec<Uuid>, // Ordered list of extent UUIDs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,  // BLAKE3 checksum of serialized map (excluding this field)
}

/// Metadata manager
pub struct MetadataManager {
    pool_dir: PathBuf,
    next_ino: u64,
    // persisted btrees for fast metadata lookup
    pub inode_table: crate::metadata_btree::PersistedBTree<u64, Inode>,
    pub extent_map_table: crate::metadata_btree::PersistedBTree<u64, ExtentMap>,
}

impl MetadataManager {
    /// Compute BLAKE3 checksum for an inode (excluding the checksum field)
    fn compute_inode_checksum(inode: &Inode) -> String {
        let mut inode_copy = inode.clone();
        inode_copy.checksum = None;
        let json = serde_json::to_string(&inode_copy).unwrap();
        blake3::hash(json.as_bytes()).to_hex().to_string()
    }
    
    /// Compute BLAKE3 checksum for an extent map (excluding the checksum field)
    fn compute_extent_map_checksum(map: &ExtentMap) -> String {
        let mut map_copy = map.clone();
        map_copy.checksum = None;
        let json = serde_json::to_string(&map_copy).unwrap();
        blake3::hash(json.as_bytes()).to_hex().to_string()
    }
    
    /// Verify inode checksum
    fn verify_inode_checksum(inode: &Inode) -> Result<()> {
        if let Some(stored_checksum) = &inode.checksum {
            let computed = Self::compute_inode_checksum(inode);
            if &computed != stored_checksum {
                return Err(anyhow!(
                    "Inode {} checksum mismatch: expected {}, got {}",
                    inode.ino,
                    stored_checksum,
                    computed
                ));
            }
        }
        Ok(())
    }
    
    /// Verify extent map checksum
    fn verify_extent_map_checksum(map: &ExtentMap) -> Result<()> {
        if let Some(stored_checksum) = &map.checksum {
            let computed = Self::compute_extent_map_checksum(map);
            if &computed != stored_checksum {
                return Err(anyhow!(
                    "ExtentMap {} checksum mismatch: expected {}, got {}",
                    map.ino,
                    stored_checksum,
                    computed
                ));
            }
        }
        Ok(())
    }

    pub fn new(pool_dir: PathBuf) -> Result<Self> {
        // Create metadata directories
        fs::create_dir_all(pool_dir.join("metadata"))?;
        fs::create_dir_all(pool_dir.join("inodes"))?;
        fs::create_dir_all(pool_dir.join("extent_maps"))?;
        fs::create_dir_all(pool_dir.join("extents"))?;
        
        // Load or initialize next_ino
        let next_ino = Self::load_next_ino(&pool_dir).unwrap_or(2); // 1 is reserved for root
        
        // Initialize persisted B-trees
        let inode_btree_path = pool_dir.join("metadata").join("inodes.btree");
        let extent_map_btree_path = pool_dir.join("metadata").join("extent_maps.btree");

        let inode_table = crate::metadata_btree::PersistedBTree::new(Some(inode_btree_path))?;
        let extent_map_table = crate::metadata_btree::PersistedBTree::new(Some(extent_map_btree_path))?;

        let mut manager = MetadataManager {
            pool_dir,
            next_ino,
            inode_table,
            extent_map_table,
        };
        
        // Ensure root directory exists
        manager.ensure_root()?;
        
        Ok(manager)
    }
    
    fn ensure_root(&mut self) -> Result<()> {
        if !self.inode_exists(1) {
            let root = Inode::new_dir(1, 1, String::from(""));
            self.save_inode(&root)?;
            // also record in inode_table
            self.inode_table.insert(1, root)?;
        }
        Ok(())
    }
    
    pub fn allocate_ino(&mut self) -> u64 {
        let ino = self.next_ino;
        self.next_ino += 1;
        self.save_next_ino().ok();
        ino
    }
    
    fn load_next_ino(pool_dir: &Path) -> Result<u64> {
        let path = pool_dir.join("metadata").join("next_ino");
        let contents = fs::read_to_string(path)?;
        Ok(contents.trim().parse()?)
    }
    
    fn save_next_ino(&self) -> Result<()> {
        let path = self.pool_dir.join("metadata").join("next_ino");
        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, self.next_ino.to_string())?;
        fs::rename(&temp_path, &path)?;
        Ok(())
    }
    
    // Inode operations
    pub fn save_inode(&self, inode: &Inode) -> Result<()> {
        // Compute checksum before saving
        let mut inode_with_checksum = inode.clone();
        inode_with_checksum.checksum = Some(Self::compute_inode_checksum(inode));
        
        let path = self.pool_dir.join("inodes").join(inode.ino.to_string());
        let contents = serde_json::to_string_pretty(&inode_with_checksum)?;
        let temp_path = path.with_extension("tmp");
        
        #[cfg(test)]
        check_crash_point(CrashPoint::BeforeTempWrite)?;
        
        fs::write(&temp_path, &contents)?;
        
        #[cfg(test)]
        check_crash_point(CrashPoint::AfterTempWrite)?;
        
        // In production, we'd fsync here
        // For testing, we simulate the crash point
        #[cfg(test)]
        check_crash_point(CrashPoint::BeforeRename)?;
        
        fs::rename(&temp_path, &path)?;
        
        #[cfg(test)]
        check_crash_point(CrashPoint::AfterRename)?;

        // Also update persisted btree index
        let _ = self.inode_table.insert(inode.ino, inode_with_checksum);
        Ok(())
    }
    
    pub fn load_inode(&self, ino: u64) -> Result<Inode> {
        // Prefer file-based storage if present (so on-disk corruption is detectable);
        // fallback to btree index if file is missing.
        let path = self.pool_dir.join("inodes").join(ino.to_string());
        if path.exists() {
            let contents = fs::read_to_string(path)?;
            let inode: Inode = serde_json::from_str(&contents)?;
            // Verify checksum if present
            Self::verify_inode_checksum(&inode)
                .context(format!("Corrupted inode metadata for ino {}", ino))?;
            return Ok(inode);
        }

        // Fallback to btree index
        if let Some(inode) = self.inode_table.get(&ino) {
            Self::verify_inode_checksum(&inode).context(format!("Corrupted inode metadata for ino {}", ino))?;
            return Ok(inode);
        }

        Err(anyhow!("Inode {} not found", ino))
    }
    
    pub fn inode_exists(&self, ino: u64) -> bool {
        self.pool_dir.join("inodes").join(ino.to_string()).exists()
    }
    
    pub fn delete_inode(&self, ino: u64) -> Result<()> {
        let path = self.pool_dir.join("inodes").join(ino.to_string());
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
    
    pub fn list_directory(&self, parent_ino: u64) -> Result<Vec<Inode>> {
        let mut children = Vec::new();
        let inodes_dir = self.pool_dir.join("inodes");
        
        for entry in fs::read_dir(inodes_dir)? {
            let entry = entry?;
            if let Ok(contents) = fs::read_to_string(entry.path()) {
                if let Ok(inode) = serde_json::from_str::<Inode>(&contents) {
                    if inode.parent_ino == parent_ino {
                        children.push(inode);
                    }
                }
            }
        }
        
        Ok(children)
    }
    
    pub fn find_child(&self, parent_ino: u64, name: &str) -> Result<Option<Inode>> {
        for child in self.list_directory(parent_ino)? {
            if child.name == name {
                return Ok(Some(child));
            }
        }
        Ok(None)
    }
    
    // Extent operations
    pub fn save_extent(&self, extent: &Extent) -> Result<()> {
        let path = self.pool_dir.join("extents").join(extent.uuid.to_string());
        let contents = serde_json::to_string_pretty(extent)?;
        let temp_path = path.with_extension("tmp");
        
        #[cfg(test)]
        check_crash_point(CrashPoint::DuringExtentMetadata)?;
        
        fs::write(&temp_path, &contents)?;
        
        #[cfg(test)]
        check_crash_point(CrashPoint::AfterTempWrite)?;
        
        #[cfg(test)]
        check_crash_point(CrashPoint::BeforeRename)?;
        
        fs::rename(&temp_path, &path)?;
        Ok(())
    }
    
    pub fn load_extent(&self, uuid: &Uuid) -> Result<Extent> {
        let path = self.pool_dir.join("extents").join(uuid.to_string());
        let contents = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&contents)?)
    }
    
    pub fn delete_extent(&self, uuid: &Uuid) -> Result<()> {
        let path = self.pool_dir.join("extents").join(uuid.to_string());
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
    
    pub fn list_all_extents(&self) -> Result<Vec<Extent>> {
        let mut extents = Vec::new();
        let extents_dir = self.pool_dir.join("extents");
        
        for entry in fs::read_dir(extents_dir)? {
            let entry = entry?;
            if let Ok(contents) = fs::read_to_string(entry.path()) {
                if let Ok(extent) = serde_json::from_str::<Extent>(&contents) {
                    extents.push(extent);
                }
            }
        }
        
        Ok(extents)
    }
    
    // Extent map operations
    pub fn save_extent_map(&self, map: &ExtentMap) -> Result<()> {
        // Compute checksum before saving
        let mut map_with_checksum = map.clone();
        map_with_checksum.checksum = Some(Self::compute_extent_map_checksum(map));
        
        let path = self.pool_dir.join("extent_maps").join(map.ino.to_string());
        let contents = serde_json::to_string_pretty(&map_with_checksum)?;
        let temp_path = path.with_extension("tmp");
        
        #[cfg(test)]
        check_crash_point(CrashPoint::DuringExtentMap)?;
        
        fs::write(&temp_path, &contents)?;
        
        #[cfg(test)]
        check_crash_point(CrashPoint::AfterTempWrite)?;
        
        #[cfg(test)]
        check_crash_point(CrashPoint::BeforeRename)?;
        
        fs::rename(&temp_path, &path)?;
        
        // update persisted extent map table
        let _ = self.extent_map_table.insert(map.ino, map_with_checksum);
        Ok(())
    }
    
    pub fn load_extent_map(&self, ino: u64) -> Result<ExtentMap> {
        // Prefer file-based storage if present (so on-disk corruption is detectable);
        // fallback to btree index if file is missing.
        let path = self.pool_dir.join("extent_maps").join(ino.to_string());
        if path.exists() {
            let contents = fs::read_to_string(&path).unwrap_or_else(|_| {
                serde_json::to_string(&ExtentMap {
                    ino,
                    extents: Vec::new(),
                    checksum: None,
                })
                .unwrap()
            });
            let map: ExtentMap = serde_json::from_str(&contents)?;
            // Verify checksum if present
            Self::verify_extent_map_checksum(&map)
                .context(format!("Corrupted extent map metadata for ino {}", ino))?;
            return Ok(map);
        }

        // Fallback to btree index
        if let Some(map) = self.extent_map_table.get(&ino) {
            Self::verify_extent_map_checksum(&map).context(format!("Corrupted extent map metadata for ino {}", ino))?;
            return Ok(map);
        }

        // If neither exists, return an empty map
        Ok(ExtentMap { ino, extents: Vec::new(), checksum: None })
    }
    
    pub fn delete_extent_map(&self, ino: u64) -> Result<()> {
        let path = self.pool_dir.join("extent_maps").join(ino.to_string());
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
