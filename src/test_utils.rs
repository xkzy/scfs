use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;
// tempfile is a dev-dependency used only by tests
#[cfg(test)]
use tempfile::TempDir;
use std::path::PathBuf;

use crate::disk::Disk;
use crate::metadata::MetadataManager;

/// Run a closure and return Err if it doesn't complete within `secs` seconds.
/// Intended for use in tests to avoid hanging forever when something deadlocks.
pub fn run_with_timeout<F, T>(secs: u64, f: F) -> Result<T, &'static str>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = channel();
    thread::spawn(move || {
        let res = f();
        let _ = tx.send(res);
    });

    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(v) => Ok(v),
        Err(_) => Err("timed out"),
    }
}

/// Create a test environment: pool tempdir, disk tempdirs, metadata manager and Disk objects.
#[cfg(test)]
pub fn setup_test_env() -> (TempDir, Vec<TempDir>, MetadataManager, Vec<Disk>) {
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
