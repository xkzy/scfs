use serde::{Serialize, Deserialize};
use std::fmt::Write;
use chrono::{DateTime, Utc};

/// Structured log event with JSON serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub component: String,
    pub message: String,
    pub context: Option<serde_json::Value>,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

impl LogEvent {
    pub fn new(component: impl Into<String>, level: LogLevel, message: impl Into<String>) -> Self {
        LogEvent {
            timestamp: Utc::now(),
            level,
            component: component.into(),
            message: message.into(),
            context: None,
            trace_id: None,
        }
    }

    /// Add context data to log event
    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = Some(context);
        self
    }

    /// Add trace ID for correlation
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    /// Convert to JSON string
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                r#"{{"timestamp":"{}","level":"{}","component":"{}","message":"{}"}}"#,
                self.timestamp.to_rfc3339(),
                self.level,
                self.component,
                self.message
            )
        })
    }

    /// Convert to human-readable format
    pub fn to_text(&self) -> String {
        let mut output = format!(
            "[{}] {} {}: {}",
            self.timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            self.level,
            self.component,
            self.message
        );

        if let Some(trace_id) = &self.trace_id {
            write!(output, " [trace_id:{}]", trace_id).unwrap();
        }

        if let Some(context) = &self.context {
            write!(output, " context={}", context).unwrap();
        }

        output
    }
}

/// Event log buffer for collection and analysis
pub struct EventLog {
    events: Vec<LogEvent>,
    max_size: usize,
    min_level: LogLevel,
}

impl EventLog {
    pub fn new(max_size: usize, min_level: LogLevel) -> Self {
        EventLog {
            events: Vec::new(),
            max_size,
            min_level,
        }
    }

    /// Add event to log, pruning oldest if needed
    pub fn log(&mut self, event: LogEvent) {
        if event.level >= self.min_level {
            self.events.push(event);
            if self.events.len() > self.max_size {
                self.events.remove(0);
            }
        }
    }

    /// Get all events
    pub fn get_events(&self) -> Vec<LogEvent> {
        self.events.clone()
    }

    /// Get events by level
    pub fn get_events_by_level(&self, level: LogLevel) -> Vec<LogEvent> {
        self.events.iter().filter(|e| e.level == level).cloned().collect()
    }

    /// Export as JSON lines (one event per line)
    pub fn export_jsonl(&self) -> String {
        self.events.iter()
            .map(|e| e.to_json())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Export as CSV
    pub fn export_csv(&self) -> String {
        let mut output = String::from("timestamp,level,component,message,trace_id\n");
        for event in &self.events {
            let trace_id = event.trace_id.as_deref().unwrap_or("");
            write!(
                output,
                "\"{}\",{},{},\"{}\",{}\n",
                event.timestamp.to_rfc3339(),
                event.level,
                event.component,
                event.message.replace("\"", "\"\""),
                trace_id
            ).unwrap();
        }
        output
    }

    /// Clear all events
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Get statistics
    pub fn stats(&self) -> LogStats {
        let mut debug_count = 0;
        let mut info_count = 0;
        let mut warn_count = 0;
        let mut error_count = 0;

        for event in &self.events {
            match event.level {
                LogLevel::Debug => debug_count += 1,
                LogLevel::Info => info_count += 1,
                LogLevel::Warn => warn_count += 1,
                LogLevel::Error => error_count += 1,
            }
        }

        LogStats {
            total_events: self.events.len(),
            debug_count,
            info_count,
            warn_count,
            error_count,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogStats {
    pub total_events: usize,
    pub debug_count: usize,
    pub info_count: usize,
    pub warn_count: usize,
    pub error_count: usize,
}

impl std::fmt::Display for LogStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LogStats: {} total | {} debug | {} info | {} warn | {} error",
            self.total_events,
            self.debug_count,
            self.info_count,
            self.warn_count,
            self.error_count
        )
    }
}

/// Request context for distributed tracing
pub struct RequestContext {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub user_id: Option<String>,
}

impl RequestContext {
    pub fn new() -> Self {
        RequestContext {
            trace_id: uuid::Uuid::new_v4().to_string(),
            span_id: uuid::Uuid::new_v4().to_string(),
            parent_span_id: None,
            user_id: None,
        }
    }

    pub fn with_parent(mut self, parent_span_id: String) -> Self {
        self.parent_span_id = Some(parent_span_id);
        self
    }

    pub fn with_user(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Create child context for nested operations
    pub fn child(&self) -> Self {
        RequestContext {
            trace_id: self.trace_id.clone(),
            span_id: uuid::Uuid::new_v4().to_string(),
            parent_span_id: Some(self.span_id.clone()),
            user_id: self.user_id.clone(),
        }
    }

    pub fn to_headers(&self) -> Vec<(String, String)> {
        vec![
            ("X-Trace-ID".to_string(), self.trace_id.clone()),
            ("X-Span-ID".to_string(), self.span_id.clone()),
        ]
    }
}

/// Timing measurement for operations
pub struct TimingEvent {
    pub name: String,
    pub start: DateTime<Utc>,
    pub duration_ms: u64,
    pub success: bool,
}

impl TimingEvent {
    pub fn new(name: impl Into<String>, start: DateTime<Utc>, duration_ms: u64, success: bool) -> Self {
        TimingEvent {
            name: name.into(),
            start,
            duration_ms,
            success,
        }
    }

    pub fn to_json(&self) -> String {
        format!(
            r#"{{"name":"{}","timestamp":"{}","duration_ms":{},"success":{}}}"#,
            self.name,
            self.start.to_rfc3339(),
            self.duration_ms,
            self.success
        )
    }
}

