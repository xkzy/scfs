#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    fn setup_test_env() -> (TempDir, Vec<TempDir>, MetadataManager, Vec<Disk>) {
        let pool_dir = tempfile::tempdir().unwrap();
        
        // Create 6 test disks
        let disk_dirs: Vec<TempDir> = (0..6)
            .map(|_| tempfile::tempdir().unwrap())
            .collect();
        
        let disks: Vec<Disk> = disk_dirs
            .iter()
            .map(|td| Disk::new(td.path().to_path_buf()).unwrap())
            .collect();
        
        let metadata = MetadataManager::new(pool_dir.path().to_path_buf()).unwrap();
        
        (pool_dir, disk_dirs, metadata, disks)
    }
    
    #[test]
    fn test_write_and_read_small_file() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "test.txt".to_string()).unwrap();
        
        // Write data
        let data = b"Hello, World!";
        storage.write_file(inode.ino, data, 0).unwrap();
        
        // Read data
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_write_and_read_large_file() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a file
        let inode = storage.create_file(1, "large.bin".to_string()).unwrap();
        
        // Write 5MB of data
        let data = vec![0x42u8; 5 * 1024 * 1024];
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Read data
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data.len(), data.len());
        assert_eq!(read_data, data);
    }
    
    #[test]
    fn test_multiple_files() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create multiple files
        for i in 0..10 {
            let name = format!("file{}.txt", i);
            let inode = storage.create_file(1, name).unwrap();
            let data = format!("Content of file {}", i);
            storage.write_file(inode.ino, data.as_bytes(), 0).unwrap();
        }
        
        // List directory
        let children = storage.list_directory(1).unwrap();
        assert_eq!(children.len(), 10);
        
        // Read each file
        for child in children {
            let data = storage.read_file(child.ino).unwrap();
            assert!(data.len() > 0);
        }
    }
    
    #[test]
    fn test_disk_failure_recovery() {
        let (_pool_dir, _disk_dirs, metadata, mut disks) = setup_test_env();
        
        // Write a file using EC(4+2)
        let storage = StorageEngine::new(metadata, disks.clone());
        let inode = storage.create_file(1, "resilient.txt".to_string()).unwrap();
        let data = vec![0xAAu8; 2 * 1024 * 1024]; // 2MB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Simulate 2 disk failures (EC can handle this)
        disks[0].mark_failed().unwrap();
        disks[3].mark_failed().unwrap();
        
        // Should still be able to read
        let storage = StorageEngine::new(
            MetadataManager::new(_pool_dir.path().to_path_buf()).unwrap(),
            disks
        );
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data.len(), data.len());
    }
    
    #[test]
    fn test_directory_operations() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create a subdirectory
        let subdir = storage.create_dir(1, "subdir".to_string()).unwrap();
        
        // Create files in subdirectory
        let file1 = storage.create_file(subdir.ino, "file1.txt".to_string()).unwrap();
        let file2 = storage.create_file(subdir.ino, "file2.txt".to_string()).unwrap();
        
        // List subdirectory
        let children = storage.list_directory(subdir.ino).unwrap();
        assert_eq!(children.len(), 2);
        
        // Write and read from files
        storage.write_file(file1.ino, b"File 1 content", 0).unwrap();
        storage.write_file(file2.ino, b"File 2 content", 0).unwrap();
        
        let data1 = storage.read_file(file1.ino).unwrap();
        let data2 = storage.read_file(file2.ino).unwrap();
        
        assert_eq!(data1, b"File 1 content");
        assert_eq!(data2, b"File 2 content");
    }
    
    #[test]
    fn test_delete_file() {
        let (_pool_dir, _disk_dirs, metadata, disks) = setup_test_env();
        let storage = StorageEngine::new(metadata, disks);
        
        // Create and write file
        let inode = storage.create_file(1, "temp.txt".to_string()).unwrap();
        storage.write_file(inode.ino, b"Temporary data", 0).unwrap();
        
        // Verify file exists
        let children = storage.list_directory(1).unwrap();
        assert_eq!(children.len(), 1);
        
        // Delete file
        storage.delete_file(inode.ino).unwrap();
        
        // Verify file is gone
        let children = storage.list_directory(1).unwrap();
        assert_eq!(children.len(), 0);
    }
    
    #[test]
    fn test_extent_rebuild() {
        let (_pool_dir, _disk_dirs, metadata, mut disks) = setup_test_env();
        
        // Write a file
        let storage = StorageEngine::new(metadata, disks.clone());
        let inode = storage.create_file(1, "rebuild_test.txt".to_string()).unwrap();
        let data = vec![0x55u8; 3 * 1024 * 1024]; // 3MB
        storage.write_file(inode.ino, &data, 0).unwrap();
        
        // Verify all extents are complete
        let extents = storage.list_all_extents().unwrap();
        for extent in &extents {
            assert!(extent.is_complete());
        }
        
        // Fail one disk
        disks[1].mark_failed().unwrap();
        
        // Read should trigger rebuild
        let storage = StorageEngine::new(
            MetadataManager::new(_pool_dir.path().to_path_buf()).unwrap(),
            disks
        );
        let read_data = storage.read_file(inode.ino).unwrap();
        assert_eq!(read_data, data);
        
        // Verify extents were rebuilt
        let extents = storage.list_all_extents().unwrap();
        for extent in &extents {
            assert!(extent.is_readable());
        }
    }
}
