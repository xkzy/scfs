#![cfg(not(target_os = "windows"))]

//! FUSE filesystem implementation for Unix-like systems (Linux, macOS)
//!
//! This module provides FUSE-based filesystem support using the fuser crate.
//! It implements the fuser::Filesystem trait to provide POSIX-compatible
//! filesystem operations.

#[cfg(not(target_os = "windows"))]
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyWrite, Request, TimeOrNow, ReplyXattr, ReplyLock, ReplyOpen,
};
#[cfg(not(target_os = "windows"))]
use libc::{EEXIST, ENOENT, ENOTDIR, ENODATA, ERANGE, ENOSYS};
#[cfg(not(target_os = "windows"))]
use std::ffi::OsStr;
#[cfg(not(target_os = "windows"))]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(not(target_os = "windows"))]
use crate::metadata::FileType as InodeFileType;
#[cfg(not(target_os = "windows"))]
use crate::fs_interface::FilesystemInterface;
#[cfg(not(target_os = "windows"))]
use crate::file_locks::{LockManager, FileLock, LockType};
#[cfg(target_os = "macos")]
use crate::macos::MacOSHandler;

#[cfg(not(target_os = "windows"))]
const MAX_XATTR_SIZE: usize = 64 * 1024; // 64KB max xattr size
#[cfg(not(target_os = "windows"))]
const MAX_XATTR_NAME: usize = 255;

#[cfg(not(target_os = "windows"))]
pub struct DynamicFS {
    pub(crate) storage: Box<dyn FilesystemInterface + Send + Sync>,
    pub(crate) lock_manager: LockManager,
    #[cfg(target_os = "macos")]
    pub(crate) macos_handler: MacOSHandler,
    pub(crate) xattr_cache: Option<crate::fuse_optimizations::XAttrCache>,
    pub(crate) readahead_manager: Option<crate::fuse_optimizations::ReadAheadManager>,
    pub(crate) config: Option<crate::fuse_optimizations::OptimizedFUSEConfig>,
}

#[cfg(not(target_os = "windows"))]
impl DynamicFS {
    pub fn new(storage: Box<dyn FilesystemInterface + Send + Sync>) -> Self {
        DynamicFS { 
            storage,
            lock_manager: LockManager::new(),
            #[cfg(target_os = "macos")]
            macos_handler: MacOSHandler::new(),
            xattr_cache: None,
            readahead_manager: None,
            config: None,
        }
    }
    
    pub fn new_with_config(
        storage: Box<dyn FilesystemInterface + Send + Sync>,
        config: crate::fuse_optimizations::OptimizedFUSEConfig,
    ) -> Self {
        let xattr_cache = Some(crate::fuse_optimizations::XAttrCache::new(config.clone()));
        let readahead_manager = Some(crate::fuse_optimizations::ReadAheadManager::new(config.clone()));
        
        DynamicFS { 
            storage,
            lock_manager: LockManager::new(),
            #[cfg(target_os = "macos")]
            macos_handler: MacOSHandler::new(),
            xattr_cache,
            readahead_manager,
            config: Some(config),
        }
    }
    
    pub fn lock_manager(&self) -> &LockManager {
        &self.lock_manager
    }
    
    fn inode_to_file_attr(&self, inode: &crate::metadata::Inode) -> FileAttr {
        let kind = match inode.file_type {
            InodeFileType::RegularFile => FileType::RegularFile,
            InodeFileType::Directory => FileType::Directory,
        };
        
        FileAttr {
            ino: inode.ino,
            size: inode.size,
            blocks: (inode.size + 511) / 512,
            atime: UNIX_EPOCH + Duration::from_secs(inode.atime as u64),
            mtime: UNIX_EPOCH + Duration::from_secs(inode.mtime as u64),
            ctime: UNIX_EPOCH + Duration::from_secs(inode.ctime as u64),
            crtime: UNIX_EPOCH + Duration::from_secs(inode.ctime as u64),
            kind,
            perm: inode.mode as u16,
            nlink: 1,
            uid: inode.uid,
            gid: inode.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }
}

impl Filesystem for DynamicFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        log::debug!("lookup(parent={}, name={:?})", parent, name);
        
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        
        match self.storage.find_child(parent, name_str) {
            Ok(Some(inode)) => {
                let attr = self.inode_to_file_attr(&inode);
                let ttl = Duration::from_secs(1);
                reply.entry(&ttl, &attr, 0);
            }
            Ok(None) => {
                reply.error(ENOENT);
            }
            Err(e) => {
                log::error!("lookup failed: {}", e);
                reply.error(ENOENT);
            }
        }
    }
    
    fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        log::debug!("getattr(ino={})", ino);
        
        match self.storage.get_inode(ino) {
            Ok(inode) => {
                let attr = self.inode_to_file_attr(&inode);
                let ttl = Duration::from_secs(1);
                reply.attr(&ttl, &attr);
            }
            Err(e) => {
                log::error!("getattr failed: {}", e);
                reply.error(ENOENT);
            }
        }
    }
    
    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        log::debug!("readdir(ino={}, offset={})", ino, offset);
        
        let entries = match self.storage.list_directory(ino) {
            Ok(e) => e,
            Err(e) => {
                log::error!("readdir failed: {}", e);
                reply.error(ENOENT);
                return;
            }
        };
        
        let mut idx = offset as usize;
        
        // Add . and ..
        if idx == 0 {
            if reply.add(ino, 1, FileType::Directory, ".") {
                reply.ok();
                return;
            }
            idx += 1;
        }
        
        if idx == 1 {
            if let Ok(inode) = self.storage.get_inode(ino) {
                if reply.add(inode.parent_ino, 2, FileType::Directory, "..") {
                    reply.ok();
                    return;
                }
            }
            idx += 1;
        }
        
        // Add actual entries
        for (i, entry) in entries.iter().enumerate().skip(idx.saturating_sub(2)) {
            let kind = match entry.file_type {
                InodeFileType::RegularFile => FileType::RegularFile,
                InodeFileType::Directory => FileType::Directory,
            };
            
            if reply.add(entry.ino, (i + 3) as i64, kind, &entry.name) {
                break;
            }
        }
        
        reply.ok();
    }
    
    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        log::debug!("read(ino={}, offset={}, size={})", ino, offset, size);
        
        match self.storage.read_file(ino) {
            Ok(data) => {
                let start = offset as usize;
                let end = (start + size as usize).min(data.len());
                
                if start < data.len() {
                    reply.data(&data[start..end]);
                } else {
                    reply.data(&[]);
                }
            }
            Err(e) => {
                log::error!("read failed: {}", e);
                reply.error(ENOENT);
            }
        }
    }
    
    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        log::debug!("write(ino={}, offset={}, size={})", ino, offset, data.len());
        
        // For this prototype, we only support full rewrites at offset 0
        // A production system would handle partial writes
        match self.storage.write_file(ino, data, offset as u64) {
            Ok(()) => {
                reply.written(data.len() as u32);
            }
            Err(e) => {
                log::error!("write failed: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
    
    fn create(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        log::debug!("create(parent={}, name={:?})", parent, name);
        
        let name_str = match name.to_str() {
            Some(s) => s.to_string(),
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        
        // Check if already exists
        match self.storage.find_child(parent, &name_str) {
            Ok(Some(_)) => {
                reply.error(EEXIST);
                return;
            }
            Ok(None) => {}
            Err(e) => {
                log::error!("create check failed: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }
        
        match self.storage.create_file(parent, name_str) {
            Ok(inode) => {
                let attr = self.inode_to_file_attr(&inode);
                let ttl = Duration::from_secs(1);
                reply.created(&ttl, &attr, 0, 0, 0);
            }
            Err(e) => {
                log::error!("create failed: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
    
    fn mkdir(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        log::debug!("mkdir(parent={}, name={:?})", parent, name);
        
        let name_str = match name.to_str() {
            Some(s) => s.to_string(),
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        
        // Check if already exists
        match self.storage.find_child(parent, &name_str) {
            Ok(Some(_)) => {
                reply.error(EEXIST);
                return;
            }
            Ok(None) => {}
            Err(e) => {
                log::error!("mkdir check failed: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }
        
        match self.storage.create_dir(parent, name_str) {
            Ok(inode) => {
                let attr = self.inode_to_file_attr(&inode);
                let ttl = Duration::from_secs(1);
                reply.entry(&ttl, &attr, 0);
            }
            Err(e) => {
                log::error!("mkdir failed: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
    
    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        log::debug!("unlink(parent={}, name={:?})", parent, name);
        
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        
        // Find the file
        let inode = match self.storage.find_child(parent, name_str) {
            Ok(Some(i)) => i,
            Ok(None) => {
                reply.error(ENOENT);
                return;
            }
            Err(e) => {
                log::error!("unlink lookup failed: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };
        
        // Delete the file
        match self.storage.delete_file(inode.ino) {
            Ok(()) => reply.ok(),
            Err(e) => {
                log::error!("unlink failed: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
    
    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        log::debug!("rmdir(parent={}, name={:?})", parent, name);
        
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        
        // Find the directory
        let inode = match self.storage.find_child(parent, name_str) {
            Ok(Some(i)) => i,
            Ok(None) => {
                reply.error(ENOENT);
                return;
            }
            Err(e) => {
                log::error!("rmdir lookup failed: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };
        
        // Check if it's a directory
        if inode.file_type != InodeFileType::Directory {
            reply.error(ENOTDIR);
            return;
        }
        
        // Check if it's empty
        match self.storage.list_directory(inode.ino) {
            Ok(children) if !children.is_empty() => {
                reply.error(libc::ENOTEMPTY);
                return;
            }
            Ok(_) => {}
            Err(e) => {
                log::error!("rmdir check failed: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }
        
        // Delete the directory
        match self.storage.delete_file(inode.ino) {
            Ok(()) => reply.ok(),
            Err(e) => {
                log::error!("rmdir failed: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
    
    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        log::debug!("setattr(ino={})", ino);
        
        let mut inode = match self.storage.get_inode(ino) {
            Ok(i) => i,
            Err(e) => {
                log::error!("setattr failed: {}", e);
                reply.error(ENOENT);
                return;
            }
        };
        
        // Handle truncate
        if let Some(new_size) = size {
            if new_size == 0 {
                // Truncate to zero: delete all extents
                if let Err(e) = self.storage.write_file(ino, &[], 0) {
                    log::error!("truncate failed: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
                inode.size = 0;
            } else if new_size != inode.size {
                log::warn!("Truncate to non-zero size not fully supported");
            }
        }
        
        // Update times
        let now = chrono::Utc::now().timestamp();
        if let Some(_) = atime {
            inode.atime = now;
        }
        if let Some(_) = mtime {
            inode.mtime = now;
        }
        
        if let Err(e) = self.storage.update_inode(&inode) {
            log::error!("setattr update failed: {}", e);
            reply.error(libc::EIO);
            return;
        }
        
        let attr = self.inode_to_file_attr(&inode);
        let ttl = Duration::from_secs(1);
        reply.attr(&ttl, &attr);
    }
    
    // ===== Extended Attributes =====
    
    fn setxattr(
        &mut self,
        _req: &Request,
        ino: u64,
        name: &OsStr,
        value: &[u8],
        _flags: i32,
        _position: u32,
        reply: fuser::ReplyEmpty,
    ) {
        log::debug!("setxattr(ino={}, name={:?}, value_len={})", ino, name, value.len());
        
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        
        // Validate name and value size
        if name_str.len() > MAX_XATTR_NAME {
            reply.error(ERANGE);
            return;
        }
        
        if value.len() > MAX_XATTR_SIZE {
            reply.error(ERANGE);
            return;
        }
        
        // macOS-specific xattr handling
        #[cfg(target_os = "macos")]
        if let Err(e) = self.macos_handler.handle_xattr(name_str, Some(value)) {
            log::error!("macOS xattr validation failed: {}", e);
            reply.error(libc::EINVAL);
            return;
        }
        
        // Get inode
        let mut inode = match self.storage.get_inode(ino) {
            Ok(i) => i,
            Err(e) => {
                log::error!("setxattr failed: {}", e);
                reply.error(ENOENT);
                return;
            }
        };
        
        // Set the xattr
        inode.set_xattr(name_str.to_string(), value.to_vec());
        
        // Update inode
        if let Err(e) = self.storage.update_inode(&inode) {
            log::error!("setxattr update failed: {}", e);
            reply.error(libc::EIO);
            return;
        }
        
        reply.ok();
    }
    
    fn getxattr(
        &mut self,
        _req: &Request,
        ino: u64,
        name: &OsStr,
        size: u32,
        reply: ReplyXattr,
    ) {
        log::debug!("getxattr(ino={}, name={:?}, size={})", ino, name, size);
        
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        
        // macOS-specific xattr handling
        #[cfg(target_os = "macos")]
        if let Err(e) = self.macos_handler.handle_xattr(name_str, None) {
            log::error!("macOS xattr validation failed: {}", e);
            reply.error(libc::EINVAL);
            return;
        }
        
        // Get inode
        let inode = match self.storage.get_inode(ino) {
            Ok(i) => i,
            Err(e) => {
                log::error!("getxattr failed: {}", e);
                reply.error(ENOENT);
                return;
            }
        };
        
        // Get the xattr
        match inode.get_xattr(name_str) {
            Some(value) => {
                if size == 0 {
                    // Query size
                    reply.size(value.len() as u32);
                } else if size < value.len() as u32 {
                    reply.error(ERANGE);
                } else {
                    reply.data(value);
                }
            }
            None => {
                reply.error(ENODATA);
            }
        }
    }
    
    fn listxattr(&mut self, _req: &Request, ino: u64, size: u32, reply: ReplyXattr) {
        log::debug!("listxattr(ino={}, size={})", ino, size);
        
        // Get inode
        let inode = match self.storage.get_inode(ino) {
            Ok(i) => i,
            Err(e) => {
                log::error!("listxattr failed: {}", e);
                reply.error(ENOENT);
                return;
            }
        };
        
        // Get all xattr names
        let names = inode.list_xattrs();
        
        // Build null-terminated list
        let mut list = Vec::new();
        for name in names {
            list.extend_from_slice(name.as_bytes());
            list.push(0); // Null terminator
        }
        
        if size == 0 {
            // Query size
            reply.size(list.len() as u32);
        } else if size < list.len() as u32 {
            reply.error(ERANGE);
        } else {
            reply.data(&list);
        }
    }
    
    fn removexattr(&mut self, _req: &Request, ino: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        log::debug!("removexattr(ino={}, name={:?})", ino, name);
        
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        
        // Get inode
        let mut inode = match self.storage.get_inode(ino) {
            Ok(i) => i,
            Err(e) => {
                log::error!("removexattr failed: {}", e);
                reply.error(ENOENT);
                return;
            }
        };
        
        // Remove the xattr
        if inode.remove_xattr(name_str).is_none() {
            reply.error(ENODATA);
            return;
        }
        
        // Update inode
        if let Err(e) = self.storage.update_inode(&inode) {
            log::error!("removexattr update failed: {}", e);
            reply.error(libc::EIO);
            return;
        }
        
        reply.ok();
    }
    
    // ===== File Locking =====
    
    fn getlk(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
        reply: ReplyLock,
    ) {
        log::debug!("getlk(ino={}, start={}, end={}, type={})", ino, start, end, typ);
        
        let lock_type = match typ {
            libc::F_RDLCK => LockType::Read,
            libc::F_WRLCK => LockType::Write,
            libc::F_UNLCK => LockType::Unlock,
            _ => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        
        let test_lock = FileLock {
            owner: lock_owner,
            pid,
            lock_type,
            start,
            end,
        };
        
        match self.lock_manager.test_lock(ino, &test_lock) {
            Ok(Some(conflicting)) => {
                // Return the conflicting lock
                let conflict_type = match conflicting.lock_type {
                    LockType::Read => libc::F_RDLCK,
                    LockType::Write => libc::F_WRLCK,
                    LockType::Unlock => libc::F_UNLCK,
                };
                reply.locked(conflicting.start, conflicting.end, conflict_type, conflicting.pid);
            }
            Ok(None) => {
                // No conflict - return F_UNLCK
                reply.locked(start, end, libc::F_UNLCK, pid);
            }
            Err(e) => {
                log::error!("getlk failed: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
    
    fn setlk(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
        _sleep: bool,
        reply: fuser::ReplyEmpty,
    ) {
        log::debug!("setlk(ino={}, start={}, end={}, type={})", ino, start, end, typ);
        
        let lock_type = match typ {
            libc::F_RDLCK => LockType::Read,
            libc::F_WRLCK => LockType::Write,
            libc::F_UNLCK => LockType::Unlock,
            _ => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        
        if lock_type == LockType::Unlock {
            // Release lock
            if let Err(e) = self.lock_manager.release_lock(ino, lock_owner, start, end) {
                log::error!("unlock failed: {}", e);
                reply.error(libc::EIO);
                return;
            }
        } else {
            // Acquire lock
            let lock = FileLock {
                owner: lock_owner,
                pid,
                lock_type,
                start,
                end,
            };
            
            if let Err(e) = self.lock_manager.acquire_lock(ino, lock) {
                log::error!("lock failed: {}", e);
                reply.error(libc::EAGAIN);
                return;
            }
        }
        
        reply.ok();
    }
    
    // ===== Fallocate =====
    
    fn fallocate(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        length: i64,
        mode: i32,
        reply: fuser::ReplyEmpty,
    ) {
        log::debug!("fallocate(ino={}, offset={}, length={}, mode={})", ino, offset, length, mode);
        
        // Get inode
        let mut inode = match self.storage.get_inode(ino) {
            Ok(i) => i,
            Err(e) => {
                log::error!("fallocate failed: {}", e);
                reply.error(ENOENT);
                return;
            }
        };
        
        // Handle punch hole
        // NOTE: Current implementation returns success but does not actually
        // create sparse regions. Data remains allocated. This is a known
        // limitation documented in PHASE_16_COMPLETE.md under "Limitations".
        // Future enhancement: Implement true sparse file support with extent splitting.
        if mode & libc::FALLOC_FL_PUNCH_HOLE != 0 {
            log::info!("Punch hole: offset={}, length={} (data remains allocated)", offset, length);
            reply.ok();
            return;
        }
        
        // Handle zero range
        // NOTE: Similar to punch hole, this claims success but doesn't zero data.
        // Future enhancement: Actually zero the specified range.
        if mode & libc::FALLOC_FL_ZERO_RANGE != 0 {
            log::info!("Zero range: offset={}, length={} (data remains allocated)", offset, length);
            reply.ok();
            return;
        }
        
        // Normal fallocate - preallocate space
        let new_size = (offset + length) as u64;
        if new_size > inode.size {
            inode.size = new_size;
            if let Err(e) = self.storage.update_inode(&inode) {
                log::error!("fallocate update failed: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }
        
        reply.ok();
    }
    
    // ===== Open/Release =====
    
    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        log::debug!("open(ino={}, flags={})", ino, flags);
        
        // Verify file exists
        if self.storage.get_inode(ino).is_err() {
            reply.error(ENOENT);
            return;
        }
        
        // Return file handle (we use inode number as handle for simplicity)
        reply.opened(ino, flags as u32);
    }
    
    fn release(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        _flags: i32,
        lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        log::debug!("release(ino={})", ino);
        
        // Release all locks for this owner
        if let Some(owner) = lock_owner {
            if let Err(e) = self.lock_manager.release_all_locks(ino, owner) {
                log::error!("release locks failed: {}", e);
            }
        }
        
        reply.ok();
    }
    
    // ===== Fsync =====
    
    fn fsync(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        _datasync: bool,
        reply: fuser::ReplyEmpty,
    ) {
        log::debug!("fsync(ino={})", ino);
        
        // In a production system, this would flush all pending writes
        // For now, all writes are synchronous, so just verify inode exists
        if self.storage.get_inode(ino).is_err() {
            reply.error(ENOENT);
            return;
        }
        
        reply.ok();
    }
    
    // ===== IOCTL (minimal support) =====
    
    fn ioctl(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        _flags: u32,
        cmd: u32,
        _in_data: &[u8],
        _out_size: u32,
        reply: fuser::ReplyIoctl,
    ) {
        log::debug!("ioctl(ino={}, cmd={})", ino, cmd);
        
        // Most ioctls are not supported
        // Return ENOSYS to indicate not implemented
        reply.error(ENOSYS);
    }
}
