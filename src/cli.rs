use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "dynamicfs")]
#[command(about = "Dynamic Object-Based Filesystem", long_about = None)]
pub struct Cli {
    /// Output results in JSON format
    #[arg(long, global = true)]
    pub json: bool,

    /// Control Direct I/O behavior: auto|always|never
    #[arg(long, global = true, default_value = "auto")]
    pub direct_io: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new storage pool
    Init {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Add a disk to the pool
    AddDisk {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// Disk directory or block device path
        #[arg(short, long)]
        disk: PathBuf,

        /// Add a raw block device (explicit confirmation required)
        #[arg(long, default_value_t = false)]
        device: bool,
    },
    
    /// Remove a disk from the pool
    RemoveDisk {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// Disk directory
        #[arg(short, long)]
        disk: PathBuf,
    },
    
    /// List all disks in the pool
    ListDisks {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// List all extents in the pool
    ListExtents {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Show redundancy status
    ShowRedundancy {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Simulate disk failure
    FailDisk {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// Disk directory
        #[arg(short, long)]
        disk: PathBuf,
    },

    /// Set disk health state (healthy|degraded|suspect|draining|failed)
    SetDiskHealth {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,

        /// Disk directory
        #[arg(short, long)]
        disk: PathBuf,

        /// Target health state
        #[arg(short, long)]
        health: String,
    },
    
    /// Change redundancy policy for a file
    ChangePolicy {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// Target policy
        #[arg(short, long)]
        policy: String, // "replication:N" or "erasure:K+M"
    },
    
    /// Show policy transition status
    PolicyStatus {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// List hot extents
    ListHot {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// List cold extents
    ListCold {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Show extent access statistics
    ExtentStats {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// Extent UUID
        #[arg(short, long)]
        extent: String,
    },
    
    /// Detect orphaned fragments
    DetectOrphans {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Clean up orphaned fragments
    CleanupOrphans {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// Minimum age in hours (default: 24)
        #[arg(short, long, default_value = "24")]
        min_age_hours: u64,
        
        /// Dry run - don't actually delete
        #[arg(short, long, default_value = "false")]
        dry_run: bool,
    },
    
    /// Show orphan statistics
    OrphanStats {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },

    /// Probe disks in the pool and update health state
    ProbeDisks {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },

    /// Scrub all extents for corruption and issues
    Scrub {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,

        /// Attempt to repair detected issues
        #[arg(short, long, default_value = "false")]
        repair: bool,
    },

    /// Control background scrub daemon
    ScrubDaemon {
        #[command(subcommand)]
        action: ScrubDaemonAction,
    },

    /// Schedule periodic scrubbing
    ScrubSchedule {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,

        /// Schedule frequency (nightly, continuous, manual)
        #[arg(short, long, default_value = "nightly")]
        frequency: String,

        /// Scrub intensity (low, medium, high)
        #[arg(short, long, default_value = "low")]
        intensity: String,

        /// Dry run mode
        #[arg(long, default_value = "false")]
        dry_run: bool,

        /// Auto repair detected issues
        #[arg(long, default_value = "true")]
        auto_repair: bool,
    },

    /// Start Prometheus metrics server
    MetricsServer {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,

        /// Port to listen on
        #[arg(long, default_value = "9090")]
        port: u16,

        /// Bind address
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
    },

    /// Show filesystem status and health
    Status {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },

    /// Display system metrics
    Metrics {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Mount the filesystem
    Mount {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// Mount point
        #[arg(short, long)]
        mountpoint: PathBuf,
    },
    
    /// Run performance benchmarks
    Benchmark {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// File size for writes (bytes)
        #[arg(short, long, default_value = "1048576")]
        file_size: usize,
        
        /// Number of operations
        #[arg(short, long, default_value = "10")]
        operations: usize,
    },
    
    /// Check system health with diagnostics
    Health {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Analyze disk fragmentation
    DefragAnalyze {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Start defragmentation process
    DefragStart {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// Defragmentation intensity (low|medium|high)
        #[arg(short, long, default_value = "medium")]
        intensity: String,
    },
    
    /// Stop defragmentation process
    DefragStop {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Show defragmentation status
    DefragStatus {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Execute TRIM/DISCARD operations
    TrimNow {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// Target disk (optional, trims all if not specified)
        #[arg(short, long)]
        disk: Option<PathBuf>,
    },
    
    /// Show TRIM statistics
    TrimStatus {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
    
    /// Set space reclamation policy
    SetReclamationPolicy {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
        
        /// Policy (aggressive|balanced|conservative|performance)
        #[arg(short, long)]
        policy: String,
    },
    
    /// Show space reclamation status and stats
    ReclamationStatus {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum ScrubDaemonAction {
    /// Start the background scrub daemon
    Start {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,

        /// Scrub intensity (low, medium, high)
        #[arg(short, long, default_value = "low")]
        intensity: String,

        /// Dry run mode (don't actually repair)
        #[arg(long, default_value = "false")]
        dry_run: bool,
    },

    /// Stop the background scrub daemon
    Stop {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },

    /// Show scrub daemon status
    Status {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },

    /// Pause the scrub daemon
    Pause {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },

    /// Resume the scrub daemon
    Resume {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,
    },

    /// Change scrub intensity
    SetIntensity {
        /// Pool directory
        #[arg(short, long)]
        pool: PathBuf,

        /// New intensity level (low, medium, high)
        #[arg(short, long)]
        intensity: String,
    },
}
