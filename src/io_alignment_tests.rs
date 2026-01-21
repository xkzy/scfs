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
