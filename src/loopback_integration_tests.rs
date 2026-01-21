#![cfg(unix)]

use anyhow::Result;
use std::process::Command;
use std::fs::OpenOptions;
use tempfile::NamedTempFile;

use crate::io_alignment::{write_aligned_file, read_aligned_file};

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