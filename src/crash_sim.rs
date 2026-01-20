use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use anyhow::{Result, anyhow};

/// Points where power loss can be simulated
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CrashPoint {
    /// Before writing temp file
    BeforeTempWrite,
    /// After writing temp file, before fsync
    AfterTempWrite,
    /// After fsync temp, before rename
    BeforeRename,
    /// After rename (crash after commit)
    AfterRename,
    /// Before fragment write
    BeforeFragmentWrite,
    /// After fragment write, before metadata commit
    AfterFragmentWrite,
    /// During extent metadata save
    DuringExtentMetadata,
    /// During extent map save
    DuringExtentMap,
    /// During inode save
    DuringInodeSave,
}

/// Configuration for crash simulation
#[derive(Clone)]
pub struct CrashSimulator {
    enabled: Arc<AtomicBool>,
    crash_point: Arc<Mutex<Option<CrashPoint>>>,
    crash_count: Arc<AtomicU64>,
    operations_count: Arc<AtomicU64>,
    crash_after_n_ops: Arc<AtomicU64>,
}

impl CrashSimulator {
    /// Create a new crash simulator (disabled by default)
    pub fn new() -> Self {
        CrashSimulator {
            enabled: Arc::new(AtomicBool::new(false)),
            crash_point: Arc::new(Mutex::new(None)),
            crash_count: Arc::new(AtomicU64::new(0)),
            operations_count: Arc::new(AtomicU64::new(0)),
            crash_after_n_ops: Arc::new(AtomicU64::new(u64::MAX)),
        }
    }
    
    /// Enable crash simulation at a specific point
    pub fn enable_at(&self, point: CrashPoint) {
        *self.crash_point.lock().unwrap() = Some(point);
        self.crash_after_n_ops.store(1, Ordering::SeqCst); // Crash immediately
        self.enabled.store(true, Ordering::SeqCst);
        self.operations_count.store(0, Ordering::SeqCst);
    }
    
    /// Enable crash after N operations at a specific point
    pub fn enable_after_n_ops(&self, point: CrashPoint, n: u64) {
        *self.crash_point.lock().unwrap() = Some(point);
        self.crash_after_n_ops.store(n, Ordering::SeqCst);
        self.enabled.store(true, Ordering::SeqCst);
        self.operations_count.store(0, Ordering::SeqCst);
    }
    
    /// Disable crash simulation
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::SeqCst);
        *self.crash_point.lock().unwrap() = None;
        self.crash_after_n_ops.store(u64::MAX, Ordering::SeqCst);
    }
    
    /// Check if we should crash at this point
    pub fn check_crash(&self, point: CrashPoint) -> Result<()> {
        if !self.enabled.load(Ordering::SeqCst) {
            return Ok(());
        }
        
        let target_point = self.crash_point.lock().unwrap();
        if *target_point == Some(point) {
            let ops = self.operations_count.fetch_add(1, Ordering::SeqCst) + 1;
            let crash_after = self.crash_after_n_ops.load(Ordering::SeqCst);
            
            if ops >= crash_after {
                self.crash_count.fetch_add(1, Ordering::SeqCst);
                return Err(anyhow!("SIMULATED POWER LOSS at {:?} (op #{})", point, ops));
            }
        }
        
        Ok(())
    }
    
    /// Get the number of times we've crashed
    pub fn crash_count(&self) -> u64 {
        self.crash_count.load(Ordering::SeqCst)
    }
    
    /// Reset the simulator
    pub fn reset(&self) {
        self.disable();
        self.crash_count.store(0, Ordering::SeqCst);
        self.operations_count.store(0, Ordering::SeqCst);
    }
    
    /// Get current operation count
    pub fn operation_count(&self) -> u64 {
        self.operations_count.load(Ordering::SeqCst)
    }
}

impl Default for CrashSimulator {
    fn default() -> Self {
        Self::new()
    }
}

// Thread-local crash simulator for testing
thread_local! {
    static CRASH_SIM: CrashSimulator = CrashSimulator::new();
}

/// Get the thread-local crash simulator
pub fn get_crash_simulator() -> CrashSimulator {
    CRASH_SIM.with(|sim| sim.clone())
}

/// Check for simulated crash at a specific point
#[inline]
pub fn check_crash_point(point: CrashPoint) -> Result<()> {
    get_crash_simulator().check_crash(point)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_crash_simulator_basic() {
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
    }
    
    #[test]
    fn test_crash_after_n_operations() {
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
    }
    
    #[test]
    fn test_crash_simulator_reset() {
        let sim = CrashSimulator::new();
        
        sim.enable_at(CrashPoint::BeforeRename);
        assert!(sim.check_crash(CrashPoint::BeforeRename).is_err());
        assert_eq!(sim.crash_count(), 1);
        
        sim.reset();
        assert!(sim.check_crash(CrashPoint::BeforeRename).is_ok());
        assert_eq!(sim.crash_count(), 0);
    }
}
