use serde_json::json;
use std::collections::HashMap;

/// JSON output formatter for CLI commands
pub struct JsonOutput;

impl JsonOutput {
    /// Format status as JSON
    pub fn status(
        disk_count: usize,
        extent_count: usize,
        healthy_extents: usize,
        degraded_extents: usize,
        total_capacity: u64,
        used_capacity: u64,
    ) -> String {
        json!({
            "status": "ok",
            "disks": {
                "total": disk_count,
                "healthy": disk_count,
                "failed": 0
            },
            "extents": {
                "total": extent_count,
                "healthy": healthy_extents,
                "degraded": degraded_extents,
                "unrecoverable": 0
            },
            "capacity": {
                "total_bytes": total_capacity,
                "used_bytes": used_capacity,
                "available_bytes": total_capacity.saturating_sub(used_capacity),
                "used_percent": if total_capacity > 0 {
                    (used_capacity as f64 / total_capacity as f64 * 100.0) as u32
                } else {
                    0
                }
            },
            "timestamp": chrono::Utc::now().to_rfc3339()
        }).to_string()
    }

    /// Format metrics as JSON
    pub fn metrics(
        disk_reads: u64,
        disk_writes: u64,
        disk_read_bytes: u64,
        disk_write_bytes: u64,
        cache_hits: u64,
        cache_misses: u64,
    ) -> String {
        let total_ops = disk_reads + disk_writes;
        let total_cache = cache_hits + cache_misses;
        let cache_hit_rate = if total_cache > 0 {
            (cache_hits as f64 / total_cache as f64 * 100.0) as u32
        } else {
            0
        };

        json!({
            "disk_io": {
                "reads": disk_reads,
                "writes": disk_writes,
                "read_bytes": disk_read_bytes,
                "write_bytes": disk_write_bytes,
                "total_operations": total_ops
            },
            "cache": {
                "hits": cache_hits,
                "misses": cache_misses,
                "hit_rate_percent": cache_hit_rate
            },
            "timestamp": chrono::Utc::now().to_rfc3339()
        }).to_string()
    }

    /// Format error as JSON
    pub fn error(message: &str, code: i32) -> String {
        json!({
            "error": {
                "message": message,
                "code": code
            },
            "timestamp": chrono::Utc::now().to_rfc3339()
        }).to_string()
    }

    /// Format success response
    pub fn success(data: serde_json::Value) -> String {
        json!({
            "status": "success",
            "data": data,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }).to_string()
    }

    /// Format list response
    pub fn list<T: serde::Serialize>(items: Vec<T>, total: usize) -> anyhow::Result<String> {
        let json = json!({
            "status": "success",
            "items": items,
            "count": total,
            "timestamp": chrono::Utc::now().to_rfc3339()
        });
        Ok(json.to_string())
    }

    /// Format paginated response
    pub fn paginated<T: serde::Serialize>(
        items: Vec<T>,
        page: usize,
        page_size: usize,
        total: usize,
    ) -> anyhow::Result<String> {
        let total_pages = (total + page_size - 1) / page_size;
        let json = json!({
            "status": "success",
            "items": items,
            "pagination": {
                "page": page,
                "page_size": page_size,
                "total_items": total,
                "total_pages": total_pages,
                "has_next": page < total_pages,
                "has_previous": page > 1
            },
            "timestamp": chrono::Utc::now().to_rfc3339()
        });
        Ok(json.to_string())
    }
}

/// Pretty-printing utilities
pub struct JsonPretty;

impl JsonPretty {
    /// Pretty-print JSON with indentation
    pub fn format(json_str: &str) -> anyhow::Result<String> {
        let value: serde_json::Value = serde_json::from_str(json_str)?;
        Ok(serde_json::to_string_pretty(&value)?)
    }

    /// Pretty-print with custom indent
    pub fn format_with_indent(json_str: &str, indent: usize) -> anyhow::Result<String> {
        let value: serde_json::Value = serde_json::from_str(json_str)?;
        let formatted = serde_json::to_string_pretty(&value)?;
        
        // Replace indentation
        let indent_str = " ".repeat(indent);
        let lines: Vec<&str> = formatted.lines().collect();
        let mut result = String::new();
        
        for line in lines {
            let leading_spaces = line.len() - line.trim_start().len();
            let new_indent = (leading_spaces / 2) * indent;
            let new_line = format!("{}{}", " ".repeat(new_indent), line.trim_start());
            result.push_str(&new_line);
            result.push('\n');
        }
        
        Ok(result.trim_end().to_string())
    }
}

#[cfg(test)]
mod tests {
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
}
