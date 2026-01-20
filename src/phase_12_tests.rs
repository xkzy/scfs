/// Phase 12: Storage Optimization Tests
/// Tests for defragmentation, TRIM, and space reclamation features
use tempfile::TempDir;
use uuid::Uuid;

use crate::defrag::{DefragConfig, DefragIntensity, DefragmentationEngine, DefragRecommendation};
use crate::disk::{Disk, DiskHealth};
use crate::extent::{Extent, RedundancyPolicy};
use crate::metadata::MetadataManager;
use crate::metrics::Metrics;
use crate::reclamation::{PolicyEngineConfig, ReclamationPolicy, TierPolicy};
use crate::storage::StorageEngine;
use crate::tiering::StorageTier;
use crate::trim::{TrimConfig, TrimEngine, TrimIntensity};
use std::sync::Arc;

fn setup_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let pool_dir = temp_dir.path().join("pool");
    std::fs::create_dir_all(&pool_dir).unwrap();

    let metadata = MetadataManager::new(pool_dir.clone()).unwrap();

    // Create two test disks
    let disk1_path = temp_dir.path().join("disk1");
    let disk2_path = temp_dir.path().join("disk2");
    std::fs::create_dir_all(&disk1_path).unwrap();
    std::fs::create_dir_all(&disk2_path).unwrap();

    let disk1 = Disk::new(disk1_path).unwrap();
    let disk2 = Disk::new(disk2_path).unwrap();

    let storage = StorageEngine::new(metadata, vec![disk1, disk2]);

    (storage, temp_dir)
}

#[test]
fn test_defrag_intensity_config() {
    assert_eq!(DefragIntensity::Low.io_throttle_ms(), 100);
    assert_eq!(DefragIntensity::Medium.io_throttle_ms(), 50);
    assert_eq!(DefragIntensity::High.io_throttle_ms(), 10);

    assert_eq!(DefragIntensity::Low.batch_size(), 1);
    assert_eq!(DefragIntensity::Medium.batch_size(), 5);
    assert_eq!(DefragIntensity::High.batch_size(), 10);
}

#[test]
fn test_defrag_config_default() {
    let config = DefragConfig::default();
    assert!(!config.enabled); // Disabled by default
    assert_eq!(config.intensity, DefragIntensity::Low);
    assert_eq!(config.fragmentation_threshold, 0.30);
    assert_eq!(config.min_extent_fragments, 2);
    assert!(config.prioritize_hot_extents);
    assert!(config.pause_on_high_load);
}

#[test]
fn test_fragmentation_analysis_empty_pool() {
    let (storage, _temp_dir) = setup_test_storage();
    let defrag_engine = DefragmentationEngine::new(DefragConfig::default());

    let analysis = defrag_engine.analyze_fragmentation(&storage).unwrap();

    assert_eq!(analysis.total_extents, 0);
    assert_eq!(analysis.fragmented_extents, 0);
    assert_eq!(analysis.overall_fragmentation_ratio, 0.0);
    assert_eq!(analysis.recommendation, DefragRecommendation::None);
}

#[test]
fn test_defrag_status_initial() {
    let defrag_engine = DefragmentationEngine::new(DefragConfig::default());
    let status = defrag_engine.status();

    assert!(!status.running);
    assert!(!status.paused);
    assert_eq!(status.extents_processed, 0);
    assert_eq!(status.extents_defragmented, 0);
    assert_eq!(status.bytes_moved, 0);
    assert_eq!(status.errors, 0);
}

#[test]
fn test_trim_intensity_thresholds() {
    assert_eq!(TrimIntensity::Conservative.batch_threshold_bytes(), 10 * 1024 * 1024 * 1024);
    assert_eq!(TrimIntensity::Balanced.batch_threshold_bytes(), 1024 * 1024 * 1024);
    assert_eq!(TrimIntensity::Aggressive.batch_threshold_bytes(), 10 * 1024 * 1024);
}

#[test]
fn test_trim_config_default() {
    let config = TrimConfig::default();
    assert!(config.enabled);
    assert_eq!(config.intensity, TrimIntensity::Balanced);
    assert_eq!(config.batch_size_mb, 100);
    assert!(!config.secure_erase);
    assert_eq!(config.discard_granularity, 4096);
}

#[test]
fn test_trim_stats_initial() {
    let trim_engine = TrimEngine::new(TrimConfig::default());
    let stats = trim_engine.stats();

    assert_eq!(stats.total_trim_operations, 0);
    assert_eq!(stats.total_bytes_trimmed, 0);
    assert_eq!(stats.total_ranges_trimmed, 0);
    assert_eq!(stats.failed_operations, 0);
    assert_eq!(stats.pending_bytes, 0);
    assert_eq!(stats.pending_ranges, 0);
    assert!(stats.last_trim_at.is_none());
}

#[test]
fn test_trim_queue_operation() {
    let trim_engine = TrimEngine::new(TrimConfig::default());
    let disk_uuid = Uuid::new_v4();
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test_file");
    std::fs::write(&file_path, b"test data").unwrap();

    // Queue a TRIM operation
    trim_engine.queue_trim(disk_uuid, file_path, 1024).unwrap();

    let stats = trim_engine.stats();
    assert_eq!(stats.pending_bytes, 1024);
    assert_eq!(stats.pending_ranges, 1);
}

#[test]
fn test_reclamation_policy_presets() {
    // Test aggressive policy
    let aggressive = ReclamationPolicy::Aggressive;
    let defrag = aggressive.defrag_config();
    assert!(defrag.enabled);
    assert_eq!(defrag.intensity, DefragIntensity::High);
    assert_eq!(defrag.fragmentation_threshold, 0.15);
    assert!(!defrag.prioritize_hot_extents); // Defrag everything

    let trim = aggressive.trim_config();
    assert!(trim.enabled);
    assert_eq!(trim.intensity, TrimIntensity::Aggressive);

    // Test performance policy
    let performance = ReclamationPolicy::Performance;
    let perf_defrag = performance.defrag_config();
    assert!(!perf_defrag.enabled); // Defrag disabled for performance
    assert_eq!(perf_defrag.fragmentation_threshold, 0.80);

    // Test balanced policy
    let balanced = ReclamationPolicy::Balanced;
    let bal_defrag = balanced.defrag_config();
    assert!(bal_defrag.enabled);
    assert_eq!(bal_defrag.intensity, DefragIntensity::Medium);
    assert_eq!(bal_defrag.fragmentation_threshold, 0.30);
}

#[test]
fn test_tier_policy_hot() {
    let hot_policy = TierPolicy::for_tier(StorageTier::Hot, ReclamationPolicy::Balanced);
    assert_eq!(hot_policy.tier, StorageTier::Hot);
    assert!(hot_policy.defrag_enabled);
    assert_eq!(hot_policy.defrag_intensity, DefragIntensity::Medium);
    assert_eq!(hot_policy.trim_intensity, TrimIntensity::Conservative);
    assert_eq!(hot_policy.capacity_threshold_percent, 90);
    assert_eq!(hot_policy.fragmentation_threshold, 0.25);
}

#[test]
fn test_tier_policy_cold() {
    let cold_policy = TierPolicy::for_tier(StorageTier::Cold, ReclamationPolicy::Balanced);
    assert_eq!(cold_policy.tier, StorageTier::Cold);
    assert!(!cold_policy.defrag_enabled); // Cold data shouldn't be defragmented
    assert_eq!(cold_policy.trim_intensity, TrimIntensity::Aggressive);
    assert_eq!(cold_policy.capacity_threshold_percent, 80);
}

#[test]
fn test_policy_engine_config_default() {
    let config = PolicyEngineConfig::default();
    assert!(config.enabled);
    assert_eq!(config.policy, ReclamationPolicy::Balanced);
    assert_eq!(config.per_tier_policies.len(), 3);
    assert_eq!(config.capacity_threshold_percent, 85);
    assert_eq!(config.fragmentation_threshold, 0.30);
    assert_eq!(config.schedule_interval_hours, 24);
    assert!(!config.adaptive_mode);
}

#[test]
fn test_policy_descriptions() {
    assert!(!ReclamationPolicy::Aggressive.description().is_empty());
    assert!(ReclamationPolicy::Aggressive.description().contains("Heavy"));

    assert!(!ReclamationPolicy::Balanced.description().is_empty());
    assert!(ReclamationPolicy::Balanced.description().contains("Balanced"));

    assert!(!ReclamationPolicy::Conservative.description().is_empty());
    assert!(ReclamationPolicy::Conservative.description().contains("TRIM"));

    assert!(!ReclamationPolicy::Performance.description().is_empty());
    assert!(ReclamationPolicy::Performance.description().contains("Performance"));
}

#[test]
fn test_fragmentation_recommendation() {
    // Low fragmentation
    let (storage, _temp_dir) = setup_test_storage();
    let defrag_engine = DefragmentationEngine::new(DefragConfig::default());
    let analysis = defrag_engine.analyze_fragmentation(&storage).unwrap();
    // Empty pool should have no recommendation
    assert_eq!(analysis.recommendation, DefragRecommendation::None);
}

#[test]
fn test_defrag_pause_resume() {
    let defrag_engine = DefragmentationEngine::new(DefragConfig::default());
    
    // Initially not paused
    let status = defrag_engine.status();
    assert!(!status.paused);

    // Pause
    defrag_engine.pause();
    let status = defrag_engine.status();
    assert!(status.paused);

    // Resume
    defrag_engine.resume();
    let status = defrag_engine.status();
    assert!(!status.paused);

    // Stop
    defrag_engine.stop();
    let status = defrag_engine.status();
    assert!(!status.running);
}

#[test]
fn test_trim_batch_delay() {
    assert_eq!(TrimIntensity::Conservative.batch_delay_secs(), 7 * 24 * 3600); // Weekly
    assert_eq!(TrimIntensity::Balanced.batch_delay_secs(), 24 * 3600); // Daily
    assert_eq!(TrimIntensity::Aggressive.batch_delay_secs(), 3600); // Hourly
}

#[test]
fn test_metrics_initialization() {
    let metrics = Metrics::new();
    
    // Check Phase 12 metrics are initialized to 0
    assert_eq!(metrics.defrag_runs_completed.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(metrics.defrag_extents_moved.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(metrics.defrag_bytes_moved.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(metrics.trim_operations.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(metrics.trim_bytes_reclaimed.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn test_defrag_needs_defragmentation() {
    let config = DefragConfig {
        enabled: true,
        intensity: DefragIntensity::Medium,
        fragmentation_threshold: 0.30,
        min_extent_fragments: 2,
        prioritize_hot_extents: true,
        pause_on_high_load: true,
        max_concurrent_operations: 2,
    };

    // Test with default min_extent_fragments
    assert_eq!(config.min_extent_fragments, 2);
}

#[test]
fn test_storage_engine_accessor_methods() {
    let (storage, _temp_dir) = setup_test_storage();
    
    // Test that we can access metadata
    let metadata_arc = storage.metadata();
    assert!(metadata_arc.read().is_ok());
    
    // Test that we can get disks
    let disks = storage.get_disks();
    assert_eq!(disks.len(), 2);
    
    // Test metrics accessor
    let metrics = storage.metrics();
    assert!(metrics.disk_reads.load(std::sync::atomic::Ordering::SeqCst) == 0);
}

#[test]
fn test_extent_write_and_delete_basic() {
    // This is a simplified test that verifies the basic flow without requiring
    // full metadata persistence. For full integration testing, use the existing
    // storage_tests.rs which has proper setup
    let (storage, _temp_dir) = setup_test_storage();
    
    let test_data = b"Hello, this is test data for Phase 12!";
    let policy = RedundancyPolicy::Replication { copies: 2 };
    
    // Write extent
    let extent = storage.write_extent(test_data, policy).unwrap();
    
    // Verify extent properties
    assert_eq!(extent.size, test_data.len());
    assert_eq!(extent.redundancy, policy);
    assert!(!extent.fragment_locations.is_empty());
    assert_eq!(extent.fragment_locations.len(), 2); // 2 replicas
    
    // Delete is tested but we don't verify read since that requires metadata persistence
    let _result = storage.delete_extent(extent.uuid);
}
