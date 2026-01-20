mod cli;
mod crash_sim;
mod disk;
mod extent;
mod fuse_impl;
mod gc;
mod hmm_classifier;
mod metadata;
mod metadata_tx;
#[cfg(test)]
mod phase_1_3_tests;
mod placement;
mod redundancy;
mod scrubber;
mod storage;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use std::fs;
use std::path::Path;

use cli::{Cli, Commands};
use disk::{Disk, DiskPool};
use extent::RedundancyPolicy;
use fuse_impl::DynamicFS;
use metadata::MetadataManager;
use storage::StorageEngine;

fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Init { pool } => cmd_init(&pool),
        Commands::AddDisk { pool, disk } => cmd_add_disk(&pool, &disk),
        Commands::RemoveDisk { pool, disk } => cmd_remove_disk(&pool, &disk),
        Commands::ListDisks { pool } => cmd_list_disks(&pool),
        Commands::ListExtents { pool } => cmd_list_extents(&pool),
        Commands::ShowRedundancy { pool } => cmd_show_redundancy(&pool),
        Commands::FailDisk { pool, disk } => cmd_fail_disk(&pool, &disk),
        Commands::SetDiskHealth { pool, disk, health } => cmd_set_disk_health(&pool, &disk, &health),
        Commands::ChangePolicy { pool, policy } => cmd_change_policy(&pool, &policy),
        Commands::PolicyStatus { pool } => cmd_policy_status(&pool),
        Commands::ListHot { pool } => cmd_list_hot(&pool),
        Commands::ListCold { pool } => cmd_list_cold(&pool),
        Commands::ExtentStats { pool, extent } => cmd_extent_stats(&pool, &extent),
        Commands::DetectOrphans { pool } => cmd_detect_orphans(&pool),
        Commands::CleanupOrphans { pool, min_age_hours, dry_run } => {
            cmd_cleanup_orphans(&pool, min_age_hours, dry_run)
        }
        Commands::OrphanStats { pool } => cmd_orphan_stats(&pool),
        Commands::ProbeDisks { pool } => cmd_probe_disks(&pool),
        Commands::Scrub { pool } => cmd_scrub(&pool),
        Commands::Mount { pool, mountpoint } => cmd_mount(&pool, &mountpoint),
    }
}

fn cmd_probe_disks(pool_dir: &Path) -> Result<()> {
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

fn cmd_scrub(pool_dir: &Path) -> Result<()> {
    println!("Scrubbing all extents in pool {:?}", pool_dir);
    println!();

    let pool = DiskPool::load(pool_dir)?;
    let disks = pool.load_disks()?;
    let metadata = MetadataManager::new(pool_dir.to_path_buf())?;

    let scrubber = scrubber::Scrubber::new(pool_dir.to_path_buf());
    let results = scrubber.scrub_all(&metadata, &disks)?;

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
        println!("✓ {} extents can be repaired via rebuild", stats.degraded);
    } else if stats.healthy > 0 {
        println!();
        println!("✓ All extents are healthy and verified");
    }

    Ok(())
}

fn cmd_init(pool_dir: &Path) -> Result<()> {
    println!("Initializing storage pool at {:?}", pool_dir);
    
    fs::create_dir_all(pool_dir).context("Failed to create pool directory")?;
    
    let pool = DiskPool::new();
    pool.save(pool_dir)?;
    
    // Initialize metadata
    MetadataManager::new(pool_dir.to_path_buf())?;
    
    println!("✓ Pool initialized");
    Ok(())
}

fn cmd_add_disk(pool_dir: &Path, disk_path: &Path) -> Result<()> {
    println!("Adding disk {:?} to pool {:?}", disk_path, pool_dir);
    
    // Create disk directory if needed
    fs::create_dir_all(disk_path).context("Failed to create disk directory")?;
    
    // Initialize disk
    let disk = Disk::new(disk_path.to_path_buf())?;
    println!("  Disk UUID: {}", disk.uuid);
    println!("  Capacity: {} MB", disk.capacity_bytes / 1024 / 1024);
    
    // Add to pool
    let mut pool = DiskPool::load(pool_dir)?;
    pool.add_disk(disk_path.to_path_buf());
    pool.save(pool_dir)?;
    
    println!("✓ Disk added");
    Ok(())
}

fn cmd_remove_disk(pool_dir: &Path, disk_path: &Path) -> Result<()> {
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

fn cmd_list_disks(pool_dir: &Path) -> Result<()> {
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

fn cmd_list_extents(pool_dir: &Path) -> Result<()> {
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

fn cmd_show_redundancy(pool_dir: &Path) -> Result<()> {
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

fn cmd_fail_disk(pool_dir: &Path, disk_path: &Path) -> Result<()> {
    println!("Simulating failure of disk {:?}", disk_path);
    
    let mut disk = Disk::load(disk_path)?;
    let old_health = disk.health;
    disk.mark_failed()?;
    
    println!("  Disk {} marked as failed (was {:?})", disk.uuid, old_health);
    println!();
    println!("⚠ Disk is now unavailable. Run 'show-redundancy' to see impact.");
    
    Ok(())
}

fn cmd_set_disk_health(_pool_dir: &Path, disk_path: &Path, health: &str) -> Result<()> {
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

fn cmd_change_policy(pool_dir: &Path, policy_str: &str) -> Result<()> {
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

fn cmd_policy_status(pool_dir: &Path) -> Result<()> {
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

fn cmd_mount(pool_dir: &Path, mountpoint: &Path) -> Result<()> {
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

fn cmd_list_hot(_pool_dir: &Path) -> Result<()> {
    println!("Hot extents:");
    println!("(Extents accessed more than 100 times/day or within last hour)");
    println!();
    
    // In a real implementation, this would query the storage engine
    println!("Note: Access statistics are tracked during operation");
    
    Ok(())
}

fn cmd_list_cold(_pool_dir: &Path) -> Result<()> {
    println!("Cold extents:");
    println!("(Extents accessed less than 10 times/day and not accessed in 24+ hours)");
    println!();
    
    // In a real implementation, this would query the storage engine
    println!("Note: Access statistics are tracked during operation");
    
    Ok(())
}

fn cmd_extent_stats(_pool_dir: &Path, extent_str: &str) -> Result<()> {
    println!("Extent statistics for: {}", extent_str);
    println!();
    println!("Classification system:");
    println!("  Hot:  frequency > 100 ops/day OR last access < 1 hour");
    println!("  Warm: frequency > 10 ops/day OR last access < 24 hours");
    println!("  Cold: frequency ≤ 10 ops/day AND last access ≥ 24 hours");
    println!();
    
    Ok(())
}

fn cmd_detect_orphans(pool_dir: &Path) -> Result<()> {
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

fn cmd_cleanup_orphans(pool_dir: &Path, min_age_hours: u64, dry_run: bool) -> Result<()> {
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

fn cmd_orphan_stats(pool_dir: &Path) -> Result<()> {
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


