// moved from src/reclamation.rs
use super::*;

    #[test]
    fn test_policy_presets() {
        let aggressive = ReclamationPolicy::Aggressive;
        let defrag = aggressive.defrag_config();
        assert!(defrag.enabled);
        assert_eq!(defrag.intensity, DefragIntensity::High);

        let performance = ReclamationPolicy::Performance;
        let perf_defrag = performance.defrag_config();
        assert!(!perf_defrag.enabled);
    }

    #[test]
    fn test_tier_policies() {
        let hot_policy = TierPolicy::for_tier(StorageTier::Hot, ReclamationPolicy::Balanced);
        assert_eq!(hot_policy.tier, StorageTier::Hot);
        assert!(hot_policy.defrag_enabled);

        let cold_policy = TierPolicy::for_tier(StorageTier::Cold, ReclamationPolicy::Balanced);
        assert_eq!(cold_policy.tier, StorageTier::Cold);
        assert!(!cold_policy.defrag_enabled); // Cold tier doesn't defrag
    }

    #[test]
    fn test_policy_descriptions() {
        assert!(!ReclamationPolicy::Aggressive.description().is_empty());
        assert!(!ReclamationPolicy::Balanced.description().is_empty());
        assert!(!ReclamationPolicy::Conservative.description().is_empty());
        assert!(!ReclamationPolicy::Performance.description().is_empty());
    }
