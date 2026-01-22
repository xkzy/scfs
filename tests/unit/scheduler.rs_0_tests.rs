// moved from src/scheduler.rs
use super::*;

    #[test]
    fn test_replica_selector_first_strategy() {
        // Test that first strategy returns first available replica
        // This would require setting up test disks and extents
    }

    #[test]
    fn test_write_scheduler_balances_load() {
        let scheduler = FragmentWriteScheduler::new(2);
        // Would test that writes are balanced across healthy disks
    }
