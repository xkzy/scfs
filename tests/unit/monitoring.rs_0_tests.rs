// moved from src/monitoring.rs
use super::*;

    #[test]
    fn test_prometheus_export_format() {
        let metrics = Arc::new(Metrics::new());
        let exporter = PrometheusExporter::new(metrics);
        
        let output = exporter.export();
        
        // Should contain Prometheus format elements
        assert!(output.contains("# HELP"));
        assert!(output.contains("# TYPE"));
        assert!(output.contains("dynamicfs_disk_reads_total"));
    }

    #[test]
    fn test_json_export_format() {
        let metrics = Arc::new(Metrics::new());
        let exporter = PrometheusExporter::new(metrics);
        
        let output = exporter.export_json();
        
        // Should be valid JSON
        assert!(output.contains("{"));
        assert!(output.contains("\"metrics\""));
        assert!(output.contains("\"timestamp\""));
    }

    #[test]
    fn test_health_check_healthy() {
        let metrics = Arc::new(Metrics::new());
        let checker = HealthChecker::new(metrics);
        
        let status = checker.check();
        assert_eq!(status.status, HealthStatus::Healthy);
        assert!(checker.is_ready());
    }

    #[test]
    fn test_health_status_json() {
        let status = HealthCheckStatus {
            status: HealthStatus::Healthy,
            message: "All systems operational".to_string(),
            timestamp: "2026-01-20T00:00:00Z".to_string(),
        };

        let json = status.to_json();
        assert!(json.contains("healthy"));
        assert!(json.contains("All systems operational"));
    }

    #[test]
    fn test_health_status_codes() {
        let healthy = HealthCheckStatus {
            status: HealthStatus::Healthy,
            message: "".to_string(),
            timestamp: "".to_string(),
        };
        assert_eq!(healthy.http_status_code(), 200);

        let degraded = HealthCheckStatus {
            status: HealthStatus::Degraded,
            message: "".to_string(),
            timestamp: "".to_string(),
        };
        assert_eq!(degraded.http_status_code(), 202);

        let critical = HealthCheckStatus {
            status: HealthStatus::Critical,
            message: "".to_string(),
            timestamp: "".to_string(),
        };
        assert_eq!(critical.http_status_code(), 503);
    }
