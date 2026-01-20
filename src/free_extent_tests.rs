use anyhow::Result;
use tempfile::tempdir;
use crate::free_extent::FreeExtentIndex;

#[test]
fn insert_and_allocate_best_fit() -> Result<()> {
    let td = tempdir()?;
    let path = td.path().join("freeext.bin");
    let mut idx = FreeExtentIndex::new(Some(path.clone()))?;
    idx.insert_run(0, 10)?; // units 0..10
    idx.insert_run(20, 10)?; // units 20..30
    // allocate best-fit for 4 -> should pick run 0 (size 10) as smallest >= 4
    let s = idx.allocate_best_fit(4).expect("alloc 4");
    assert!(s == 0 || s == 20);
    // after allocation, remaining runs should reflect split
    let runs = idx.list_runs();
    assert!(runs.iter().any(|(st,len)| *len >= 6));
    Ok(())
}

#[test]
fn consume_range_and_merge() -> Result<()> {
    let td = tempdir()?;
    let path = td.path().join("freeext2.bin");
    let mut idx = FreeExtentIndex::new(Some(path.clone()))?;
    idx.insert_run(0, 5)?;
    idx.insert_run(10, 5)?;
    idx.insert_run(5, 5)?; // should merge into 0..15
    let runs = idx.list_runs();
    assert!(runs.len() == 1);
    // consume middle part 4..6
    idx.consume_range(4, 2)?;
    let runs2 = idx.list_runs();
    assert!(runs2.len() == 2);
    Ok(())
}