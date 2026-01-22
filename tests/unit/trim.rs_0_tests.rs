// moved from src/trim.rs
use super::*;

    #[test]
    fn test_trim_intensity() {
        assert_eq!(
            TrimIntensity::Conservative.batch_threshold_bytes(),
            10 * 1024 * 1024 * 1024
        );
        assert_eq!(
            TrimIntensity::Balanced.batch_threshold_bytes(),
            1024 * 1024 * 1024
        );
        assert_eq!(
            TrimIntensity::Aggressive.batch_threshold_bytes(),
            10 * 1024 * 1024
        );
    }

    #[test]
    fn test_trim_config_default() {
        let config = TrimConfig::default();
        assert!(config.enabled);
        assert_eq!(config.intensity, TrimIntensity::Balanced);
        assert_eq!(config.discard_granularity, 4096);
    }

    #[test]
    fn test_trim_stats() {
        let engine = TrimEngine::new(TrimConfig::default());
        let stats = engine.stats();
        
        assert_eq!(stats.total_trim_operations, 0);
        assert_eq!(stats.total_bytes_trimmed, 0);
        assert_eq!(stats.pending_bytes, 0);
    }
