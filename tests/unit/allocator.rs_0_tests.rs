// moved from src/allocator.rs
use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_and_persist_allocator() -> Result<()> {
        let td = tempdir()?;
        let path = td.path().join("allocator.bin");
        let unit_size = 1024u64 * 1024u64; // 1 MiB
        let total_units = 10u64;
        let alloc = BitmapAllocator::new(unit_size, total_units, Some(path.clone()))?;
        assert_eq!(alloc.free_count(), 10);
        let s = alloc.allocate_contiguous(2).expect("alloc 2 units");
        assert_eq!(alloc.free_count(), 8);
        // reload
        let re = BitmapAllocator::load_from_path(&path)?;
        assert_eq!(re.free_count(), 8);
        re.free_contiguous(s, 2)?;
        assert_eq!(re.free_count(), 10);
        Ok(())
    }

    #[test]
    fn find_contiguous() -> Result<()> {
        let alloc = BitmapAllocator::new(4096, 100, None)?;
        // occupy some units
        let _ = alloc.allocate_contiguous(3).unwrap();
        let _ = alloc.allocate_contiguous(4).unwrap();
        let start = alloc.find_contiguous(10).unwrap();
        assert!(start >= 0);
        Ok(())
    }
