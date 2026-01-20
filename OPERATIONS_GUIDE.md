# DynamicFS Operations Guide

## Quick Start

### Initialize a Storage Pool

```bash
# Create a new storage pool with at least one disk
dynamicfs init --pool /data/scfs
dynamicfs add-disk --pool /data/scfs --disk /mnt/disk1

# Verify setup
dynamicfs list-disks --pool /data/scfs
dynamicfs status --pool /data/scfs
```

### Mount the Filesystem

```bash
# Create mount point
mkdir -p /mnt/fs

# Mount the filesystem
dynamicfs mount --pool /data/scfs --mountpoint /mnt/fs

# The filesystem is now available for reads/writes
# Unmount with: fusermount -u /mnt/fs
```

## Daily Operations

### Monitor System Health

```bash
# Check overall health status
dynamicfs health --pool /data/scfs

# JSON output for monitoring integration
dynamicfs --json health --pool /data/scfs

# Get detailed status including disk and extent information
dynamicfs status --pool /data/scfs
```

### Check Performance Metrics

```bash
# View current performance metrics
dynamicfs metrics --pool /data/scfs

# Collect baseline performance (small benchmark)
dynamicfs benchmark --pool /data/scfs --file-size 1048576 --operations 10

# Larger benchmark for realistic testing
dynamicfs --json benchmark --pool /data/scfs --file-size 10485760 --operations 100
```

### Disk Management

```bash
# List all disks
dynamicfs list-disks --pool /data/scfs

# Show redundancy configuration
dynamicfs show-redundancy --pool /data/scfs

# Check disk health
dynamicfs probe-disks --pool /data/scfs

# View extent statistics
dynamicfs list-extents --pool /data/scfs
```

## Maintenance Tasks

### Scrubbing and Repair

```bash
# Verify data integrity (no repairs)
dynamicfs scrub --pool /data/scfs

# Scrub and repair any detected issues
dynamicfs scrub --pool /data/scfs --repair

# View repair progress and results
dynamicfs status --pool /data/scfs
```

### Orphan Cleanup

```bash
# Detect orphaned fragments
dynamicfs detect-orphans --pool /data/scfs

# Check orphan statistics
dynamicfs orphan-stats --pool /data/scfs

# Clean up old orphans (dry-run)
dynamicfs cleanup-orphans --pool /data/scfs --min-age-hours 24 --dry-run

# Actually cleanup orphans
dynamicfs cleanup-orphans --pool /data/scfs --min-age-hours 24
```

## Failure Recovery

### Handle Disk Failures

```bash
# Simulate disk failure for testing
dynamicfs fail-disk --pool /data/scfs --disk /mnt/disk1

# Set disk health manually
dynamicfs set-disk-health --pool /data/scfs --disk /mnt/disk1 --health degraded

# After fixing failed disk, probe to update health status
dynamicfs probe-disks --pool /data/scfs

# Check if recovery started
dynamicfs status --pool /data/scfs
```

### Monitor Rebuild Progress

```bash
# Check extent rebuild status
dynamicfs status --pool /data/scfs

# Monitor with JSON for scripting
while true; do
    dynamicfs --json status --pool /data/scfs | jq '.extents'
    sleep 5
done
```

## Performance Optimization

### Change Redundancy Policy

```bash
# View current policy
dynamicfs policy-status --pool /data/scfs

# Change policy for better performance or reliability
dynamicfs change-policy --pool /data/scfs --policy "replication:2"

# Monitor transition progress
dynamicfs status --pool /data/scfs
```

### Hot/Cold Data Analysis

```bash
# Identify hot extents (frequently accessed)
dynamicfs list-hot --pool /data/scfs

# Identify cold extents (rarely accessed)
dynamicfs list-cold --pool /data/scfs

# Get detailed statistics for specific extent
dynamicfs extent-stats --pool /data/scfs --extent <UUID>
```

## Monitoring Integration

### Prometheus Metrics

All metrics commands support JSON output for easy integration:

```bash
# Get metrics in JSON format
dynamicfs --json metrics --pool /data/scfs

# Example fields available:
# - disk.reads, disk.writes
# - disk.read_bytes, disk.write_bytes
# - disk.errors
# - extents.healthy, extents.degraded, extents.unrecoverable
# - rebuild.attempted, rebuild.successful, rebuild.failed
# - scrub.completed, scrub.issues_found, scrub.repairs_attempted
# - cache.hits, cache.misses
```

### Health Dashboard

```bash
# Create a simple monitoring loop
#!/bin/bash
while true; do
    clear
    echo "=== DynamicFS Health Dashboard ==="
    dynamicfs --json health --pool /data/scfs | jq .
    dynamicfs --json metrics --pool /data/scfs | jq '.disk, .extents, .rebuild'
    sleep 10
done
```

## Troubleshooting

### Unreadable Extents

**Issue**: `dynamicfs status` shows unreadable extents

**Diagnosis**:
```bash
# Check disk health
dynamicfs status --pool /data/scfs

# Run scrub to detect issues
dynamicfs scrub --pool /data/scfs
```

**Recovery**:
- If multiple disks failed: Restore from backup
- If single disk failed: Repair with `dynamicfs scrub --repair`
- Monitor rebuild progress with `dynamicfs health`

### Degraded Extents

**Issue**: Rebuilding slowly or stuck

**Diagnosis**:
```bash
# Monitor rebuild progress
dynamicfs status --pool /data/scfs

# Check disk errors
dynamicfs --json metrics --pool /data/scfs | jq '.disk.errors'
```

**Recovery**:
- Wait for automatic rebuild (can take hours on large pools)
- Ensure disks are healthy: `dynamicfs probe-disks`
- Trigger explicit scrub: `dynamicfs scrub --repair`

### High Orphan Count

**Issue**: Orphaned fragments accumulating

**Diagnosis**:
```bash
dynamicfs orphan-stats --pool /data/scfs

# Check for fragment leaks
dynamicfs detect-orphans --pool /data/scfs
```

**Recovery**:
```bash
# Cleanup orphans older than 24 hours
dynamicfs cleanup-orphans --pool /data/scfs --min-age-hours 24
```

## Backup and Restore

### Creating Backups

```bash
# Backup entire pool metadata
tar -czf /backups/scfs-metadata-$(date +%s).tar.gz /data/scfs/.

# Backup to network location
rsync -av /data/scfs/ backup-server:/backups/scfs-pool/
```

### Restoring from Backup

```bash
# Restore metadata
tar -xzf /backups/scfs-metadata-<timestamp>.tar.gz -C /data/

# Verify integrity
dynamicfs status --pool /data/scfs

# Run scrub to validate
dynamicfs scrub --pool /data/scfs
```

## Advanced Topics

### Performance Tuning

Monitor these metrics for optimization opportunities:

```bash
# Current performance
dynamicfs benchmark --pool /data/scfs --file-size 1048576 --operations 100

# Key metrics:
# - Write throughput (MB/s) - optimize write path
# - Read throughput (MB/s) - optimize read path
# - IOPS - operations per second

# Check cache effectiveness
dynamicfs --json metrics --pool /data/scfs | jq '.cache'
```

### Capacity Planning

```bash
# Monitor disk utilization
dynamicfs --json health --pool /data/scfs | jq '.disks.utilization_percent'

# Add more disks when approaching 80% capacity
dynamicfs add-disk --pool /data/scfs --disk /mnt/disk2
```

### Multi-Tier Strategy

```bash
# Show extent distribution
dynamicfs list-extents --pool /data/scfs

# Monitor hot/cold split
dynamicfs list-hot --pool /data/scfs
dynamicfs list-cold --pool /data/scfs

# Use list-hot to plan caching strategy
```

## Command Reference

### Pool Management
- `init` - Initialize new pool
- `add-disk` - Add disk to pool
- `remove-disk` - Remove disk from pool
- `list-disks` - List all disks
- `probe-disks` - Update disk health status

### Status and Monitoring
- `status` - Filesystem status overview
- `health` - System health check
- `metrics` - Performance metrics
- `benchmark` - Performance testing

### Data Operations
- `list-extents` - List data extents
- `show-redundancy` - Show redundancy config
- `change-policy` - Change redundancy policy
- `policy-status` - Policy transition status

### Data Integrity
- `scrub` - Verify and repair data
- `detect-orphans` - Find orphaned fragments
- `cleanup-orphans` - Delete orphaned fragments
- `orphan-stats` - Orphan statistics

### File Operations
- `mount` - Mount filesystem to directory
- `extent-stats` - Statistics for specific extent

### Hot/Cold Data
- `list-hot` - List frequently accessed extents
- `list-cold` - List rarely accessed extents
- `list-hot` - List hot extents

## Support

For more information:
- See PRODUCTION_ROADMAP.md for feature roadmap
- See CRASH_CONSISTENCY.md for consistency guarantees
- See FINAL_COMPLETION_REPORT.md for implementation details
