// moved from src/logging.rs
use super::*;

    #[test]
    fn test_log_event_creation() {
        let event = LogEvent::new("test", LogLevel::Info, "Test message");
        
        assert_eq!(event.component, "test");
        assert_eq!(event.level, LogLevel::Info);
        assert_eq!(event.message, "Test message");
    }

    #[test]
    fn test_log_event_json() {
        let event = LogEvent::new("test", LogLevel::Error, "Error occurred");
        let json = event.to_json();
        
        assert!(json.contains("ERROR"));
        assert!(json.contains("Error occurred"));
    }

    #[test]
    fn test_log_event_text() {
        let event = LogEvent::new("storage", LogLevel::Warn, "Disk space low");
        let text = event.to_text();
        
        assert!(text.contains("WARN"));
        assert!(text.contains("storage"));
    }

    #[test]
    fn test_event_log_pruning() {
        let mut log = EventLog::new(3, LogLevel::Debug);
        
        for i in 0..5 {
            log.log(LogEvent::new("test", LogLevel::Info, format!("Event {}", i)));
        }
        
        assert_eq!(log.events.len(), 3);
        assert_eq!(log.events[0].message, "Event 2");
    }

    #[test]
    fn test_event_log_filtering() {
        let mut log = EventLog::new(100, LogLevel::Debug);
        
        log.log(LogEvent::new("test", LogLevel::Debug, "debug"));
        log.log(LogEvent::new("test", LogLevel::Info, "info"));
        log.log(LogEvent::new("test", LogLevel::Error, "error"));

        let errors = log.get_events_by_level(LogLevel::Error);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_log_stats() {
        let mut log = EventLog::new(100, LogLevel::Debug);
        
        log.log(LogEvent::new("test", LogLevel::Info, "msg1"));
        log.log(LogEvent::new("test", LogLevel::Info, "msg2"));
        log.log(LogEvent::new("test", LogLevel::Error, "msg3"));

        let stats = log.stats();
        assert_eq!(stats.total_events, 3);
        assert_eq!(stats.info_count, 2);
        assert_eq!(stats.error_count, 1);
    }

    #[test]
    fn test_request_context() {
        let ctx = RequestContext::new();
        
        assert!(!ctx.trace_id.is_empty());
        assert!(!ctx.span_id.is_empty());
        assert_eq!(ctx.parent_span_id, None);
    }

    #[test]
    fn test_request_context_child() {
        let parent = RequestContext::new();
        let child = parent.child();
        
        assert_eq!(parent.trace_id, child.trace_id);
        assert_ne!(parent.span_id, child.span_id);
        assert_eq!(child.parent_span_id, Some(parent.span_id));
    }

    #[test]
    fn test_timing_event() {
        let timing = TimingEvent::new(
            "write_file",
            Utc::now(),
            42,
            true
        );
        
        let json = timing.to_json();
        assert!(json.contains("write_file"));
        assert!(json.contains("42"));
        assert!(json.contains("true"));
    }

    #[test]
    fn test_log_export_formats() {
        let mut log = EventLog::new(100, LogLevel::Debug);
        
        log.log(LogEvent::new("test", LogLevel::Info, "msg1"));
        log.log(LogEvent::new("test", LogLevel::Error, "msg2"));

        let jsonl = log.export_jsonl();
        assert!(jsonl.contains("\n"));

        let csv = log.export_csv();
        assert!(csv.contains("timestamp,level"));
        assert!(csv.contains("msg1"));
    }
