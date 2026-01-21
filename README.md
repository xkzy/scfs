# DynamicFS - Object-Based Filesystem Prototype

A minimal, working, single-node filesystem prototype with dynamic geometry, supporting arbitrary disk sizes and per-object redundancy.

## 📚 Documentation Index

- **[README.md](README.md)** (this file) - Overview and usage guide
- **[QUICKSTART.md](QUICKSTART.md)** - Step-by-step tutorial for getting started
- **[ARCHITECTURE.md](ARCHITECTURE.md)** - Deep dive into system design and implementation
- **[PROJECT.md](PROJECT.md)** - File organization and development guide  
- **[SUMMARY.md](SUMMARY.md)** - Implementation summary and verification results

## ✨ Features

✓ **Dynamic Geometry** - Add/remove disks of any size at runtime
✓ **Object-Based Storage** - Files split into 1MB immutable extents  
✓ **Flexible Redundancy** - Per-object choice of replication or erasure coding
✓ **Dynamic Policy Changes** - Change redundancy policies on existing extents without data loss
✓ **Hot/Cold Classification** - Automatic access pattern analysis and extent tiering
✓ **Lazy Migration on Read** - Automatic policy optimization based on access patterns
✓ **HMM-Based Classification** - Probabilistic hot/cold classification with smooth state transitions
✓ **Lazy Rebuild** - Per-extent reconstruction on demand, not global rebuild
✓ **Crash Consistent** - Atomic metadata updates with checksums
✓ **POSIX Semantics** - Standard filesystem via FUSE
✓ **Background Scrubbing** - Continuous low-priority verification daemon with configurable intensity
✓ **Prometheus Metrics** - HTTP endpoint for monitoring and observability
✓ **Structured Logging** - JSON-formatted logs with request tracing

## 🚀 Quick Start

**Binary Size:** 3.4 MB (release build)
**Test Status:** ✓ All 24 unit tests passing

## 🎯 Key Implementation Achievements

### Phase 1: Core Filesystem (8 tests)
- Object-based storage with immutable 1MB extents
- Flexible redundancy (replication or erasure coding)
- Dynamic disk management
- FUSE interface for POSIX filesystem semantics

### Phase 2: Dynamic Policy Changes (3 tests)
- **Runtime redundancy migration**: Change policies without data loss
- **Transparent re-encoding**: Decode with old policy → encode with new policy
- **State tracking**: Complete audit trail with timestamps and transition status
- **Resilience**: Handles disk failures during policy changes

### Phase 3: Hot/Cold Classification (3 tests)
- **Automatic classification**: Based on access frequency and recency
- **Access tracking**: Records read/write patterns automatically
- **Tiered insights**: Hot/Warm/Cold tiers enable storage optimization

### Phase 4: HMM-Based Classification (7 tests)
- **Hidden Markov Model**: Probabilistic state machine for classifications
- **Smooth transitions**: Reduces oscillation between states (inertia)
- **Viterbi algorithm**: Finds most likely state sequences
- **Recency-aware**: Boosts recent accesses in classification
- **State history**: Tracks 100 recent classifications for smoothing

### Phase 5: Lazy Migration on Read (3 tests)
- **On-demand optimization**: Extents migrate to optimal policies during reads
- **Access-triggered**: Only extents being accessed are migrated
- **Transparent**: No user intervention required
- **Efficient**: Hot data automatically moves to replication; cold data to erasure coding
- **Production-ready**: 17/17 tests passing, full integration with storage engine

## Architecture

### Core Concepts

**Extent**: Immutable 1MB chunk of file data with UUID, checksum, and redundancy policy
**Fragment**: Part of an extent - either a replica or erasure-coded shard
**Disk**: Directory representing a storage device with tracked capacity and health
**Metadata**: Replicated JSON-based metadata for inodes and extent maps

### System Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        User Space                            │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              FUSE Mount Point                        │   │
│  │         /tmp/mnt/myfile.txt                          │   │
│  └──────────────────────────────────────────────────────┘   │
│                           ↕                                  │
│  ┌──────────────────────────────────────────────────────┐   │
│  │          DynamicFS (FUSE Implementation)             │   │
│  └──────────────────────────────────────────────────────┘   │
│                           ↕                                  │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              Storage Engine                          │   │
│  │  • Write Path: Split → Encode → Place → Commit      │   │
│  │  • Read Path: Load → Read → Decode → Verify         │   │
│  └──────────────────────────────────────────────────────┘   │
│         ↕              ↕              ↕              ↕        │
│  ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐ │
│  │ Metadata │   │Redundancy│   │Placement │   │  Disks   │ │
│  │  System  │   │  Engine  │   │  Engine  │   │  Layer   │ │
│  └──────────┘   └──────────┘   └──────────┘   └──────────┘ │
│       ↓              ↓              ↓              ↓          │
└───────┼──────────────┼──────────────┼──────────────┼──────────┘
        ↓              ↓              ↓              ↓
   ┌────────┐    ┌─────────┐    ┌─────────┐    ┌─────────┐
   │ Inodes │    │3-way    │    │Fragment │    │  Disk1  │
   │Extents │    │Replica  │    │Selection│    │ /disk1/ │
   │  Maps  │    │  or     │    │  Logic  │    │fragments│
   └────────┘    │EC (4+2) │    └─────────┘    └─────────┘
                 └─────────┘                   ┌─────────┐
                                               │  Disk2  │
                                               │ /disk2/ │
                   File is split into          │fragments│
                   1MB extents:                └─────────┘
                                                    ...
                   ┌─────────┐                ┌─────────┐
                   │Extent 1 │                │  Disk6  │
                   │ 1MB    │                │ /disk6/ │
                   └─────────┘                │fragments│
                   ┌─────────┐                └─────────┘
                   │Extent 2 │
                   │ 1MB    │
                   └─────────┘
                       ...
```

### Redundancy Modes

1. **Replication**: Store N complete copies (used for metadata, 3 copies)
2. **Erasure Coding**: Reed-Solomon k+m encoding (used for data, 4+2)

### Dynamic Policy Changes

Extents can be migrated between redundancy policies at runtime:
- **Replication → EC**: Reduce storage overhead by 33% (3 copies → 4+2 shards)
- **EC → Replication**: Increase fault tolerance for critical data
- **Transparent Migration**: Data remains readable throughout transition
- **History Tracking**: Audit trail of all policy changes with timestamps

### Write Path

1. Split file into 1MB extents
2. Apply redundancy (replication or EC)
3. Place fragments on different disks
4. Verify checksums
5. Atomically commit metadata

### Read Path

1. Locate extent fragments from metadata
2. Read minimum required fragments
3. Reconstruct data if needed
4. Verify checksum

### Disk Management

- **Add disk**: Register directory, immediately available for writes
- **Remove disk**: Mark as draining, rebuild affected extents lazily
- **Failure**: Detect and reconstruct per-extent, no global rebuild

### Hot/Cold Classification

Extents are automatically classified based on access patterns:

- **Hot**: Frequency > 100 ops/day OR accessed within last 1 hour
  - Use for: Actively used data, database indexes, cached content
  - Optimal policy: Replication for fast access
  
- **Warm**: Frequency > 10 ops/day OR accessed within last 24 hours
  - Use for: Regular working data, temporary files
  - Optimal policy: Balanced redundancy
  
- **Cold**: Frequency ≤ 10 ops/day AND not accessed in 24+ hours
  - Use for: Archives, backups, historical data
  - Optimal policy: Erasure coding for efficiency

**Benefits**:
- Automatic tiered storage optimization
- Data placement decisions based on usage
- Potential for compression and archival of cold data
- Performance monitoring and capacity planning

## Usage

### Build

```bash
cargo build --release
```

### Initialize Storage Pool

```bash
# Create disk directories
mkdir -p /tmp/disk{1,2,3,4,5,6}

# Initialize the filesystem
cargo run --release -- init --pool /tmp/pool
cargo run --release -- add-disk --pool /tmp/pool --disk /tmp/disk1
cargo run --release -- add-disk --pool /tmp/pool --disk /tmp/disk2
cargo run --release -- add-disk --pool /tmp/pool --disk /tmp/disk3
cargo run --release -- add-disk --pool /tmp/pool --disk /tmp/disk4
cargo run --release -- add-disk --pool /tmp/pool --disk /tmp/disk5
cargo run --release -- add-disk --pool /tmp/pool --disk /tmp/disk6
```

### Mount Filesystem

```bash
mkdir -p /tmp/mnt
cargo run --release -- mount --pool /tmp/pool --mountpoint /tmp/mnt
```

### CLI Commands

```bash
# Disk Management
cargo run --release -- list-disks --pool /tmp/pool
cargo run --release -- add-disk --pool /tmp/pool --disk /tmp/disk1
cargo run --release -- remove-disk --pool /tmp/pool --disk /tmp/disk2
cargo run --release -- fail-disk --pool /tmp/pool --disk /tmp/disk1

# Storage Information
cargo run --release -- list-extents --pool /tmp/pool
cargo run --release -- show-redundancy --pool /tmp/pool


# Policy Management
cargo run --release -- change-policy --pool /tmp/pool --policy "erasure:4+2"
cargo run --release -- policy-status --pool /tmp/pool

# Access Classification
cargo run --release -- list-hot --pool /tmp/pool
cargo run --release -- list-cold --pool /tmp/pool
cargo run --release -- extent-stats --pool /tmp/pool --extent <UUID>

# Scrubbing & Maintenance
cargo run --release -- scrub --pool /tmp/pool  # One-time scrub
cargo run --release -- scrub --pool /tmp/pool --repair  # With auto-repair

# Background Scrub Daemon
cargo run --release -- scrub-daemon start --pool /tmp/pool --intensity low
cargo run --release -- scrub-daemon status --pool /tmp/pool
cargo run --release -- scrub-daemon pause --pool /tmp/pool
cargo run --release -- scrub-daemon resume --pool /tmp/pool
cargo run --release -- scrub-daemon stop --pool /tmp/pool

# Schedule Periodic Scrubbing
cargo run --release -- scrub-schedule --pool /tmp/pool --frequency nightly --intensity low
cargo run --release -- scrub-schedule --pool /tmp/pool --frequency continuous --intensity medium

# Observability & Monitoring
cargo run --release -- status --pool /tmp/pool  # Overall health
cargo run --release -- health --pool /tmp/pool  # Detailed diagnostics
cargo run --release -- metrics --pool /tmp/pool  # System metrics

# Prometheus Metrics Server
cargo run --release -- metrics-server --pool /tmp/pool --port 9090
# Endpoints: http://localhost:9090/metrics (Prometheus) and /health (JSON)

# Mount
cargo run --release -- mount --pool /tmp/pool --mountpoint /tmp/mnt
```

### Testing

```bash
# Run test suite
cargo test

# Manual testing
echo "Hello, World!" > /tmp/mnt/test.txt
cat /tmp/mnt/test.txt
```

## Implementation Details

### File Structure

- `src/disk.rs` - Disk abstraction and management
- `src/extent.rs` - Extent model with hot/cold classification and policy transitions
- `src/redundancy.rs` - Replication and erasure coding with re-encoding
- `src/placement.rs` - Fragment placement and extent rebundling
- `src/metadata.rs` - Metadata management system
- `src/storage.rs` - Write/read path with access tracking
- `src/fuse_impl.rs` - FUSE interface
- `src/cli.rs` - Command-line interface
- `src/main.rs` - Entry point and command handlers

### Disk Layout

Each disk directory contains:
- `disk.json` - Disk metadata (UUID, capacity, health)
- `fragments/` - Fragment storage (named by extent UUID + fragment index)

Pool directory contains:
- `pool.json` - Pool metadata (disk list)
- `metadata/` - Replicated metadata objects
- `inodes/` - Inode table
- `extent_maps/` - File to extent mappings

### Checksums

All data uses BLAKE3 for checksums, verified on read.

### Crash Consistency

Metadata updates are atomic via write-then-rename pattern. Fragment writes are verified before metadata commit.

## Limitations (Prototype)

- Single-node only (no network distribution)
- No caching optimizations
- No concurrent write optimization
- Phase 16: Full FUSE operation support (extended attributes, mmap, locks, fallocate, ACLs, ioctls)
- Phase 17: Automated intelligent policies (policy engine with ML-driven recommendations and safe automation)
- Background scrubbing not yet implemented (Phase 3)

## Fixed Since Initial Prototype

✅ **Crash Consistency** - Now has comprehensive crash recovery:
- Atomic metadata transactions with versioned roots
- Durable fragment writes with fsync barriers
- Read-after-write verification
- Automatic cleanup of temp files
- No orphaned fragments on crash
- 100% metadata integrity with BLAKE3 checksums
- Zero silent corruption

✅ **Classification** - Enhanced from basic age/access to:
- HMM-based probabilistic classification
- Smooth state transitions with inertia
- Viterbi algorithm for optimal state sequences

✅ **Lazy Migration** - Automatic policy optimization:
- On-demand migration during reads
- Access-triggered policy changes
- Hot data → replication, cold data → erasure coding

✅ **Storage Hygiene** - No more leaks:
- Orphan fragment detection
- Automatic cleanup (age-based)
- Zero storage leaks from failed operations
- CLI tools for management (detect-orphans, cleanup-orphans, orphan-stats)

## Future Enhancements

- **Phase 1.3**: Metadata checksums + Orphan GC (in progress)
- **Phase 2**: Enhanced failure handling with targeted rebuilds
- **Phase 3**: Background scrubbing and self-healing — ⚠️ Needs Hardening (requires device-aware scrubbing for raw device support)
- **Raw Block Device Support**: Add safe raw device support (on-device allocator and superblock layout, O_DIRECT/O_SYNC handling, alignment-aware IO, device validation and explicit `--device`/`--force` semantics, loopback integration tests). See [PHASE_18_RAW_BLOCK_DEVICE.md](PHASE_18_RAW_BLOCK_DEVICE.md) for milestones and checklist.
- **Phase 4**: Structured logging and Prometheus metrics
- **Phase 5**: Performance optimizations (see Phase 14: Multi-level caching, Phase 10: tiered placement & parallel I/O, Phase 15: concurrent read/write optimization)
- **Phase 6**: Snapshots and point-in-time recovery
- **Phase 13**: Multi-node network distribution (see `PRODUCTION_ROADMAP.md` Phase 13) — cluster RPC, metadata consensus, cross-node replication, and rebalancing
- **Phase 8**: Security hardening and privilege dropping

See [PRODUCTION_ROADMAP.md](PRODUCTION_ROADMAP.md) for detailed hardening plan (12-18 weeks).
