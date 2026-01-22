// moved from src/scrub_daemon.rs
use super::*;

    #[test]
    fn test_scrub_intensity_levels() {
        assert_eq!(ScrubIntensity::Low.io_throttle_ms(), 100);
        assert_eq!(ScrubIntensity::Medium.io_throttle_ms(), 50);
        assert_eq!(ScrubIntensity::High.io_throttle_ms(), 10);
    }

    #[test]
    fn test_scrub_daemon_lifecycle() {
        let daemon = ScrubDaemon::new();
        
        assert!(!daemon.is_running());
        daemon.start(ScrubSchedule::nightly_low()).unwrap();
        assert!(daemon.is_running());
        
        daemon.pause();
        assert!(daemon.is_paused());
        
        daemon.resume();
        assert!(!daemon.is_paused());
        
        daemon.stop();
        assert!(!daemon.is_running());
    }

    #[test]
    fn test_scrub_intensity_change() {
        let daemon = ScrubDaemon::new();
        
        daemon.set_intensity(ScrubIntensity::High).unwrap();
        assert_eq!(daemon.get_intensity().unwrap(), ScrubIntensity::High);
    }

    #[test]
    fn test_scrub_metrics() {
        let daemon = ScrubDaemon::new();
        
        daemon.record_extent_scanned(1, 0, 4096);
        daemon.record_extent_scanned(0, 1, 8192);
        
        let metrics = daemon.get_metrics();
        assert_eq!(metrics.extents_scanned, 2);
        assert_eq!(metrics.issues_found, 1);
        assert_eq!(metrics.repairs_triggered, 1);
        assert_eq!(metrics.scrub_io_bytes, 12288);
    }

    #[test]
    fn test_scrub_schedule() {
        let nightly = ScrubSchedule::nightly_low();
        assert_eq!(nightly.interval_hours, 24);
        assert_eq!(nightly.intensity, ScrubIntensity::Low);
        assert!(nightly.auto_repair);
    }

    #[test]
    fn test_repair_queue() {
        let queue = RepairQueue::new(4);
        
        let task = RepairTask {
            extent_uuid: Uuid::new_v4(),
            priority: 5,
            created_at: std::time::SystemTime::now(),
            status: RepairStatus::Queued,
        };
        
        queue.enqueue(task.clone()).unwrap();
        assert_eq!(queue.queue_size().unwrap(), 1);
        
        let next = queue.next_task().unwrap();
        assert!(next.is_some());
        assert_eq!(queue.queue_size().unwrap(), 0);
    }

    #[test]
    fn test_repair_queue_priority() {
        let queue = RepairQueue::new(4);
        
        let high_priority = RepairTask {
            extent_uuid: Uuid::new_v4(),
            priority: 1,
            created_at: std::time::SystemTime::now(),
            status: RepairStatus::Queued,
        };
        
        let low_priority = RepairTask {
            extent_uuid: Uuid::new_v4(),
            priority: 10,
            created_at: std::time::SystemTime::now(),
            status: RepairStatus::Queued,
        };
        
        queue.enqueue(low_priority).unwrap();
        queue.enqueue(high_priority.clone()).unwrap();
        
        let first = queue.next_task().unwrap().unwrap();
        assert_eq!(first.priority, 1); // High priority task should be first
    }
