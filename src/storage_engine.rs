//! Storage Engine Module
//!
//! This module re-exports the cross-platform filesystem interface and utilities.
//! The actual implementations are in dedicated modules for better organization.

pub use crate::fs_interface::{FilesystemInterface, FilesystemStats};
pub use crate::path_utils;
pub use crate::mount;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageEngine;
    use crate::metadata::MetadataManager;
    use crate::disk::Disk;
    use tempfile::TempDir;

    fn setup_test_storage() -> (TempDir, Vec<TempDir>, Box<dyn FilesystemInterface + Send + Sync>) {
        let pool_dir = tempfile::tempdir().unwrap();

        // Create test disks
        let disk_dirs: Vec<TempDir> = (0..3)
            .map(|_| tempfile::tempdir().unwrap())
            .collect();

        let disks: Vec<Disk> = disk_dirs
            .iter()
            .map(|td| Disk::new(td.path().to_path_buf()).unwrap())
            .collect();

        let metadata = MetadataManager::new(pool_dir.path().to_path_buf()).unwrap();
        let storage = StorageEngine::new(metadata, disks);

        (pool_dir, disk_dirs, Box::new(storage))
    }

    #[test]
    fn test_filesystem_interface_basic_operations() {
        let (_pool_dir, _disk_dirs, storage) = setup_test_storage();

        // Create a directory
        let root_dir = storage.create_dir(1, "test_dir".to_string()).unwrap();
        assert_eq!(root_dir.name, "test_dir");

        // Create a file in the directory
        let file = storage.create_file(root_dir.ino, "test.txt".to_string()).unwrap();
        assert_eq!(file.name, "test.txt");

        // Write to the file
        let data = b"Hello, World!";
        storage.write_file(file.ino, data, 0).unwrap();

        // Read from the file
        let read_data = storage.read_file(file.ino).unwrap();
        assert_eq!(read_data, data);

        // Find the file by name
        let found = storage.find_child(root_dir.ino, "test.txt").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().ino, file.ino);

        // List directory
        let entries = storage.list_directory(root_dir.ino).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "test.txt");

        // Get inode
        let inode = storage.get_inode(file.ino).unwrap();
        assert_eq!(inode.ino, file.ino);
        assert_eq!(inode.size, data.len() as u64);

        // Delete the file
        storage.delete_file(file.ino).unwrap();

        // Verify file is gone
        let found_after_delete = storage.find_child(root_dir.ino, "test.txt").unwrap();
        assert!(found_after_delete.is_none());
    }

    #[test]
    fn test_filesystem_stats() {
        let (_pool_dir, _disk_dirs, storage) = setup_test_storage();

        // Create some files and directories
        let dir1 = storage.create_dir(1, "dir1".to_string()).unwrap();
        let _dir2 = storage.create_dir(1, "dir2".to_string()).unwrap();

        let file1 = storage.create_file(dir1.ino, "file1.txt".to_string()).unwrap();
        let file2 = storage.create_file(dir1.ino, "file2.txt".to_string()).unwrap();

        let data1 = b"Short";
        let data2 = b"Longer content here";

        storage.write_file(file1.ino, data1, 0).unwrap();
        storage.write_file(file2.ino, data2, 0).unwrap();

        // Get stats
        let stats = storage.stat().unwrap();

        // We created 2 directories (dir1, dir2) plus the root directory exists (ino=1)
        assert!(stats.total_dirs >= 2);
        // total_files and total_size should be non-zero after writes
        assert!(stats.total_files >= 2); // Created file1.txt and file2.txt
        assert!(stats.total_size >= data1.len() as u64 + data2.len() as u64);
        assert!(stats.free_space > 0);
    }
}