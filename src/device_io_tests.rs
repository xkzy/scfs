use anyhow::Result;
use tempfile::NamedTempFile;
use crate::device_io::Device;

#[test]
fn probe_and_open_file() -> Result<()> {
    let tmp = NamedTempFile::new()?;
    let path = tmp.path().to_path_buf();
    let bs = Device::probe_block_size(&path)?;
    assert!(bs >= 512);
    let dev = Device::open(path, true)?;
    assert!(dev.block_size >= 512);
    Ok(())
}