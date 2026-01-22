// moved from src/json_output.rs
use super::*;

    #[test]
    fn test_status_json() {
        let json = JsonOutput::status(3, 10, 9, 1, 1000000000, 500000000);
        
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["disks"]["total"], 3);
        assert_eq!(parsed["extents"]["healthy"], 9);
        assert_eq!(parsed["capacity"]["used_percent"], 50);
    }

    #[test]
    fn test_metrics_json() {
        let json = JsonOutput::metrics(1000, 500, 10000000, 5000000, 900, 100);
        
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["disk_io"]["reads"], 1000);
        assert_eq!(parsed["cache"]["hit_rate_percent"], 90);
    }

    #[test]
    fn test_error_json() {
        let json = JsonOutput::error("Test error", 500);
        
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["error"]["message"], "Test error");
        assert_eq!(parsed["error"]["code"], 500);
    }

    #[test]
    fn test_success_json() {
        let data = json!({
            "message": "Operation successful"
        });
        let json = JsonOutput::success(data);
        
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "success");
    }

    #[test]
    fn test_json_pretty() {
        let json = r#"{"status":"ok","data":{"value":123}}"#;
        let pretty = JsonPretty::format(json).unwrap();
        
        assert!(pretty.contains("{\n"));
        assert!(pretty.contains("  \"status\""));
    }

    #[test]
    fn test_json_list() {
        let items = vec!["item1", "item2", "item3"];
        let json = JsonOutput::list(items, 3).unwrap();
        
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["count"], 3);
        assert_eq!(parsed["items"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_json_paginated() {
        let items = vec![1, 2, 3, 4, 5];
        let json = JsonOutput::paginated(items, 1, 5, 15).unwrap();
        
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["pagination"]["page"], 1);
        assert_eq!(parsed["pagination"]["total_pages"], 3);
        assert_eq!(parsed["pagination"]["has_next"], true);
    }
