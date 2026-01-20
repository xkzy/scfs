# Background Scrubbing Guide

## Overview

DynamicFS includes a background scrubbing system that continuously verifies data integrity, detects corruption, and automatically repairs issues. This guide covers how to use the scrubbing features for production deployments.

## Features

- **Continuous Verification**: Background daemon scans extents for corruption
- **Configurable Intensity**: Low, medium, or high priority levels
- **Automatic Repair**: Optional auto-repair of detected issues
- **Throttling**: I/O throttling to minimize impact on foreground operations
- **Metrics**: Comprehensive metrics for monitoring and alerting
- **Scheduling**: Flexible scheduling options (nightly, continuous, manual)

## Quick Start

### One-Time Scrub

Run a one-time scrub of all extents:

```bash
# Verify only (no repairs)
cargo run --release -- scrub --pool /tmp/pool

# Verify and auto-repair issues
cargo run --release -- scrub --pool /tmp/pool --repair

# JSON output for automation
cargo run --release -- scrub --pool /tmp/pool --repair --json
```

### Background Daemon

Start a background scrub daemon for continuous verification:

```bash
# Start daemon with low intensity (recommended for production)
cargo run --release -- scrub-daemon start --pool /tmp/pool --intensity low

# Check daemon status
cargo run --release -- scrub-daemon status --pool /tmp/pool

# Pause/resume daemon
cargo run --release -- scrub-daemon pause --pool /tmp/pool
cargo run --release -- scrub-daemon resume --pool /tmp/pool

# Change intensity on the fly
cargo run --release -- scrub-daemon set-intensity --pool /tmp/pool --intensity medium

# Stop daemon
cargo run --release -- scrub-daemon stop --pool /tmp/pool
```

### Scheduled Scrubbing

Configure periodic scrubbing schedules:

```bash
# Nightly scrub (runs every 24 hours)
cargo run --release -- scrub-schedule --pool /tmp/pool \
  --frequency nightly \
  --intensity low \
  --auto-repair true

# Continuous scrub (runs every 6 hours)
cargo run --release -- scrub-schedule --pool /tmp/pool \
  --frequency continuous \
  --intensity medium \
  --auto-repair false

# Dry-run mode (test without making changes)
cargo run --release -- scrub-schedule --pool /tmp/pool \
  --frequency nightly \
  --intensity low \
  --dry-run true
```

## Intensity Levels

### Low Intensity
- **Use case**: Production systems with active workloads
- **I/O throttle**: 100ms between operations
- **Batch size**: 1 extent at a time
- **Priority**: Background (nice 10)
- **Impact**: Minimal impact on foreground I/O

### Medium Intensity
- **Use case**: Off-peak hours or lightly loaded systems
- **I/O throttle**: 50ms between operations
- **Batch size**: 5 extents at a time
- **Priority**: Normal (nice 5)
- **Impact**: Moderate impact on foreground I/O

### High Intensity
- **Use case**: Emergency corruption detection or maintenance windows
- **I/O throttle**: 10ms between operations
- **Batch size**: 20 extents at a time
- **Priority**: High (nice 1)
- **Impact**: Significant impact on foreground I/O

## Monitoring

### CLI Status

Check scrub daemon status and metrics:

```bash
# Text output
cargo run --release -- scrub-daemon status --pool /tmp/pool

# JSON output for monitoring systems
cargo run --release -- scrub-daemon status --pool /tmp/pool --json
```

Example output:
```
Scrub Daemon Status
==================
Status: Running
Running: true
Paused:  false

Metrics:
  Extents scanned:    1234
  Issues found:       5
  Repairs triggered:  5
  I/O bytes:          1048576000
```

### Prometheus Metrics

Start the Prometheus metrics server:

```bash
cargo run --release -- metrics-server --pool /tmp/pool --port 9090
```

Available metrics:
- `dynamicfs_scrubs_completed` - Total completed scrubs
- `dynamicfs_scrub_issues_found` - Total issues detected
- `dynamicfs_scrub_repairs_attempted` - Total repair attempts
- `dynamicfs_scrub_repairs_successful` - Successful repairs
- `dynamicfs_extents_healthy` - Current healthy extent count
- `dynamicfs_extents_degraded` - Current degraded extent count
- `dynamicfs_extents_unrecoverable` - Current unrecoverable extent count

Access metrics at:
- `http://localhost:9090/metrics` - Prometheus format
- `http://localhost:9090/health` - Health check (JSON)

## Best Practices

### Production Deployments

1. **Start with Low Intensity**: Begin with low intensity to minimize impact
2. **Monitor Impact**: Watch foreground I/O performance during scrubbing
3. **Schedule Nightly**: Run during off-peak hours (nightly schedule)
4. **Enable Auto-Repair**: Let the system automatically fix issues (low-risk)
5. **Set Up Alerts**: Configure Prometheus alerts for unrecoverable extents

### Recommended Schedule

```bash
# Production: Nightly scrub with low intensity
cargo run --release -- scrub-schedule --pool /tmp/pool \
  --frequency nightly \
  --intensity low \
  --auto-repair true
```

### Emergency Response

If corruption is suspected:

```bash
# 1. Run immediate high-priority scrub with repair
cargo run --release -- scrub --pool /tmp/pool --repair

# 2. Check status
cargo run --release -- status --pool /tmp/pool

# 3. If issues persist, start continuous scrubbing
cargo run --release -- scrub-daemon start --pool /tmp/pool --intensity high

# 4. Monitor progress
watch -n 5 'cargo run --release -- scrub-daemon status --pool /tmp/pool'
```

## Troubleshooting

### Daemon Won't Start

Check if another instance is already running:
```bash
cargo run --release -- scrub-daemon status --pool /tmp/pool
```

Stop existing daemon if needed:
```bash
cargo run --release -- scrub-daemon stop --pool /tmp/pool
```

### High I/O Impact

Reduce intensity level:
```bash
cargo run --release -- scrub-daemon set-intensity --pool /tmp/pool --intensity low
```

Or pause temporarily:
```bash
cargo run --release -- scrub-daemon pause --pool /tmp/pool
```

### Repairs Failing

Check disk health:
```bash
cargo run --release -- probe-disks --pool /tmp/pool
cargo run --release -- status --pool /tmp/pool
```

Some repairs may require manual intervention if:
- Multiple disks have failed
- Insufficient redundancy for reconstruction
- Underlying hardware issues

## Integration

### Systemd Service

Create `/etc/systemd/system/dynamicfs-scrub.service`:

```ini
[Unit]
Description=DynamicFS Background Scrubber
After=network.target

[Service]
Type=simple
User=dynamicfs
ExecStart=/usr/local/bin/dynamicfs scrub-daemon start --pool /var/lib/dynamicfs/pool --intensity low
ExecStop=/usr/local/bin/dynamicfs scrub-daemon stop --pool /var/lib/dynamicfs/pool
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
sudo systemctl enable dynamicfs-scrub
sudo systemctl start dynamicfs-scrub
sudo systemctl status dynamicfs-scrub
```

### Monitoring Integration

#### Prometheus
Add scrape config to `prometheus.yml`:
```yaml
scrape_configs:
  - job_name: 'dynamicfs'
    static_configs:
      - targets: ['localhost:9090']
```

#### Alerting Rules
Create alerting rules for critical conditions:
```yaml
groups:
  - name: dynamicfs_alerts
    rules:
      - alert: UnrecoverableExtents
        expr: dynamicfs_extents_unrecoverable > 0
        for: 5m
        annotations:
          summary: "DynamicFS has unrecoverable extents"
          description: "{{ $value }} extents cannot be recovered"
```

## See Also

- [PRODUCTION_ROADMAP.md](PRODUCTION_ROADMAP.md) - Overall roadmap
- [OPERATIONS_GUIDE.md](OPERATIONS_GUIDE.md) - Operations guide
- [README.md](README.md) - Main documentation
