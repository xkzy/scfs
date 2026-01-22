// moved from src/write_optimizer.rs
use super::*;

    #[test]
    fn test_write_batcher_simple() {
        let batcher = WriteBatcher::new(2, 1024);
        let extent1 = create_test_extent(100);
        
        // First extent doesn't trigger batch
        assert!(batcher.add_extent(extent1.clone()).is_none());
        
        // Second extent triggers batch
        let extent2 = create_test_extent(100);
        let batch = batcher.add_extent(extent2).unwrap();
        assert_eq!(batch.extents.len(), 2);
        assert_eq!(batch.total_bytes, 200);
    }

    #[test]
    fn test_metadata_cache_lru() {
        let cache = MetadataCache::new(2);
        let uuid1 = Uuid::new_v4();
        let extent1 = create_test_extent(100);

        // Add first extent
        cache.put(uuid1, extent1.clone());
        assert_eq!(cache.len(), 1);
        
        // Get it back
        assert!(cache.get(&uuid1).is_some());
        
        // Add second extent
        let uuid2 = Uuid::new_v4();
        let extent2 = create_test_extent(100);
        cache.put(uuid2, extent2);
        assert_eq!(cache.len(), 2);
        
        // Add third (should evict first)
        let uuid3 = Uuid::new_v4();
        let extent3 = create_test_extent(100);
        cache.put(uuid3, extent3);
        assert_eq!(cache.len(), 2);
        
        // uuid1 should be gone
        assert!(cache.get(&uuid1).is_none());
    }

    fn create_test_extent(size: usize) -> Extent {
        use crate::extent::{AccessStats, RedundancyPolicy, AccessClassification};
        use chrono::Utc;
        Extent {
            uuid: Uuid::new_v4(),
            size,
            checksum: [0; 32],
            redundancy: RedundancyPolicy::Replication { copies: 3 },
            fragment_locations: Vec::new(),
            previous_policy: None,
            policy_transitions: Vec::new(),
            last_policy_change: None,
            access_stats: AccessStats {
                read_count: 0,
                write_count: 0,
                last_read: 0,
                last_write: 0,
                created_at: Utc::now().timestamp(),
                classification: AccessClassification::Cold,
                hmm_classifier: None,
            },
            rebuild_in_progress: false,
            rebuild_progress: None,
            generation: 0,
        }
    }

    #[test]
    fn test_write_coalescer() {
        let coalescer = WriteCoalescer::new(100, 500);
        
        // Small write doesn't coalesce
        let result1 = coalescer.try_coalesce(&[0; 50]);
        assert!(result1.is_none());
        
        // Another small write triggers coalescing
        let result2 = coalescer.try_coalesce(&[0; 60]);
        assert!(result2.is_some());
        let coalesced = result2.unwrap();
        assert_eq!(coalesced.len(), 110);
    }
