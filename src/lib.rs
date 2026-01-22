pub use crate::metadata::*;
pub use crate::storage::*;
pub use crate::disk::*;
// Re-export modules used by integration tests

mod cli;
mod config;
mod crash_sim;
mod diagnostics;
pub mod disk;
// test_utils moved into tests/unit; expose helper shim to compile test-only APIs
#[cfg(test)]
pub mod test_utils {
    include!("../tests/unit/test_utils.rs");
}

mod allocator;
mod on_device_allocator;
mod free_extent;
mod metadata_btree;
mod file_locks;
mod io_scheduler;
mod defrag;
mod trim;
mod reclamation;
mod io_alignment;
mod extent;
#[cfg(not(target_os = "windows"))]
mod fuse_impl;
mod gc;
mod hmm_classifier;
mod json_output;
mod logging;
mod metadata;
mod metadata_tx;
mod metrics;
mod monitoring;
mod storage_engine;
mod placement;
mod redundancy;
mod scheduler;
mod scrubber;
mod scrub_daemon;
pub mod storage;
mod write_optimizer;
mod adaptive;
mod snapshots;
mod tiering;
mod backup_evolution;
mod security;

// Phase 9.1: Cross-Platform Storage Abstraction modules
pub mod fs_interface;
pub mod path_utils;
pub mod mount;

// Phase 10: Mixed Storage Speed Optimization
pub mod data_cache;
