// moved from src/adaptive.rs
use super::*;

    #[test]
    fn test_sequence_detector() {
        let mut detector = SequenceDetector::new(1024, 10);
        
        // Record sequential accesses
        detector.record_access(0);
        detector.record_access(512);
        detector.record_access(1024);
        
        assert!(detector.is_sequential());
        assert!(detector.recommended_readahead() > 0);
    }

    #[test]
    fn test_dynamic_extent_sizer() {
        let mut sizer = DynamicExtentSizer::new(4096, 65536);
        let initial_size = sizer.recommended_size();
        
        // Record sequential pattern
        for _ in 0..7 {
            sizer.observe_pattern(AccessPattern::Sequential);
        }
        
        // Size should increase for sequential
        let new_size = sizer.recommended_size();
        assert!(new_size >= initial_size);
    }

    #[test]
    fn test_workload_cache() {
        let mut cache = WorkloadCache::new(10);
        let uuid1 = Uuid::new_v4();
        let uuid2 = Uuid::new_v4();
        let uuid3 = Uuid::new_v4();
        
        // Access uuid1 many times (hot)
        for _ in 0..20 {
            cache.record_access(uuid1);
        }
        
        // Access uuid2 medium times (medium)
        for _ in 0..5 {
            cache.record_access(uuid2);
        }
        
        // Access uuid3 few times (cold)
        cache.record_access(uuid3);
        
        cache.update_classifications();
        
        // With counts [1, 5, 20], median is 5
        // uuid1 (20) > 5*2 (10) = true -> hot
        // uuid3 (1) < 5/2 (2) = true -> cold
        assert!(cache.is_hot(&uuid1), "Expected uuid1 to be hot");
        assert!(cache.is_cold(&uuid3), "Expected uuid3 to be cold");
    }
