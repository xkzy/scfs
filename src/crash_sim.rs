use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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
    /// Before writing fragment data to device
    BeforeFragmentDataWrite,
    /// After writing fragment data, before fdatasync
    AfterFragmentDataWrite,
    /// After fdatasync, before allocator persist
    AfterFragmentFsync,
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

        #[cfg(test)]
        eprintln!(
            "[CRASH_SIM DEBUG] enable_at {:?} crash_after={}",
            point,
            self.crash_after_n_ops.load(Ordering::SeqCst)
        );
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
        #[cfg(test)]
        eprintln!(
            "[CRASH_SIM DEBUG] check {:?} enabled={}",
            point,
            self.enabled.load(Ordering::SeqCst)
        );

        if !self.enabled.load(Ordering::SeqCst) {
            return Ok(());
        }
        
        let target_point = self.crash_point.lock().unwrap();
        #[cfg(test)]
        eprintln!(
            "[CRASH_SIM DEBUG] check {:?} enabled target={:?} ops={} crash_after={}",
            point,
            *target_point,
            self.operations_count.load(Ordering::SeqCst),
            self.crash_after_n_ops.load(Ordering::SeqCst)
        );

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

// Global crash simulator shared across threads (tests spawn worker threads during writes)
static CRASH_SIM: OnceLock<CrashSimulator> = OnceLock::new();

/// Get the shared crash simulator
pub fn get_crash_simulator() -> &'static CrashSimulator {
    CRASH_SIM.get_or_init(CrashSimulator::new)
}

/// Check for simulated crash at a specific point
#[inline]
pub fn check_crash_point(point: CrashPoint) -> Result<()> {
    let res = get_crash_simulator().check_crash(point);
    #[cfg(test)]
    if res.is_err() {
        eprintln!("[CRASH_SIM DEBUG] crash at {:?}", point);
    }
    res
}

