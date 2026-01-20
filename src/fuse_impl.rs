use anyhow::Result;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyWrite, Request, TimeOrNow,
};
use libc::{EEXIST, ENOENT, ENOTDIR};
use std::ffi::OsStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::metadata::FileType as InodeFileType;
use crate::storage::StorageEngine;

pub struct DynamicFS {
    storage: StorageEngine,
}

impl DynamicFS {
    pub fn new(storage: StorageEngine) -> Self {
        DynamicFS { storage }
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
}
