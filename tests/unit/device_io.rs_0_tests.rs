// moved from src/device_io.rs
use super::*;
    use tempfile::NamedTempFile;
    use anyhow::Result;

    #[test]
    fn align_up_works() {
        assert_eq!(Device::align_up(1000, 4096), 4096);
        assert_eq!(Device::align_up(4096, 4096), 4096);
        assert_eq!(Device::align_up(4097, 4096), 8192);
        assert_eq!(Device::align_up(0, 4096), 0);
    }

    #[test]
    fn is_aligned_works() {
        assert!(Device::is_aligned(0, 512));
        assert!(Device::is_aligned(4096, 4096));
        assert!(!Device::is_aligned(1000, 4096));
    }

    #[test]
    fn probe_block_size_default_on_regular_file() -> Result<()> {
        let tmp = NamedTempFile::new()?;
        let path = tmp.path().to_path_buf();
        // Regular files won't support BLKSSZGET; method should return default
        let sz = Device::probe_block_size(&path)?;
        assert!(sz >= 512);
        Ok(())
    }

    #[test]
    fn open_fallback_without_direct() -> Result<()> {
        let tmp = NamedTempFile::new()?;
        let path = tmp.path().to_path_buf();

        let dev = Device::open(path, true)?;
        // On regular files, O_DIRECT likely failed; ensure fallback worked
        assert!(dev.block_size >= 512);
        Ok(())
    }
