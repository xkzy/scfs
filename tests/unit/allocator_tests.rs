use anyhow::Result;
use tempfile::tempdir;
use crate::allocator::BitmapAllocator;

#[test]
fn allocator_basic_ops() -> Result<()> {
    let td = tempdir()?;
    let path = td.path().join("alloc.bin");
    let alloc = BitmapAllocator::new(1024*1024, 16, Some(path.clone()))?;
    assert_eq!(alloc.free_count(), 16);
    let s = alloc.allocate_contiguous(3).expect("alloc 3");
    assert_eq!(alloc.free_count(), 13);
    alloc.free_contiguous(s, 3)?;
    assert_eq!(alloc.free_count(), 16);
    // persist exists
    let re = BitmapAllocator::load_from_path(&path)?;
    assert_eq!(re.free_count(), 16);
    Ok(())
}