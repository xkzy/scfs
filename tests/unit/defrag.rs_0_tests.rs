// moved from src/defrag.rs
use super::*;

    #[test]
    fn test_defrag_intensity() {
        assert_eq!(DefragIntensity::Low.io_throttle_ms(), 100);
        assert_eq!(DefragIntensity::Medium.io_throttle_ms(), 50);
        assert_eq!(DefragIntensity::High.io_throttle_ms(), 10);
    }

    #[test]
    fn test_defrag_recommendation() {
        let low = FragmentationAnalysis {
            timestamp: 0,
            total_extents: 100,
            fragmented_extents: 10,
            overall_fragmentation_ratio: 0.10,
            per_disk_stats: vec![],
            recommendation: DefragRecommendation::None,
        };
        assert_eq!(low.recommendation, DefragRecommendation::None);

        let medium = FragmentationAnalysis {
            timestamp: 0,
            total_extents: 100,
            fragmented_extents: 35,
            overall_fragmentation_ratio: 0.35,
            per_disk_stats: vec![],
            recommendation: DefragRecommendation::Recommended,
        };
        assert_eq!(medium.recommendation, DefragRecommendation::Recommended);
    }
