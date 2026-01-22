// moved from src/crash_sim.rs
use super::*;
    use crate::test_utils::run_with_timeout;

    #[test]
    fn test_crash_simulator_basic() {
        // Wrap with timeout to guard against unexpected hangs
        let res = run_with_timeout(2, || {
            let sim = CrashSimulator::new();
            // Initially disabled
            assert!(sim.check_crash(CrashPoint::BeforeTempWrite).is_ok());
            // Enable at specific point
            sim.enable_at(CrashPoint::BeforeTempWrite);
            assert!(sim.check_crash(CrashPoint::BeforeTempWrite).is_err());
            assert_eq!(sim.crash_count(), 1);
            // Different point should not crash
            assert!(sim.check_crash(CrashPoint::AfterTempWrite).is_ok());
            assert_eq!(sim.crash_count(), 1);
            // Disable
            sim.disable();
            assert!(sim.check_crash(CrashPoint::BeforeTempWrite).is_ok());
        });
        assert!(res.is_ok(), "test timed out");
    }

    #[test]
    fn test_crash_after_n_operations() {
        let res = run_with_timeout(2, || {
            let sim = CrashSimulator::new();
            // Enable after 3 operations
            sim.enable_after_n_ops(CrashPoint::BeforeTempWrite, 3);
            // First two operations should succeed
            assert!(sim.check_crash(CrashPoint::BeforeTempWrite).is_ok());
            assert!(sim.check_crash(CrashPoint::BeforeTempWrite).is_ok());
            assert_eq!(sim.operation_count(), 2);
            // Third operation should crash
            assert!(sim.check_crash(CrashPoint::BeforeTempWrite).is_err());
            assert_eq!(sim.crash_count(), 1);
            assert_eq!(sim.operation_count(), 3);
        });
        assert!(res.is_ok(), "test timed out");
    }

    #[test]
    fn test_crash_simulator_reset() {
        let res = run_with_timeout(2, || {
            let sim = CrashSimulator::new();
            sim.enable_at(CrashPoint::BeforeRename);
            assert!(sim.check_crash(CrashPoint::BeforeRename).is_err());
            assert_eq!(sim.crash_count(), 1);
            sim.reset();
            assert!(sim.check_crash(CrashPoint::BeforeRename).is_ok());
            assert_eq!(sim.crash_count(), 0);
        });
        assert!(res.is_ok(), "test timed out");
    }
