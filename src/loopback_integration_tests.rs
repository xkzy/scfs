#![cfg(unix)]

use anyhow::Result;
use std::process::Command;
use std::fs::OpenOptions;
use tempfile::NamedTempFile;

use crate::io_alignment::{write_aligned_file, read_aligned_file};
use crate::on_device_allocator::{OnDeviceAllocator, FragmentHeader};
use crate::crash_sim::{get_crash_simulator, CrashPoint};
use uuid::Uuid;
use blake3;

fn has_losetup() -> bool {
    which::which("losetup").is_ok()
}

fn is_root() -> bool {
    nix::unistd::Uid::effective().is_root()
}

#[test]
fn loopback_device_direct_io() -> Result<()> {
    // Skip test unless losetup is present and we're root, to avoid flakiness on CI
    if !has_losetup() || !is_root() {
        eprintln!("skipping loopback integration test: requires losetup/root");
        return Ok(());
    }

    // Create backing file and attach loop device
    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();
    let size: u64 = 10 * 1024 * 1024; // 10 MB
    let f = OpenOptions::new().write(true).open(&path)?;
    f.set_len(size)?;
    drop(f);

    // losetup --find --show <path>
    let out = Command::new("losetup").arg("--find").arg("--show").arg(&path).output()?;
    if !out.status.success() {
        eprintln!("losetup failed: {}", String::from_utf8_lossy(&out.stderr));
        return Ok(());
    }
    let dev = String::from_utf8_lossy(&out.stdout).trim().to_string();

    // Ensure we detach at the end
    let result = (|| -> Result<()> {
        // Try an aligned write to the device (prefer direct)
        let data = vec![0x5Au8; 8192]; // 8KiB
        write_aligned_file(std::path::Path::new(&dev), &data, true)?;
        let read_back = read_aligned_file(std::path::Path::new(&dev), data.len(), true)?;
        assert_eq!(read_back[..data.len()], data);
        Ok(())
    })();

    // Detach
    let _ = Command::new("losetup").arg("-d").arg(&dev).status();

    result
}

#[test]
fn crash_simulation_fragment_write_before_data() -> Result<()> {
    // Skip test unless losetup is present and we're root
    if !has_losetup() || !is_root() {
        eprintln!("skipping crash integration test: requires losetup/root");
        return Ok(());
    }

    // Create backing file and attach loop device
    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();
    let size: u64 = 50 * 1024 * 1024; // 50 MB
    let f = OpenOptions::new().write(true).open(&path)?;
    f.set_len(size)?;
    drop(f);

    // losetup --find --show <path>
    let out = Command::new("losetup").arg("--find").arg("--show").arg(&path).output()?;
    if !out.status.success() {
        eprintln!("losetup failed: {}", String::from_utf8_lossy(&out.stderr));
        return Ok(());
    }
    let dev = String::from_utf8_lossy(&out.stdout).trim().to_string();

    // Ensure we detach at the end
    let result = (|| -> Result<()> {
        // Initialize on-device allocator
        let mut oda = OnDeviceAllocator::new(&dev, size)?;
        oda.initialize()?;

        // Enable crash simulation before fragment data write
        let sim = get_crash_simulator();
        sim.enable_at(CrashPoint::BeforeFragmentDataWrite);

        // Try to write a fragment - should crash
        let test_data = vec![0x42u8; 8192];
        let fragment_id = Uuid::new_v4();
        let result = oda.write_fragment(fragment_id, &test_data);

        // Should fail due to simulated crash
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("SIMULATED POWER LOSS"));

        sim.disable();

        // Reconcile should find no valid fragments (crash happened before data write)
        let changed = oda.reconcile_and_persist()?;
        assert!(!changed, "Bitmap should not change - no valid fragments written");

        Ok(())
    })();

    // Detach
    let _ = Command::new("losetup").arg("-d").arg(&dev).status();

    result
}

#[test]
fn crash_simulation_fragment_write_after_data_before_fsync() -> Result<()> {
    // Skip test unless losetup is present and we're root
    if !has_losetup() || !is_root() {
        eprintln!("skipping crash integration test: requires losetup/root");
        return Ok(());
    }

    // Create backing file and attach loop device
    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();
    let size: u64 = 50 * 1024 * 1024; // 50 MB
    let f = OpenOptions::new().write(true).open(&path)?;
    f.set_len(size)?;
    drop(f);

    // losetup --find --show <path>
    let out = Command::new("losetup").arg("--find").arg("--show").arg(&path).output()?;
    if !out.status.success() {
        eprintln!("losetup failed: {}", String::from_utf8_lossy(&out.stderr));
        return Ok(());
    }
    let dev = String::from_utf8_lossy(&out.stdout).trim().to_string();

    // Ensure we detach at the end
    let result = (|| -> Result<()> {
        // Initialize on-device allocator
        let mut oda = OnDeviceAllocator::new(&dev, size)?;
        oda.initialize()?;

        // Enable crash simulation after data write but before fdatasync
        let sim = get_crash_simulator();
        sim.enable_at(CrashPoint::AfterFragmentDataWrite);

        // Try to write a fragment - should crash after data write
        let test_data = vec![0x42u8; 8192];
        let fragment_id = Uuid::new_v4();
        let result = oda.write_fragment(fragment_id, &test_data);

        // Should fail due to simulated crash
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("SIMULATED POWER LOSS"));

        sim.disable();

        // Reconcile should find the fragment and mark it as used
        let changed = oda.reconcile_and_persist()?;
        assert!(changed, "Bitmap should be updated - valid fragment found");

        // Verify the fragment can be read back
        let read_data = oda.read_fragment(fragment_id)?;
        assert_eq!(read_data, test_data);

        Ok(())
    })();

    // Detach
    let _ = Command::new("losetup").arg("-d").arg(&dev).status();

    result
}

#[test]
fn crash_simulation_fragment_write_after_fsync_before_allocator_persist() -> Result<()> {
    // Skip test unless losetup is present and we're root
    if !has_losetup() || !is_root() {
        eprintln!("skipping crash integration test: requires losetup/root");
        return Ok(());
    }

    // Create backing file and attach loop device
    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();
    let size: u64 = 50 * 1024 * 1024; // 50 MB
    let f = OpenOptions::new().write(true).open(&path)?;
    f.set_len(size)?;
    drop(f);

    // losetup --find --show <path>
    let out = Command::new("losetup").arg("--find").arg("--show").arg(&path).output()?;
    if !out.status.success() {
        eprintln!("losetup failed: {}", String::from_utf8_lossy(&out.stderr));
        return Ok(());
    }
    let dev = String::from_utf8_lossy(&out.stdout).trim().to_string();

    // Ensure we detach at the end
    let result = (|| -> Result<()> {
        // Initialize on-device allocator
        let mut oda = OnDeviceAllocator::new(&dev, size)?;
        oda.initialize()?;

        // Enable crash simulation after fdatasync but before allocator persist
        let sim = get_crash_simulator();
        sim.enable_at(CrashPoint::AfterFragmentFsync);

        // Try to write a fragment - should crash during allocator update
        let test_data = vec![0x42u8; 8192];
        let fragment_id = Uuid::new_v4();
        let result = oda.write_fragment(fragment_id, &test_data);

        // Should fail due to simulated crash
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("SIMULATED POWER LOSS"));

        sim.disable();

        // Reconcile should find the fragment and mark it as used
        let changed = oda.reconcile_and_persist()?;
        assert!(changed, "Bitmap should be updated - valid fragment found");

        // Verify the fragment can be read back
        let read_data = oda.read_fragment(fragment_id)?;
        assert_eq!(read_data, test_data);

        Ok(())
    })();

    // Detach
    let _ = Command::new("losetup").arg("-d").arg(&dev).status();

    result
}