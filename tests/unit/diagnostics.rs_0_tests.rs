// moved from src/diagnostics.rs
use super::*;

    #[test]
    fn test_diagnostic_analyzer_healthy() {
        let analyzer = DiagnosticAnalyzer::new();
        let diag = analyzer.analyze(5.0, 10.0, 500.0, 95.0, 0.01, 99.0, 0, 50.0);
        
        assert_eq!(diag.health, DiagnosticHealth::Healthy);
        assert!(diag.issues.is_empty());
    }

    #[test]
    fn test_diagnostic_analyzer_high_latency() {
        let analyzer = DiagnosticAnalyzer::new();
        let diag = analyzer.analyze(50.0, 10.0, 100.0, 80.0, 0.01, 99.0, 0, 50.0);
        
        assert_eq!(diag.health, DiagnosticHealth::Warning);
        assert!(!diag.issues.is_empty());
    }

    #[test]
    fn test_diagnostic_analyzer_critical_capacity() {
        let analyzer = DiagnosticAnalyzer::new();
        let diag = analyzer.analyze(5.0, 10.0, 100.0, 80.0, 0.01, 99.0, 0, 97.0);
        
        assert_eq!(diag.health, DiagnosticHealth::Critical);
        assert!(diag.issues.len() >= 1);
    }

    #[test]
    fn test_runbook_resolution() {
        let issue = DiagnosticIssue {
            severity: IssueSeverity::Critical,
            category: IssueCategory::Capacity,
            title: "test".to_string(),
            description: "test".to_string(),
            recommendation: "test".to_string(),
        };

        let steps = Runbook::get_resolution(&issue);
        assert!(!steps.is_empty());
    }

    #[test]
    fn test_diagnostics_formatter() {
        let analyzer = DiagnosticAnalyzer::new();
        let diag = analyzer.analyze(5.0, 10.0, 100.0, 80.0, 0.01, 99.0, 0, 50.0);
        
        let text = DiagnosticsFormatter::format_text(&diag);
        assert!(text.contains("DIAGNOSTICS REPORT"));
        
        let json = DiagnosticsFormatter::format_json(&diag);
        assert!(json.contains("timestamp"));
    }
