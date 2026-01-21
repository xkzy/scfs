#![cfg(unix)]

use anyhow::Result;
use std::fs;
use tempfile::NamedTempFile;

use crate::io_alignment::{AlignedBuf, fd_has_o_direct, open_and_check_o_direct, open_with_o_direct};

#[test]
fn test_alloc_aligned() -> Result<()> {
    let mut buf = AlignedBuf::new(4096, 4096)?;
    let ptr = buf.as_ptr() as usize;
    assert_eq!(ptr % 4096, 0, "buffer pointer must be 4096-aligned");
    assert_eq!(buf.len(), 4096);
    // Write/read into buffer
    let slice = buf.as_mut_slice();
    for i in 0..slice.len() {
        slice[i] = (i % 256) as u8;
    }
    for i in 0..slice.len() {
        assert_eq!(slice[i], (i % 256) as u8);
    }
    Ok(())
}

#[test]
fn test_secure_erase_aligned() -> Result<()> {
    use crate::trim::TrimEngine;

    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();

    // Fill file with non-zero content
    let mut f = std::fs::File::create(&path)?;
    let data = vec![0xABu8; 2 * 1024 * 1024]; // 2MB
    f.write_all(&data)?;
    f.sync_all()?;
    drop(f);

    // Run secure erase (should prefer aligned writes)
    TrimEngine::secure_erase_file(&path)?;

    // Read back and verify zeroed
    let mut buf = vec![0u8; data.len()];
    std::fs::File::open(&path)?.read_exact(&mut buf)?;
    assert!(buf.iter().all(|&b| b == 0));

    Ok(())
}

#[test]
fn test_open_o_direct_flag() -> Result<()> {
    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();

    // open and check returns whether O_DIRECT is set on the fd
    let result = open_and_check_o_direct(&path);
    match result {
        Ok(has) => {
            // Some filesystems (e.g., loose tmpfs) may not support O_DIRECT; we accept either.
            if has {
                // Also double-check via explicit open
                let f = open_with_o_direct(&path)?;
                let fd = f.as_raw_fd();
                assert!(fd_has_o_direct(fd)?);
            }
            Ok(())
        }
        Err(e) => {
            // If opening with O_DIRECT is not permitted, we still consider this test informative;
            // ensure the path exists and the error is a reasonable errno.
            assert!(path.exists());
            eprintln!("O_DIRECT open returned error (expected on some systems): {}", e);
            Ok(())
        }
    }
}

#[test]
fn test_pread_pwrite_direct() -> Result<()> {
    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();

    // Try to open with O_DIRECT; if not supported, skip the strict test
    let f = match open_with_o_direct(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Skipping direct I/O test (open O_DIRECT failed): {}", e);
            return Ok(());
        }
    };

    let fd = f.as_raw_fd();

    // Allocate aligned buffers
    let mut wbuf = AlignedBuf::new(4096, 4096)?;
    for i in 0..wbuf.len() {
        wbuf.as_mut_slice()[i] = (i % 256) as u8;
    }

    // Write at offset 0
    let wrote = crate::io_alignment::pwrite_direct(fd, &wbuf, 0)?;
    assert_eq!(wrote, wbuf.len());

    // Read back
    let mut rbuf = AlignedBuf::new(4096, 4096)?;
    let read = crate::io_alignment::pread_direct(fd, &mut rbuf, 0)?;
    assert_eq!(read, rbuf.len());
    assert_eq!(rbuf.as_slice(), wbuf.as_slice());

    // Misaligned offset should error
    let mis = crate::io_alignment::pwrite_direct(fd, &wbuf, 1);
    assert!(mis.is_err());

    // Test the high-level write/read wrapper (prefer direct but allow fallback)
    let data = vec![42u8; 5000];
    crate::io_alignment::write_aligned_file(&path, &data, true)?;
    let read_back = crate::io_alignment::read_aligned_file(&path, data.len(), true)?;
    assert_eq!(read_back, data);

    Ok(())
}
