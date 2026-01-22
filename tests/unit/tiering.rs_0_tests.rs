// moved from src/tiering.rs
use super::*;
    use crate::extent::{AccessStats, RedundancyPolicy, FragmentLocation};

    fn create_test_extent(size: usize, classification: AccessClassification) -> Extent {
        Extent {
            uuid: Uuid::new_v4(),
            size,
            checksum: [0; 32],
            redundancy: RedundancyPolicy::Replication { copies: 3 },
            fragment_locations: vec![
                FragmentLocation {
                    disk_uuid: Uuid::new_v4(),
                    fragment_index: 0,
                    on_device: None,
                },
            ],
            previous_policy: None,
            policy_transitions: Vec::new(),
            last_policy_change: None,
            access_stats: AccessStats {
                read_count: 10,
                write_count: 1,
                last_read: current_timestamp() as i64,
                last_write: current_timestamp() as i64,
                created_at: (current_timestamp() - 86400 * 30) as i64, // 30 days old
                classification,
                hmm_classifier: None,
            },
            rebuild_in_progress: false,
            rebuild_progress: None,
            generation: 0,
        }
    }

    #[test]
    fn test_tiering_policies() {
        let aggressive = TieringPolicy::aggressive();
        assert!(aggressive.hot_threshold_days < 7);

        let balanced = TieringPolicy::balanced();
        assert_eq!(balanced.hot_threshold_days, 7);

        let performance = TieringPolicy::performance();
        assert!(performance.hot_threshold_days > 7);
    }

    #[test]
    fn test_tiering_recommendation() {
        let policy = TieringPolicy::balanced();
        let extent = create_test_extent(1024, AccessClassification::Hot);
        
        let target = policy.target_tier(&extent);
        assert_eq!(target, StorageTier::Hot);
    }

    #[test]
    fn test_tiering_analyzer() {
        let policy = TieringPolicy::balanced();
        let mut analyzer = TieringAnalyzer::new(policy);

        let hot_extent = create_test_extent(1024, AccessClassification::Hot);
        let cold_extent = create_test_extent(512, AccessClassification::Cold);
        let extents = vec![hot_extent, cold_extent];

        let recommendations = analyzer.analyze_extents(&extents);
        
        // Should have recommendations for movement
        assert!(!recommendations.is_empty());
        
        // Check distribution tracking
        assert!(analyzer.current_tier_distribution.total_extents() > 0);
    }

    #[test]
    fn test_tier_distribution() {
        let mut dist = TierDistribution::default();
        dist.hot_bytes = 100;
        dist.warm_bytes = 100;
        dist.cold_bytes = 100;

        assert_eq!(dist.total_bytes(), 300);
        assert_eq!(dist.hot_percent() as u32, 33);
    }

    #[test]
    fn test_policy_violation_detection() {
        let mut policy = TieringPolicy::aggressive();
        policy.max_hot_percent = 10;

        let mut analyzer = TieringAnalyzer::new(policy);
        analyzer.current_tier_distribution.hot_bytes = 100;
        analyzer.current_tier_distribution.warm_bytes = 100;

        let violations = analyzer.check_policy_compliance();
        // 50% hot > 10% limit, so should detect violation
        assert!(!violations.is_empty());
    }
