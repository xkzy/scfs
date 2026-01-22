use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Background scrubber daemon for continuous verification
pub struct ScrubDaemon {
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    intensity: Arc<std::sync::Mutex<ScrubIntensity>>,
    
    // Metrics
    extents_scanned: Arc<AtomicU64>,
    issues_found: Arc<AtomicU64>,
    repairs_triggered: Arc<AtomicU64>,
    scrub_io_bytes: Arc<AtomicU64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrubIntensity {
    Low,
    Medium,
    High,
}

impl ScrubIntensity {
    pub fn io_throttle_ms(&self) -> u64 {
        match self {
            ScrubIntensity::Low => 100,
            ScrubIntensity::Medium => 50,
            ScrubIntensity::High => 10,
        }
    }

    pub fn batch_size(&self) -> usize {
        match self {
            ScrubIntensity::Low => 1,
            ScrubIntensity::Medium => 5,
            ScrubIntensity::High => 20,
        }
    }

    pub fn priority(&self) -> u32 {
        match self {
            ScrubIntensity::Low => 10,
            ScrubIntensity::Medium => 5,
            ScrubIntensity::High => 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScrubSchedule {
    pub enabled: bool,
    pub interval_hours: u32,
    pub intensity: ScrubIntensity,
    pub dry_run: bool,
    pub auto_repair: bool,
}

impl ScrubSchedule {
    pub fn nightly_low() -> Self {
        ScrubSchedule {
            enabled: true,
            interval_hours: 24,
            intensity: ScrubIntensity::Low,
            dry_run: false,
            auto_repair: true,
        }
    }

    pub fn continuous_medium() -> Self {
        ScrubSchedule {
            enabled: true,
            interval_hours: 6,
            intensity: ScrubIntensity::Medium,
            dry_run: false,
            auto_repair: false, // Manual approval for medium/high
        }
    }

    pub fn manual(intensity: ScrubIntensity) -> Self {
        ScrubSchedule {
            enabled: false,
            interval_hours: 0,
            intensity,
            dry_run: false,
            auto_repair: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScrubProgress {
    pub status: ScrubStatus,
    pub extents_total: u64,
    pub extents_scanned: u64,
    pub issues_found: u64,
    pub repairs_triggered: u64,
    pub start_time: std::time::SystemTime,
    pub estimated_completion: std::time::SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrubStatus {
    Idle,
    Running,
    Paused,
    Throttled,
    Completed,
    Failed,
}

impl ScrubDaemon {
    pub fn new() -> Self {
        ScrubDaemon {
            running: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            intensity: Arc::new(std::sync::Mutex::new(ScrubIntensity::Low)),
            extents_scanned: Arc::new(AtomicU64::new(0)),
            issues_found: Arc::new(AtomicU64::new(0)),
            repairs_triggered: Arc::new(AtomicU64::new(0)),
            scrub_io_bytes: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Start the scrub daemon
    pub fn start(&self, schedule: ScrubSchedule) -> anyhow::Result<()> {
        if self.running.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("Daemon already running"));
        }

        self.running.store(true, Ordering::Relaxed);
        log::info!("Scrub daemon started with intensity: {:?}", schedule.intensity);

        Ok(())
    }

    /// Stop the scrub daemon
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        self.paused.store(false, Ordering::Relaxed);
        log::info!("Scrub daemon stopped");
    }

    /// Pause the daemon (doesn't stop, just pauses current work)
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
        log::info!("Scrub daemon paused");
    }

    /// Resume the daemon
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
        log::info!("Scrub daemon resumed");
    }

    /// Check if daemon is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Check if daemon is paused
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Change intensity level
    pub fn set_intensity(&self, intensity: ScrubIntensity) -> anyhow::Result<()> {
        let mut current = self.intensity.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        *current = intensity;
        log::info!("Scrub intensity changed to: {:?}", intensity);
        Ok(())
    }

    /// Get current intensity
    pub fn get_intensity(&self) -> anyhow::Result<ScrubIntensity> {
        self.intensity.lock()
            .map(|guard| *guard)
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))
    }

    /// Record extent scan
    pub fn record_extent_scanned(&self, issues: u64, repairs: u64, io_bytes: u64) {
        self.extents_scanned.fetch_add(1, Ordering::Relaxed);
        self.issues_found.fetch_add(issues, Ordering::Relaxed);
        self.repairs_triggered.fetch_add(repairs, Ordering::Relaxed);
        self.scrub_io_bytes.fetch_add(io_bytes, Ordering::Relaxed);
    }

    /// Get current progress
    pub fn get_progress(&self) -> ScrubProgress {
        ScrubProgress {
            status: if self.is_paused() {
                ScrubStatus::Paused
            } else if self.is_running() {
                ScrubStatus::Running
            } else {
                ScrubStatus::Idle
            },
            extents_total: 0, // Would be populated from metadata
            extents_scanned: self.extents_scanned.load(Ordering::Relaxed),
            issues_found: self.issues_found.load(Ordering::Relaxed),
            repairs_triggered: self.repairs_triggered.load(Ordering::Relaxed),
            start_time: std::time::SystemTime::now(),
            estimated_completion: std::time::SystemTime::now() + Duration::from_secs(3600),
        }
    }

    /// Get metrics snapshot
    pub fn get_metrics(&self) -> ScrubMetrics {
        ScrubMetrics {
            extents_scanned: self.extents_scanned.load(Ordering::Relaxed),
            issues_found: self.issues_found.load(Ordering::Relaxed),
            repairs_triggered: self.repairs_triggered.load(Ordering::Relaxed),
            scrub_io_bytes: self.scrub_io_bytes.load(Ordering::Relaxed),
            is_running: self.is_running(),
            is_paused: self.is_paused(),
        }
    }

    /// Reset metrics
    pub fn reset_metrics(&self) {
        self.extents_scanned.store(0, Ordering::Relaxed);
        self.issues_found.store(0, Ordering::Relaxed);
        self.repairs_triggered.store(0, Ordering::Relaxed);
        self.scrub_io_bytes.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub struct ScrubMetrics {
    pub extents_scanned: u64,
    pub issues_found: u64,
    pub repairs_triggered: u64,
    pub scrub_io_bytes: u64,
    pub is_running: bool,
    pub is_paused: bool,
}

/// Repair queue for managing repair operations
pub struct RepairQueue {
    queue: std::sync::Mutex<Vec<RepairTask>>,
    max_concurrent: usize,
}

#[derive(Debug, Clone)]
pub struct RepairTask {
    pub extent_uuid: Uuid,
    pub priority: u32,
    pub created_at: std::time::SystemTime,
    pub status: RepairStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
}

impl RepairQueue {
    pub fn new(max_concurrent: usize) -> Self {
        RepairQueue {
            queue: std::sync::Mutex::new(Vec::new()),
            max_concurrent,
        }
    }

    /// Enqueue a repair task
    pub fn enqueue(&self, task: RepairTask) -> anyhow::Result<()> {
        let mut queue = self.queue.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        queue.push(task);
        // Sort by priority (lower is higher priority)
        queue.sort_by_key(|t| t.priority);
        Ok(())
    }

    /// Get next task to repair (highest priority = lowest number)
    pub fn next_task(&self) -> anyhow::Result<Option<RepairTask>> {
        let mut queue = self.queue.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        if !queue.is_empty() {
            // Sort by priority ascending (1 = highest priority)
            queue.sort_by_key(|t| t.priority);
            let mut task = queue.remove(0);  // Take first (highest priority)
            task.status = RepairStatus::InProgress;
            Ok(Some(task))
        } else {
            Ok(None)
        }
    }

    /// Mark task as completed
    pub fn mark_completed(&self, extent_uuid: Uuid) -> anyhow::Result<()> {
        log::info!("Repair completed for extent: {}", extent_uuid);
        Ok(())
    }

    /// Get queue status
    pub fn queue_size(&self) -> anyhow::Result<usize> {
        self.queue.lock()
            .map(|queue| queue.len())
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))
    }
}

#[cfg(test)]
mod tests {
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
}
