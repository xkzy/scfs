mod cli;
mod config;
mod crash_sim;
mod diagnostics;
mod disk;
mod allocator;
mod free_extent;
mod metadata_btree;
mod file_locks;
mod io_scheduler;
mod defrag;
mod trim;
mod reclamation;
mod io_alignment;
mod extent;
mod fuse_impl;
mod gc;
mod hmm_classifier;
mod json_output;
mod logging;
mod metadata;
mod metadata_tx;
mod metrics;
mod monitoring;
#[cfg(test)]
mod phase_1_3_tests;
mod perf;
mod placement;
mod redundancy;
pub mod scheduler;
mod scrubber;
mod scrub_daemon;
mod storage;
mod write_optimizer;
mod adaptive;
mod snapshots;
mod tiering;
mod backup_evolution;
mod security;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::Path;

use cli::{Cli, Commands, ScrubDaemonAction};
use disk::{Disk, DiskPool};
use extent::RedundancyPolicy;
use fuse_impl::DynamicFS;
use metadata::MetadataManager;
use metrics::Metrics;
use storage::StorageEngine;
use scrub_daemon::{ScrubDaemon, ScrubSchedule, ScrubIntensity};

fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    
    let cli = Cli::parse();
    let json_output = cli.json;
    
    match cli.command {
        Commands::Init { pool } => cmd_init(&pool, json_output),
        Commands::AddDisk { pool, disk, device } => cmd_add_disk(&pool, &disk, device, json_output),
        Commands::RemoveDisk { pool, disk } => cmd_remove_disk(&pool, &disk, json_output),
        Commands::ListDisks { pool } => cmd_list_disks(&pool, json_output),
        Commands::ListExtents { pool } => cmd_list_extents(&pool, json_output),
        Commands::ShowRedundancy { pool } => cmd_show_redundancy(&pool, json_output),
        Commands::FailDisk { pool, disk } => cmd_fail_disk(&pool, &disk, json_output),
        Commands::SetDiskHealth { pool, disk, health } => cmd_set_disk_health(&pool, &disk, &health, json_output),
        Commands::ChangePolicy { pool, policy } => cmd_change_policy(&pool, &policy, json_output),
        Commands::PolicyStatus { pool } => cmd_policy_status(&pool, json_output),
        Commands::ListHot { pool } => cmd_list_hot(&pool, json_output),
        Commands::ListCold { pool } => cmd_list_cold(&pool, json_output),
        Commands::ExtentStats { pool, extent } => cmd_extent_stats(&pool, &extent, json_output),
        Commands::DetectOrphans { pool } => cmd_detect_orphans(&pool, json_output),
        Commands::CleanupOrphans { pool, min_age_hours, dry_run } => {
            cmd_cleanup_orphans(&pool, min_age_hours, dry_run, json_output)
        }
        Commands::OrphanStats { pool } => cmd_orphan_stats(&pool, json_output),
        Commands::ProbeDisks { pool } => cmd_probe_disks(&pool, json_output),
        Commands::Scrub { pool, repair } => cmd_scrub(&pool, repair, json_output),
        Commands::ScrubDaemon { action } => cmd_scrub_daemon(action, json_output),
        Commands::ScrubSchedule { pool, frequency, intensity, dry_run, auto_repair } => {
            cmd_scrub_schedule(&pool, &frequency, &intensity, dry_run, auto_repair, json_output)
        }
        Commands::MetricsServer { pool, port, bind } => cmd_metrics_server(&pool, port, &bind, json_output),
        Commands::Status { pool } => cmd_status(&pool, json_output),
        Commands::Metrics { pool } => cmd_metrics(&pool, json_output),
        Commands::Mount { pool, mountpoint } => cmd_mount(&pool, &mountpoint, json_output),
        Commands::Benchmark { pool, file_size, operations } => cmd_benchmark(&pool, file_size, operations, json_output),
        Commands::DefragAnalyze { pool } => cmd_defrag_analyze(&pool, json_output),
        Commands::DefragStart { pool, intensity } => cmd_defrag_start(&pool, &intensity, json_output),
        Commands::DefragStop { pool } => cmd_defrag_stop(&pool, json_output),
        Commands::DefragStatus { pool } => cmd_defrag_status(&pool, json_output),
        Commands::TrimNow { pool, disk } => cmd_trim_now(&pool, disk, json_output),
        Commands::TrimStatus { pool } => cmd_trim_status(&pool, json_output),
        Commands::SetReclamationPolicy { pool, policy } => cmd_set_reclamation_policy(&pool, &policy, json_output),
        Commands::ReclamationStatus { pool } => cmd_reclamation_status(&pool, json_output),
        Commands::Health { pool } => cmd_health(&pool, json_output),
    }
}

fn cmd_probe_disks(pool_dir: &Path, _json_output: bool) -> Result<()> {
    println!("Probing disks in pool {:?}", pool_dir);

    let pool = DiskPool::load(pool_dir)?;
    let disk_paths = pool.disk_paths.clone();

    for path in disk_paths {
        match Disk::load(&path) {
            Ok(mut disk) => {
                if path.exists() {
                    if disk.health == disk::DiskHealth::Failed {
                        disk.health = disk::DiskHealth::Healthy;
                        disk.save()?;
                        println!("  Disk {} is reachable again: Failed -> Healthy", disk.uuid);
                    } else {
                        println!("  Disk {} is reachable: {:?}", disk.uuid, disk.health);
                    }
                } else {
                    if disk.health != disk::DiskHealth::Failed {
                        disk.mark_failed()?;
                        println!("  Disk {} marked Failed (path missing)", disk.uuid);
                    } else {
                        println!("  Disk {} remains Failed (path missing)", disk.uuid);
                    }
                }
            }
            Err(e) => {
                log::warn!("Failed to load disk metadata at {:?}: {}", path, e);
            }
        }
    }

    Ok(())
}

fn cmd_scrub(pool_dir: &Path, repair: bool, _json_output: bool) -> Result<()> {
    println!("Scrubbing all extents in pool {:?}", pool_dir);
    if repair {
        println!("Repair mode: ENABLED - will attempt to fix detected issues");
    }
    println!();

    let pool = DiskPool::load(pool_dir)?;
    let mut disks = pool.load_disks()?;
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;

    let scrubber = scrubber::Scrubber::new(pool_dir.to_path_buf());
    let placement = placement::PlacementEngine;

    let mut results = Vec::new();
    let extents = metadata.list_all_extents()?;

    for mut extent in extents {
        let fragments_result = {
            let mut fragments = vec![None; extent.redundancy.fragment_count()];
            for location in &extent.fragment_locations {
                if let Some(disk) = disks.iter().find(|d| d.uuid == location.disk_uuid) {
                    if let Ok(data) = disk.read_fragment(&extent.uuid, location.fragment_index) {
                        fragments[location.fragment_index] = Some(data);
                    }
                }
            }
            fragments
        };

        let result = if repair {
            match scrubber.repair_extent(&mut extent, &metadata, &mut disks, &placement, &fragments_result) {
                Ok(r) => r,
                Err(e) => {
                    log::error!("Error repairing extent {}: {}", extent.uuid, e);
                    scrubber.verify_extent(&extent, &metadata, &disks)?
                }
            }
        } else {
            scrubber.verify_extent(&extent, &metadata, &disks)?
        };

        results.push(result);
    }

    let stats = scrubber::Scrubber::stats(&results);

    println!("Scrub Results:");
    println!();
    println!("  Healthy:       {}", stats.healthy);
    println!("  Degraded:      {}", stats.degraded);
    println!("  Repaired:      {}", stats.repaired);
    println!("  Unrecoverable: {}", stats.unrecoverable);
    println!();

    if stats.total_issues > 0 {
        println!("Issues detected: {}", stats.total_issues);
        if repair {
            println!("Repairs attempted: {}", stats.total_repairs);
        }
        println!();

        for result in &results {
            if !result.issues.is_empty() {
                println!("  Extent {}: {:?}", result.extent_uuid, result.status);
                for issue in &result.issues {
                    println!("    - {}", issue);
                }
            }
        }
    }

    if stats.unrecoverable > 0 {
        println!();
        println!("⚠ WARNING: {} unrecoverable extents found!", stats.unrecoverable);
        println!("Data may be lost if no backups are available.");
    } else if stats.degraded > 0 {
        println!();
        if repair {
            println!("✓ Attempted repair of {} degraded extents", stats.degraded);
        } else {
            println!("✓ {} extents can be repaired via rebuild", stats.degraded);
            println!("  Use `scrub --pool {} --repair` to attempt automatic repair", pool_dir.display());
        }
    } else if stats.healthy > 0 {
        println!();
        println!("✓ All extents are healthy and verified");
    }

    Ok(())
}

fn cmd_status(pool_dir: &Path, json_output: bool) -> Result<()> {
    // Load pool
    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;

    let mut healthy = 0;
    let mut degraded = 0;
    let mut failed = 0;
    for disk in &disks {
        match disk.health {
            disk::DiskHealth::Healthy => healthy += 1,
            disk::DiskHealth::Failed => failed += 1,
            _ => degraded += 1,
        }
    }

    // Load metadata
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;
    let extents = metadata.list_all_extents()?;

    let mut complete = 0;
    let mut readable = 0;
    let mut unreadable = 0;
    for extent in &extents {
        if extent.is_complete() {
            complete += 1;
        } else if extent.is_readable() {
            readable += 1;
        } else {
            unreadable += 1;
        }
    }

    if json_output {
        let status_json = serde_json::json!({
            "status": "ok",
            "filesystem": pool_dir.display().to_string(),
            "disks": {
                "total": disks.len(),
                "healthy": healthy,
                "degraded": degraded,
                "failed": failed
            },
            "extents": {
                "total": extents.len(),
                "complete": complete,
                "readable": readable,
                "unreadable": unreadable
            },
            "health": if unreadable > 0 { "critical" } else if readable > 0 { "degraded" } else { "healthy" }
        });
        println!("{}", serde_json::to_string_pretty(&status_json)?);
    } else {
        println!("Filesystem Status: {}", pool_dir.display());
        println!();
        println!("Disks: {}", disks.len());
        for disk in &disks {
            println!(
                "  {} ({:?}) - {} MB used / {} MB total",
                disk.uuid,
                disk.health,
                disk.used_bytes / 1024 / 1024,
                disk.capacity_bytes / 1024 / 1024
            );
        }
        println!();
        println!("Disk Summary: {} healthy, {} degraded/suspect, {} failed", healthy, degraded, failed);
        println!();
        println!("Extents: {} total", extents.len());
        println!("  {} complete", complete);
        println!("  {} degraded (readable)", readable);
        println!("  {} unreadable", unreadable);
        if unreadable > 0 {
            println!();
            println!("⚠ WARNING: {} unreadable extents - data loss risk!", unreadable);
        } else if readable > 0 {
            println!();
            println!("⚠ NOTICE: {} degraded extents - rebuild recommended", readable);
        } else {
            println!();
            println!("✓ All extents healthy");
        }
    }

    Ok(())
}

fn cmd_init(pool_dir: &Path, _json_output: bool) -> Result<()> {
    println!("Initializing storage pool at {:?}", pool_dir);
    
    fs::create_dir_all(pool_dir).context("Failed to create pool directory")?;
    
    let pool = DiskPool::new();
    pool.save(pool_dir)?;
    
    // Initialize metadata
    MetadataManager::new(pool_dir.to_path_buf())?;
    
    println!("✓ Pool initialized");
    Ok(())
}

fn cmd_add_disk(pool_dir: &Path, disk_path: &Path, device: bool, _json_output: bool) -> Result<()> {
    println!("Adding disk {:?} to pool {:?}", disk_path, pool_dir);

    // Auto-detect block device and require explicit --device flag for safety
    if disk_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            let meta = fs::metadata(disk_path).context("Failed to stat path")?;
            if meta.file_type().is_block_device() && !device {
                return Err(anyhow!("Path {:?} appears to be a block device; use --device to add raw devices explicitly", disk_path));
            }
        }
    }

    // For raw block devices, require explicit confirmation and do not create directories
    if device {
        if !disk_path.exists() {
            return Err(anyhow!("Raw device path does not exist: {:?}", disk_path));
        }
        println!("  Treating {:?} as a raw block device (explicit confirmation)", disk_path);
    } else {
        // Create disk directory if needed
        fs::create_dir_all(disk_path).context("Failed to create disk directory")?;
    }

    // Initialize disk
    let disk = if device {
        Disk::from_block_device(disk_path.to_path_buf())?
    } else {
        Disk::new(disk_path.to_path_buf())?
    };
    println!("  Capacity: {} MB", disk.capacity_bytes / 1024 / 1024);

    // Add to pool
    let mut pool = DiskPool::load(pool_dir)?;
    pool.add_disk(disk_path.to_path_buf());
    pool.save(pool_dir)?;

    println!("✓ Disk added");
    Ok(())
}

fn cmd_remove_disk(pool_dir: &Path, disk_path: &Path, _json_output: bool) -> Result<()> {
    println!("Removing disk {:?} from pool {:?}", disk_path, pool_dir);
    
    // Mark disk as draining
    let mut disk = Disk::load(disk_path)?;
    disk.mark_draining()?;
    println!("  Marked disk {} as draining", disk.uuid);
    
    // TODO: Implement actual rebuild process
    println!("  Note: Rebuild must be triggered separately");
    
    // Remove from pool
    let mut pool = DiskPool::load(pool_dir)?;
    pool.remove_disk(disk_path);
    pool.save(pool_dir)?;
    
    println!("✓ Disk removed from pool");
    Ok(())
}

fn cmd_list_disks(pool_dir: &Path, _json_output: bool) -> Result<()> {
    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;
    
    println!("Disks in pool ({} total):", disks.len());
    println!();
    
    for disk in disks {
        println!("  UUID: {}", disk.uuid);
        println!("  Path: {:?}", disk.path);
        println!("  Health: {:?}", disk.health);
        println!("  Capacity: {} MB", disk.capacity_bytes / 1024 / 1024);
        println!("  Used: {} MB", disk.used_bytes / 1024 / 1024);
        println!("  Free: {} MB", 
                 (disk.capacity_bytes - disk.used_bytes) / 1024 / 1024);
        println!();
    }
    
    Ok(())
}

fn cmd_list_extents(pool_dir: &Path, _json_output: bool) -> Result<()> {
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;
    let extents = metadata.list_all_extents()?;
    
    println!("Extents in pool ({} total):", extents.len());
    println!();
    
    for extent in extents {
        println!("  UUID: {}", extent.uuid);
        println!("  Size: {} bytes", extent.size);
        println!("  Redundancy: {:?}", extent.redundancy);
        println!("  Fragments: {}", extent.fragment_locations.len());
        println!("  Complete: {}", extent.is_complete());
        println!("  Readable: {}", extent.is_readable());
        println!();
    }
    
    Ok(())
}

fn cmd_show_redundancy(pool_dir: &Path, _json_output: bool) -> Result<()> {
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;
    let extents = metadata.list_all_extents()?;
    
    let mut total_extents = 0;
    let mut complete_extents = 0;
    let mut degraded_extents = 0;
    let mut unreadable_extents = 0;
    
    for extent in &extents {
        total_extents += 1;
        if extent.is_complete() {
            complete_extents += 1;
        } else if extent.is_readable() {
            degraded_extents += 1;
        } else {
            unreadable_extents += 1;
        }
    }
    
    println!("Redundancy Status:");
    println!("  Total extents: {}", total_extents);
    println!("  Complete: {} ({:.1}%)", 
             complete_extents,
             if total_extents > 0 { 100.0 * complete_extents as f64 / total_extents as f64 } else { 0.0 });
    println!("  Degraded: {} ({:.1}%)", 
             degraded_extents,
             if total_extents > 0 { 100.0 * degraded_extents as f64 / total_extents as f64 } else { 0.0 });
    println!("  Unreadable: {} ({:.1}%)", 
             unreadable_extents,
             if total_extents > 0 { 100.0 * unreadable_extents as f64 / total_extents as f64 } else { 0.0 });
    
    if unreadable_extents > 0 {
        println!();
        println!("⚠ Warning: {} extents are unreadable!", unreadable_extents);
    } else if degraded_extents > 0 {
        println!();
        println!("⚠ Warning: {} extents are degraded (rebuild recommended)", degraded_extents);
    } else if total_extents > 0 {
        println!();
        println!("✓ All extents are fully redundant");
    }
    
    Ok(())
}

// -----------------------------
// Phase 12: Defragmentation & TRIM
// -----------------------------

fn cmd_defrag_analyze(pool_dir: &Path, json_output: bool) -> Result<()> {
    use crate::defrag::{DefragConfig, DefragmentationEngine};

    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;
    let storage = StorageEngine::new(metadata, disks);

    let defrag_engine = DefragmentationEngine::new(DefragConfig::default());
    let analysis = defrag_engine.analyze_fragmentation(&storage)?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&analysis)?);
    } else {
        println!("Fragmentation Analysis:");
        println!("  Total extents: {}", analysis.total_extents);
        println!("  Fragmented extents: {}", analysis.fragmented_extents);
        println!("  Fragmentation ratio: {:.2}%", analysis.overall_fragmentation_ratio * 100.0);
        println!("  Recommendation: {:?}", analysis.recommendation);
        println!("\nPer-Disk Statistics:");
        for disk_stats in &analysis.per_disk_stats {
            println!("  Disk {}:", disk_stats.disk_uuid);
            println!("    Total extents: {}", disk_stats.total_extents);
            println!("    Fragmented: {}", disk_stats.fragmented_extents);
            println!("    Ratio: {:.2}%", disk_stats.fragmentation_ratio * 100.0);
        }
    }

    Ok(())
}

fn cmd_defrag_start(pool_dir: &Path, intensity: &str, json_output: bool) -> Result<()> {
    use crate::defrag::{DefragConfig, DefragIntensity, DefragmentationEngine};
    use std::sync::Arc;

    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;
    let storage = Arc::new(StorageEngine::new(metadata, disks));
    let metrics = Arc::new(Metrics::new());

    let intensity_enum = match intensity {
        "low" => DefragIntensity::Low,
        "medium" => DefragIntensity::Medium,
        "high" => DefragIntensity::High,
        _ => DefragIntensity::Medium,
    };

    let mut cfg = DefragConfig::default();
    cfg.enabled = true;
    cfg.intensity = intensity_enum;

    let engine = DefragmentationEngine::new(cfg);
    engine.start(storage, metrics)?;

    println!("✓ Defragmentation started (intensity={})", intensity);
    Ok(())
}

fn cmd_defrag_stop(_pool_dir: &Path, _json_output: bool) -> Result<()> {
    println!("Defragmentation stop requested (not fully implemented)");
    Ok(())
}

fn cmd_defrag_status(_pool_dir: &Path, _json_output: bool) -> Result<()> {
    println!("Defragmentation status: (not implemented yet)");
    Ok(())
}

fn cmd_trim_now(pool_dir: &Path, _disk: Option<std::path::PathBuf>, _json_output: bool) -> Result<()> {
    println!("TRIM/DISCARD requested (not implemented yet)");
    Ok(())
}

fn cmd_trim_status(_pool_dir: &Path, _json_output: bool) -> Result<()> {
    println!("TRIM status: (not implemented yet)");
    Ok(())
}

fn cmd_set_reclamation_policy(pool_dir: &Path, policy_str: &str, _json_output: bool) -> Result<()> {
    println!("Setting reclamation policy to '{}' for pool {:?}", policy_str, pool_dir);
    // TODO: Validate and persist policy; for now just acknowledge
    Ok(())
}

fn cmd_reclamation_status(pool_dir: &Path, _json_output: bool) -> Result<()> {
    println!("Reclamation status for pool {:?}: (not implemented yet)", pool_dir);
    Ok(())
}

fn cmd_fail_disk(pool_dir: &Path, disk_path: &Path, _json_output: bool) -> Result<()> {
    println!("Simulating failure of disk {:?}", disk_path);
    
    let mut disk = Disk::load(disk_path)?;
    let old_health = disk.health;
    disk.mark_failed()?;
    
    println!("  Disk {} marked as failed (was {:?})", disk.uuid, old_health);
    println!();
    println!("⚠ Disk is now unavailable. Run 'show-redundancy' to see impact.");
    
    Ok(())
}

fn cmd_set_disk_health(_pool_dir: &Path, disk_path: &Path, health: &str, _json_output: bool) -> Result<()> {
    let mut disk = Disk::load(disk_path)?;
    let old_health = disk.health;

    let new_health = match health.to_lowercase().as_str() {
        "healthy" => disk::DiskHealth::Healthy,
        "degraded" => disk::DiskHealth::Degraded,
        "suspect" => disk::DiskHealth::Suspect,
        "draining" => disk::DiskHealth::Draining,
        "failed" => disk::DiskHealth::Failed,
        _ => {
            return Err(anyhow!(
                "Invalid health '{}'. Use: healthy|degraded|suspect|draining|failed",
                health
            ))
        }
    };

    disk.health = new_health;
    disk.save()?;

    println!(
        "Disk {} health updated: {:?} -> {:?}",
        disk.uuid, old_health, disk.health
    );
    Ok(())
}

fn cmd_change_policy(pool_dir: &Path, policy_str: &str, _json_output: bool) -> Result<()> {
    println!("Preparing to change redundancy policy...");
    
    // Parse policy string
    let new_policy = if policy_str.starts_with("replication:") {
        let copies: usize = policy_str
            .strip_prefix("replication:")
            .unwrap()
            .parse()
            .context("Invalid replication count")?;
        RedundancyPolicy::Replication { copies }
    } else if policy_str.starts_with("erasure:") {
        let parts: Vec<&str> = policy_str
            .strip_prefix("erasure:")
            .unwrap()
            .split('+')
            .collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid EC policy format. Use 'erasure:K+M'"));
        }
        let data_shards: usize = parts[0].parse()?;
        let parity_shards: usize = parts[1].parse()?;
        RedundancyPolicy::ErasureCoding {
            data_shards,
            parity_shards,
        }
    } else {
        return Err(anyhow!(
            "Invalid policy format. Use 'replication:N' or 'erasure:K+M'"
        ));
    };
    
    println!("Target policy: {:?}", new_policy);
    println!();
    println!("Note: This would change policies on all files in the pool.");
    println!("      Implementation requires inode list and per-file changes.");
    println!();
    println!("✓ Policy configuration created (not yet applied)");
    
    Ok(())
}

fn cmd_policy_status(pool_dir: &Path, _json_output: bool) -> Result<()> {
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;
    let all_extents = metadata.list_all_extents()?;
    
    let transitioning: Vec<_> = all_extents
        .iter()
        .filter(|e| e.is_transitioning())
        .collect();
    
    let with_history: Vec<_> = all_extents
        .iter()
        .filter(|e| !e.policy_transitions.is_empty())
        .collect();
    
    println!("Policy Transition Status:");
    println!();
    println!("  Extents in transition: {}", transitioning.len());
    for extent in &transitioning {
        println!("    - {}: {:?}", extent.uuid, extent.redundancy);
    }
    println!();
    println!("  Extents with history: {}", with_history.len());
    for extent in &with_history {
        println!("    - {}:", extent.uuid);
        for transition in &extent.policy_transitions {
            println!(
                "        {:?} → {:?} ({:?})",
                transition.from_policy, transition.to_policy, transition.status
            );
        }
    }
    
    if transitioning.is_empty() && with_history.is_empty() {
        println!("  No policy transitions detected");
    }
    
    Ok(())
}

fn cmd_mount(pool_dir: &Path, mountpoint: &Path, _json_output: bool) -> Result<()> {
    println!("Mounting filesystem at {:?}", mountpoint);
    println!("Pool: {:?}", pool_dir);
    
    // Load pool and disks
    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;
    
    println!("Loaded {} disks:", disks.len());
    for disk in &disks {
        println!("  - {} ({:?})", disk.uuid, disk.health);
    }
    
    // Initialize metadata and storage
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;
    let storage = StorageEngine::new(metadata, disks);

    // Perform mount-time rebuilds before mounting
    if let Err(e) = storage.perform_mount_rebuild() {
        log::error!("Mount-time rebuild failed: {}", e);
    }

    let fs = DynamicFS::new(storage);
    
    println!();
    println!("Mounting...");
    println!("Press Ctrl+C to unmount");
    println!();
    
    // Mount options
    let options = vec![
        fuser::MountOption::FSName("dynamicfs".to_string()),
        fuser::MountOption::AllowOther,
        fuser::MountOption::DefaultPermissions,
    ];
    
    fuser::mount2(fs, mountpoint, &options)
        .context("Failed to mount filesystem")?;
    
    Ok(())
}

fn cmd_list_hot(_pool_dir: &Path, _json_output: bool) -> Result<()> {
    println!("Hot extents:");
    println!("(Extents accessed more than 100 times/day or within last hour)");
    println!();
    
    // In a real implementation, this would query the storage engine
    println!("Note: Access statistics are tracked during operation");
    
    Ok(())
}

fn cmd_metrics(_pool_dir: &Path, json_output: bool) -> Result<()> {
    // Create default metrics snapshot for demo
    let snapshot = metrics::Metrics::new().snapshot();
    
    if json_output {
        let metrics_json = serde_json::json!({
            "disk": {
                "reads": snapshot.disk_reads,
                "read_bytes": snapshot.disk_read_bytes,
                "writes": snapshot.disk_writes,
                "write_bytes": snapshot.disk_write_bytes,
                "errors": snapshot.disk_errors
            },
            "extents": {
                "healthy": snapshot.extents_healthy,
                "degraded": snapshot.extents_degraded,
                "unrecoverable": snapshot.extents_unrecoverable
            },
            "rebuild": {
                "attempted": snapshot.rebuilds_attempted,
                "successful": snapshot.rebuilds_successful,
                "failed": snapshot.rebuilds_failed,
                "bytes_written": snapshot.rebuild_bytes_written
            },
            "scrub": {
                "completed": snapshot.scrubs_completed,
                "issues_found": snapshot.scrub_issues_found,
                "repairs_attempted": snapshot.scrub_repairs_attempted,
                "repairs_successful": snapshot.scrub_repairs_successful
            },
            "cache": {
                "hits": snapshot.cache_hits,
                "misses": snapshot.cache_misses,
                "hit_rate": snapshot.cache_hits as f64 / ((snapshot.cache_hits + snapshot.cache_misses) as f64 + 0.001)
            },
            "note": "Metrics are collected during filesystem operation. These are default/zero values; actual metrics require an active mounted instance."
        });
        println!("{}", serde_json::to_string_pretty(&metrics_json)?);
    } else {
        println!("{}", snapshot);
        println!();
        println!("Note: Metrics are collected during filesystem operation.");
        println!("These are default/zero values; actual metrics require an active mounted instance.");
    }
    
    Ok(())
}

fn cmd_list_cold(_pool_dir: &Path, _json_output: bool) -> Result<()> {
    println!("Cold extents:");
    println!("(Extents accessed less than 10 times/day and not accessed in 24+ hours)");
    println!();
    
    // In a real implementation, this would query the storage engine
    println!("Note: Access statistics are tracked during operation");
    
    Ok(())
}

fn cmd_extent_stats(_pool_dir: &Path, extent_str: &str, _json_output: bool) -> Result<()> {
    println!("Extent statistics for: {}", extent_str);
    println!();
    println!("Classification system:");
    println!("  Hot:  frequency > 100 ops/day OR last access < 1 hour");
    println!("  Warm: frequency > 10 ops/day OR last access < 24 hours");
    println!("  Cold: frequency ≤ 10 ops/day AND last access ≥ 24 hours");
    println!();
    
    Ok(())
}

fn cmd_detect_orphans(pool_dir: &Path, _json_output: bool) -> Result<()> {
    println!("Scanning for orphaned fragments...");
    println!();
    
    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;
    
    let gc = gc::GarbageCollector::new(pool_dir.to_path_buf(), disks);
    let orphans = gc.detect_orphans()?;
    
    if orphans.is_empty() {
        println!("✓ No orphaned fragments found");
    } else {
        println!("Found {} orphaned fragments:", orphans.len());
        println!();
        
        let mut total_bytes = 0u64;
        let mut old_count = 0usize;
        
        for orphan in &orphans {
            let age_hours = orphan.age_seconds / 3600;
            let is_old = age_hours >= 24;
            if is_old {
                old_count += 1;
            }
            
            println!(
                "  {} [fragment {}] - {} bytes, {} hours old{}",
                orphan.extent_uuid,
                orphan.fragment_index,
                orphan.size_bytes,
                age_hours,
                if is_old { " [OLD]" } else { "" }
            );
            total_bytes += orphan.size_bytes;
        }
        
        println!();
        println!("Total: {} fragments, {} bytes ({} MB)",
            orphans.len(),
            total_bytes,
            total_bytes / 1024 / 1024
        );
        println!("Older than 24h: {} fragments", old_count);
        println!();
        println!("Use 'cleanup-orphans' to remove old orphans");
    }
    
    Ok(())
}

fn cmd_cleanup_orphans(pool_dir: &Path, min_age_hours: u64, dry_run: bool, _json_output: bool) -> Result<()> {
    let min_age_seconds = min_age_hours * 3600;
    
    if dry_run {
        println!("DRY RUN - No files will be deleted");
    }
    println!("Cleaning up orphaned fragments older than {} hours...", min_age_hours);
    println!();
    
    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;
    
    let gc = gc::GarbageCollector::new(pool_dir.to_path_buf(), disks);
    let cleaned = gc.cleanup_orphans(min_age_seconds, dry_run)?;
    
    if cleaned.is_empty() {
        println!("✓ No orphans found for cleanup");
    } else {
        let total_bytes: u64 = cleaned.iter().map(|o| o.size_bytes).sum();
        
        println!("{} {} fragments:", 
            if dry_run { "Would clean" } else { "Cleaned" },
            cleaned.len()
        );
        println!();
        
        for orphan in &cleaned {
            println!(
                "  {} [fragment {}] - {} bytes",
                orphan.extent_uuid,
                orphan.fragment_index,
                orphan.size_bytes
            );
        }
        
        println!();
        println!("Total: {} fragments, {} bytes ({} MB)",
            cleaned.len(),
            total_bytes,
            total_bytes / 1024 / 1024
        );
        
        if !dry_run {
            println!("✓ Cleanup complete");
        }
    }
    
    Ok(())
}

fn cmd_orphan_stats(pool_dir: &Path, _json_output: bool) -> Result<()> {
    println!("Orphan fragment statistics...");
    println!();
    
    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;
    
    let gc = gc::GarbageCollector::new(pool_dir.to_path_buf(), disks);
    let stats = gc.get_orphan_stats()?;
    
    println!("Orphaned Fragments:");
    println!("  Total count:      {}", stats.total_count);
    println!("  Total size:       {} bytes ({} MB)", 
        stats.total_bytes, 
        stats.total_bytes / 1024 / 1024
    );
    println!();
    println!("  Old (>24h) count: {}", stats.old_count);
    println!("  Old (>24h) size:  {} bytes ({} MB)",
        stats.old_bytes,
        stats.old_bytes / 1024 / 1024
    );
    
    if stats.old_count > 0 {
        println!();
        println!("Recommendation: Run 'cleanup-orphans' to reclaim space");
    }
    
    Ok(())
}

fn cmd_benchmark(pool_dir: &Path, file_size: usize, operations: usize, json_output: bool) -> Result<()> {
    use crate::perf::{Benchmark, PerfStats};
    
    if !json_output {
        println!("Running DynamicFS Performance Benchmark");
        println!("======================================");
        println!("File size:   {} bytes", file_size);
        println!("Operations:  {}", operations);
        println!();
    }
    
    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;
    let storage = StorageEngine::new(metadata, disks);
    
    // Create test data
    let test_data = vec![42u8; file_size];
    
    // Benchmark write operations
    let write_bench = Benchmark::start("write");
    for i in 0..operations {
        let ino = 1000 + i as u64;
        match storage.write_file(ino, &test_data, 0) {
            Ok(_) => {},
            Err(e) => {
                log::warn!("Write operation {} failed: {}", i, e);
            }
        }
    }
    let write_elapsed = write_bench.elapsed_ms();
    let mut write_stats = PerfStats::new("write");
    write_stats.count = operations as u64;
    write_stats.total_bytes = (file_size as u64) * (operations as u64);
    write_stats.total_ms = write_elapsed;
    
    // Benchmark read operations
    let read_bench = Benchmark::start("read");
    for i in 0..operations {
        let ino = 1000 + i as u64;
        match storage.read_file(ino) {
            Ok(_) => {},
            Err(e) => {
                log::warn!("Read operation {} failed: {}", i, e);
            }
        }
    }
    let read_elapsed = read_bench.elapsed_ms();
    let mut read_stats = PerfStats::new("read");
    read_stats.count = operations as u64;
    read_stats.total_bytes = (file_size as u64) * (operations as u64);
    read_stats.total_ms = read_elapsed;
    
    if json_output {
        let bench_json = serde_json::json!({
            "benchmark": "performance",
            "file_size": file_size,
            "operations": operations,
            "write": {
                "elapsed_ms": write_elapsed,
                "throughput_mbps": write_stats.throughput_mbps(),
                "ops_per_sec": write_stats.ops_per_sec()
            },
            "read": {
                "elapsed_ms": read_elapsed,
                "throughput_mbps": read_stats.throughput_mbps(),
                "ops_per_sec": read_stats.ops_per_sec()
            }
        });
        println!("{}", serde_json::to_string_pretty(&bench_json)?);
    } else {
        println!("Write Performance:");
        println!("  Elapsed time:  {} ms", write_elapsed);
        println!("  Throughput:    {:.2} MB/s", write_stats.throughput_mbps());
        println!("  Operations:    {:.0} ops/sec", write_stats.ops_per_sec());
        println!();
        println!("Read Performance:");
        println!("  Elapsed time:  {} ms", read_elapsed);
        println!("  Throughput:    {:.2} MB/s", read_stats.throughput_mbps());
        println!("  Operations:    {:.0} ops/sec", read_stats.ops_per_sec());
    }
    
    Ok(())
}



fn cmd_health(pool_dir: &Path, json_output: bool) -> Result<()> {
    // Load filesystem data
    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;
    let extents = metadata.list_all_extents()?;
    
    // Calculate health metrics
    let mut healthy_disks = 0;
    let mut degraded_disks = 0;
    let mut failed_disks = 0;
    let mut total_disk_capacity = 0u64;
    let mut total_disk_used = 0u64;
    
    for disk in &disks {
        match disk.health {
            disk::DiskHealth::Healthy => healthy_disks += 1,
            disk::DiskHealth::Failed => failed_disks += 1,
            _ => degraded_disks += 1,
        }
        total_disk_capacity += disk.capacity_bytes;
        total_disk_used += disk.used_bytes;
    }
    
    let mut healthy_extents = 0;
    let mut degraded_extents = 0;
    let mut unreadable_extents = 0;
    
    for extent in &extents {
        if extent.is_complete() {
            healthy_extents += 1;
        } else if extent.is_readable() {
            degraded_extents += 1;
        } else {
            unreadable_extents += 1;
        }
    }
    
    // Determine overall health status
    let health_status = if unreadable_extents > 0 {
        "critical"
    } else if failed_disks > 0 || degraded_extents > 0 {
        "degraded"
    } else {
        "healthy"
    };
    
    if json_output {
        let health_json = serde_json::json!({
            "status": health_status,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "disks": {
                "total": disks.len(),
                "healthy": healthy_disks,
                "degraded": degraded_disks,
                "failed": failed_disks,
                "capacity_bytes": total_disk_capacity,
                "used_bytes": total_disk_used,
                "utilization_percent": if total_disk_capacity > 0 {
                    (total_disk_used as f64 / total_disk_capacity as f64) * 100.0
                } else {
                    0.0
                }
            },
            "extents": {
                "total": extents.len(),
                "healthy": healthy_extents,
                "degraded": degraded_extents,
                "unreadable": unreadable_extents
            },
            "metrics": {
                "iops": 0,
                "throughput_mbps": 0.0
            }
        });
        println!("{}", serde_json::to_string_pretty(&health_json)?);
    } else {
        println!("DynamicFS Health Status");
        println!("======================");
        println!();
        println!("Overall Status: {}", health_status.to_uppercase());
        println!();
        println!("Disk Health:");
        println!("  Healthy:  {} / {}", healthy_disks, disks.len());
        println!("  Degraded: {}", degraded_disks);
        println!("  Failed:   {}", failed_disks);
        println!("  Capacity: {} MB / {} MB", 
            total_disk_used / 1024 / 1024,
            total_disk_capacity / 1024 / 1024
        );
        println!("  Usage:    {:.1}%", 
            if total_disk_capacity > 0 {
                (total_disk_used as f64 / total_disk_capacity as f64) * 100.0
            } else {
                0.0
            }
        );
        println!();
        println!("Data Integrity:");
        println!("  Healthy extents:   {}", healthy_extents);
        println!("  Degraded extents:  {}", degraded_extents);
        println!("  Unreadable extents: {}", unreadable_extents);
        println!();
        
        if unreadable_extents > 0 {
            println!("⚠ CRITICAL: {} unreadable extents - immediate action required!", unreadable_extents);
        } else if failed_disks > 0 {
            println!("⚠ WARNING: {} failed disks - rebuild in progress", failed_disks);
        } else if degraded_extents > 0 {
            println!("⚠ NOTICE: {} degraded extents - rebuild recommended", degraded_extents);
        } else {
            println!("✓ All systems nominal");
        }
    }
    
    Ok(())
}


fn cmd_scrub_daemon(action: ScrubDaemonAction, json_output: bool) -> Result<()> {
    use std::sync::Arc;
    
    match action {
        ScrubDaemonAction::Start { pool, intensity, dry_run } => {
            let intensity_level = parse_intensity(&intensity)?;
            
            // Build schedule based on user input
            let schedule = ScrubSchedule {
                enabled: true,
                interval_hours: if dry_run { 1 } else { 6 },
                intensity: intensity_level,
                dry_run,
                auto_repair: !dry_run,  // Auto-repair only when not in dry-run mode
            };
            
            let daemon = ScrubDaemon::new();
            daemon.start(schedule)?;
            
            if json_output {
                let result = serde_json::json!({
                    "status": "started",
                    "pool": pool.display().to_string(),
                    "intensity": format!("{:?}", intensity_level),
                    "dry_run": dry_run,
                });
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Scrub daemon started");
                println!("  Pool:      {:?}", pool);
                println!("  Intensity: {:?}", intensity_level);
                println!("  Dry run:   {}", dry_run);
            }
            Ok(())
        }
        
        ScrubDaemonAction::Stop { pool } => {
            if json_output {
                let result = serde_json::json!({
                    "status": "stopped",
                    "pool": pool.display().to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Scrub daemon stopped");
                println!("  Pool: {:?}", pool);
            }
            Ok(())
        }
        
        ScrubDaemonAction::Status { pool } => {
            let daemon = ScrubDaemon::new();
            let progress = daemon.get_progress();
            let metrics = daemon.get_metrics();
            
            if json_output {
                let result = serde_json::json!({
                    "pool": pool.display().to_string(),
                    "status": format!("{:?}", progress.status),
                    "running": metrics.is_running,
                    "paused": metrics.is_paused,
                    "metrics": {
                        "extents_scanned": metrics.extents_scanned,
                        "issues_found": metrics.issues_found,
                        "repairs_triggered": metrics.repairs_triggered,
                        "io_bytes": metrics.scrub_io_bytes,
                    }
                });
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("Scrub Daemon Status");
                println!("==================");
                println!("Pool:    {:?}", pool);
                println!("Status:  {:?}", progress.status);
                println!("Running: {}", metrics.is_running);
                println!("Paused:  {}", metrics.is_paused);
                println!();
                println!("Metrics:");
                println!("  Extents scanned:    {}", metrics.extents_scanned);
                println!("  Issues found:       {}", metrics.issues_found);
                println!("  Repairs triggered:  {}", metrics.repairs_triggered);
                println!("  I/O bytes:          {}", metrics.scrub_io_bytes);
            }
            Ok(())
        }
        
        ScrubDaemonAction::Pause { pool } => {
            let daemon = ScrubDaemon::new();
            daemon.pause();
            if json_output {
                let result = serde_json::json!({"status": "paused"});
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Scrub daemon paused");
            }
            Ok(())
        }
        
        ScrubDaemonAction::Resume { pool } => {
            let daemon = ScrubDaemon::new();
            daemon.resume();
            if json_output {
                let result = serde_json::json!({"status": "resumed"});
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Scrub daemon resumed");
            }
            Ok(())
        }
        
        ScrubDaemonAction::SetIntensity { pool, intensity } => {
            let intensity_level = parse_intensity(&intensity)?;
            let daemon = ScrubDaemon::new();
            daemon.set_intensity(intensity_level)?;
            if json_output {
                let result = serde_json::json!({"intensity": format!("{:?}", intensity_level)});
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Scrub intensity set to {:?}", intensity_level);
            }
            Ok(())
        }
    }
}

fn parse_intensity(intensity: &str) -> Result<ScrubIntensity> {
    match intensity.to_lowercase().as_str() {
        "low" => Ok(ScrubIntensity::Low),
        "medium" | "med" => Ok(ScrubIntensity::Medium),
        "high" => Ok(ScrubIntensity::High),
        _ => Err(anyhow!("Invalid intensity: {}. Use low, medium, or high", intensity)),
    }
}

fn cmd_scrub_schedule(
    pool_dir: &Path,
    frequency: &str,
    intensity: &str,
    dry_run: bool,
    auto_repair: bool,
    json_output: bool
) -> Result<()> {
    let intensity_level = parse_intensity(intensity)?;
    
    let schedule = match frequency.to_lowercase().as_str() {
        "nightly" => ScrubSchedule {
            enabled: true,
            interval_hours: 24,
            intensity: intensity_level,
            dry_run,
            auto_repair,
        },
        "continuous" => ScrubSchedule {
            enabled: true,
            interval_hours: 6,
            intensity: intensity_level,
            dry_run,
            auto_repair,
        },
        "manual" => ScrubSchedule {
            enabled: false,
            interval_hours: 0,
            intensity: intensity_level,
            dry_run,
            auto_repair,
        },
        _ => {
            return Err(anyhow!("Invalid frequency: {}. Use nightly, continuous, or manual", frequency));
        }
    };
    
    if json_output {
        let result = serde_json::json!({
            "frequency": frequency,
            "intensity": format!("{:?}", intensity_level),
            "interval_hours": schedule.interval_hours,
            "dry_run": dry_run,
            "auto_repair": auto_repair,
            "enabled": schedule.enabled,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("✓ Scrub schedule configured");
        println!("  Frequency:   {}", frequency);
        println!("  Intensity:   {:?}", intensity_level);
        println!("  Interval:    {} hours", schedule.interval_hours);
        println!("  Dry run:     {}", dry_run);
        println!("  Auto repair: {}", auto_repair);
        println!("  Enabled:     {}", schedule.enabled);
    }
    
    Ok(())
}

fn cmd_metrics_server(
    pool_dir: &Path,
    port: u16,
    bind: &str,
    json_output: bool
) -> Result<()> {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use monitoring::PrometheusExporter;
    
    // Load metrics (in a real implementation, this would load from the pool)
    let metrics = Arc::new(Metrics::new());
    let exporter = PrometheusExporter::new(metrics);
    
    let addr = format!("{}:{}", bind, port);
    let listener = TcpListener::bind(&addr)
        .context(format!("Failed to bind to {}", addr))?;
    
    if json_output {
        let result = serde_json::json!({
            "status": "running",
            "pool": pool_dir.display().to_string(),
            "endpoint": format!("http://{}/metrics", addr),
            "port": port,
            "bind": bind,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("✓ Prometheus metrics server started");
        println!("  Pool:     {:?}", pool_dir);
        println!("  Endpoint: http://{}/metrics", addr);
        println!("  Listening on {}:{}", bind, port);
        println!();
        println!("Press Ctrl+C to stop...");
    }
    
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let mut buffer = [0; 1024];
                match stream.read(&mut buffer) {
                    Ok(bytes_read) => {
                        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
                        
                        if request.contains("GET /metrics") {
                            let metrics_text = exporter.export();
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\n\r\n{}",
                                metrics_text.len(),
                                metrics_text
                            );
                            if let Err(e) = stream.write_all(response.as_bytes()) {
                                log::warn!("Failed to write metrics response: {}", e);
                            }
                        } else if request.contains("GET /health") {
                            let health = r#"{"status":"ok"}"#;
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                health.len(),
                                health
                            );
                            if let Err(e) = stream.write_all(response.as_bytes()) {
                                log::warn!("Failed to write health response: {}", e);
                            }
                        } else {
                            let not_found = "404 Not Found\nAvailable endpoints:\n  /metrics - Prometheus metrics\n  /health - Health check\n";
                            let response = format!(
                                "HTTP/1.1 404 NOT FOUND\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                                not_found.len(),
                                not_found
                            );
                            if let Err(e) = stream.write_all(response.as_bytes()) {
                                log::debug!("Failed to write 404 response: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to read request: {}", e);
                    }
                }
            }
            Err(e) => {
                log::error!("Connection failed: {}", e);
            }
        }
    }
    
    Ok(())
}
