# Phase 18 — Raw Block Device Support

Status: Planned
Priority: High (Safety-first)
Estimated Effort: 3-4 sprints (6-8 weeks)

Scope
- Add safe, explicit support for raw block devices (e.g. /dev/loopX, /dev/sdX) as disks in the pool.
- Support device-aware I/O semantics: O_DIRECT (or O_SYNC fallback), TRIM support, alignment-aware reads/writes.
- Implement an on-device layout (superblock + allocator) for storing fragments by offset.
- Provide explicit CLI safety controls (`--device`, `--force`) and safety checks (device signature detection, exclusive lock).

CLI: Add a global `--direct-io <auto|always|never>` flag to control whether the system prefers O_DIRECT aligned I/O (also available via `DYNAMICFS_DIRECT_IO` environment variable).
- Integration tests using loopback devices and crash-safety scenarios.

Milestones
1. Phase 18.1 — Detection & Safe Stubbing (1 sprint)
   - Add `DiskKind` enum: Directory | BlockDevice
   - Detect block device on `add-disk` and persist kind in `disk.json`
   - Accept block devices but reject write operations until allocator exists (safe-reject stub)
   - CLI: `--device` flag required for adding device paths, clear warning message
   - Tests: unit tests for detection, add-disk refusal semantics

2. Phase 18.2 — I/O primitives & Alignment (1 sprint)
   - Implement device I/O layer (open with O_DIRECT and O_SYNC fallback)
   - Alignment helper utilities and tests for BLKSSZGET/BLKBSZGET
   - Ensure read/write helpers enforce alignment and use aligned buffers
   - Tests: alignment tests, O_DIRECT fallback tests

3. Phase 18.3 — On-Device Layout & Allocator (2 sprints)
   - Design simple superblock format with version & checksum
   - Implement allocation table (bitmap or B-tree) for fragment placement by offset
   - Atomic write protocol: write fragment -> fsync device -> update allocator -> fsync allocator -> persist metadata
   - Exclusive device lock (flock or ioctl) to prevent concurrent access
   - Tests: integration tests with loop devices, crash-power-loss tests during allocation and commit

4. Phase 18.4 — Integration & Safety (1 sprint)
   - Implement TRIM support and defragmentation hooks where applicable
   - Update scrubbing to be device-aware (offset reads, alignment-safe verification)
   - Add operator docs and CLI help; emphasize required root permissions and `--force` semantics
   - Add Prometheus metrics for device IO, alignment failures, and scrub progress
   - Tests: end-to-end tests, defrag/TRIM tests, automation tests using `losetup`

## On-Device Format

Raw block devices use a simple on-device layout for storing fragments directly on the device without filesystem overhead:

### Superblock (First 4KB)
- **Magic**: "DFSBLOCK" (8 bytes)
- **Version**: Current version 1 (4 bytes, little-endian)
- **Device UUID**: 16 bytes
- **Sequence Number**: Monotonically increasing counter (8 bytes)
- **Allocator Offset**: Offset to bitmap start (8 bytes)
- **Allocator Length**: Bitmap size in bytes (8 bytes)
- **Checksum**: CRC32 of above fields (4 bytes)

### Allocation Bitmap
- **Location**: Immediately after superblock, aligned to 4KB
- **Format**: Bit-packed bitmap where each bit represents one allocation unit
- **Unit Size**: 64KB by default (configurable)
- **Purpose**: Tracks which units contain valid fragment data

### Data Region
- **Location**: After bitmap, aligned to unit size
- **Format**: Fragments stored as header + data + padding
- **Fragment Header** (60 bytes):
  - Extent UUID (16 bytes)
  - Fragment Index (4 bytes, little-endian)
  - Total Length (8 bytes, little-endian)
  - Data Checksum (32 bytes, BLAKE3)
  - Header Checksum (4 bytes, CRC32)

### Atomic Write Protocol
Fragment writes follow a strict atomic protocol:
1. Allocate contiguous units in bitmap
2. Write fragment header + data to device
3. Call `fdatasync()` to ensure durability
4. Persist updated bitmap and superblock
5. Verify readback for correctness

## Recovery Procedures

### Automatic Reconciliation
The `dynamicfs recover` command performs automatic recovery:

```bash
# Check for orphaned fragments
dynamicfs recover --pool /path/to/pool

# Automatically clean up orphaned fragments
dynamicfs recover --pool /path/to/pool --cleanup
```

Recovery scans the data region for valid fragment headers and ensures the allocation bitmap correctly marks used units. This handles cases where:
- Power loss occurred after data write but before bitmap update
- Manual device modifications corrupted metadata
- Previous crashes left inconsistent state

### Manual Recovery Steps
1. **Stop all I/O**: Ensure no active operations on the pool
2. **Run reconciliation**: `dynamicfs recover --pool <pool>`
3. **Review orphans**: Check reported orphaned fragments
4. **Clean up if safe**: Use `--cleanup` flag to remove orphans
5. **Verify integrity**: Run `dynamicfs scrub` to check data integrity

### --force Semantics
The `--force` flag on `add-disk --device` allows:
- Adding devices that appear previously formatted
- Overriding safety checks for known-good devices
- Recovery from partially initialized devices

**Warning**: Only use `--force` when you understand the risks and have backups.

## Integration with Placement Flows

On-device allocators integrate with the placement system:
- **Allocation**: Requests contiguous units from bitmap
- **Placement Metadata**: Records start unit and count in extent metadata
- **Read/Write**: Direct I/O to device offsets calculated from unit positions
- **Rebuild**: Uses placement metadata to reconstruct fragments during rebuild
- **Migration**: Supports moving fragments between devices or tiers

Acceptance Criteria
- CI includes loopback-device tests that run on Linux runners
- Adding a device path requires `--device` and explicit confirmation for destructive operations
- Fragment writes on device are alignment-correct and durable (fsync/verify)
- Scrub, rebuild, and repair workflows operate correctly on device-backed disks
- Documentation updates (README + PRODUCTION_ROADMAP) and operator warnings present

Risks & Mitigations
- Risk: accidental overwrite of existing device data — mitigate by requiring `--device` + `--force`, and verifying device signatures
- Risk: alignment/O_DIRECT portability — mitigate with fallback paths and extensive tests on platforms we support
- Risk: complexity of on-device allocator — mitigate with phased rollout (stub -> read-only -> full allocator)

Owner: @xkzy (primary) — collaborators: storage, disk, scrubbing, testing teams

Notes
- CI must use `losetup` to create loopback devices from sparse files to ensure safe CI runs and reproducible test scenarios.
- Consider designing the superblock layout to be backward compatible with possible future on-disk metadata versions.

See also: `PRODUCTION_ROADMAP.md` for timeline and dependencies.
