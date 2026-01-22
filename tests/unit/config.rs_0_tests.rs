// moved from src/config.rs
use super::*;

    #[test]
    fn test_production_config() {
        let config = Config::production();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_development_config() {
        let config = Config::development();
        assert!(config.validate().is_ok());
        assert!(!config.performance.enable_write_batching);
    }

    #[test]
    fn test_testing_config() {
        let config = Config::testing();
        assert!(config.validate().is_ok());
        assert_eq!(config.storage.default_extent_size, 1024);
    }

    #[test]
    fn test_high_performance_config() {
        let config = Config::high_performance();
        assert!(config.validate().is_ok());
        assert!(config.performance.max_parallel_writes > Config::production().performance.max_parallel_writes);
    }

    #[test]
    fn test_config_json_roundtrip() {
        let config = Config::production();
        let json = config.to_json();
        let restored = Config::from_json(&json).unwrap();
        
        assert_eq!(config.storage.default_extent_size, restored.storage.default_extent_size);
    }

    #[test]
    fn test_config_builder() {
        let config = ConfigBuilder::new()
            .extent_size(8192)
            .enable_write_batching(false)
            .build()
            .unwrap();

        assert_eq!(config.storage.default_extent_size, 8192);
        assert!(!config.performance.enable_write_batching);
    }

    #[test]
    fn test_config_validation() {
        let mut config = Config::production();
        config.storage.default_extent_size = 0;
        
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_merge() {
        let config1 = Config::production();
        let config2 = Config::development();
        
        let merged = config1.merge(&config2);
        assert_eq!(merged.performance.enable_write_batching, config2.performance.enable_write_batching);
    }
