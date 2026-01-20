# Phase 18 — Raw Block Device Support

Status: Planned
Priority: High (Safety-first)
Estimated Effort: 3-4 sprints (6-8 weeks)

Scope
- Add safe, explicit support for raw block devices (e.g. /dev/loopX, /dev/sdX) as disks in the pool.
- Support device-aware I/O semantics: O_DIRECT (or O_SYNC fallback), TRIM support, alignment-aware reads/writes.
- Implement an on-device layout (superblock + allocator) for storing fragments by offset.
- Provide explicit CLI safety controls (`--device`, `--force`) and safety checks (device signature detection, exclusive lock).
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
