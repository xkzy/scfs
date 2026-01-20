# Phase 18.3 — On-Device Layout & Allocator (Design)

Status: In-Progress (design)
Owner: @xkzy

Goal
- Define an on-device layout and allocation scheme for storing fragments directly on raw block devices safely and recoverably.
- Produce a clear atomic commit protocol and test matrix for crash-safety and alignment correctness.

Constraints & Requirements
- Safety-first: default to read-only until device allocator proves safe.
- Portable across Linux-ish block devices (support loopback devices for CI)
- Alignment-aware: honor device block size and use O_DIRECT where possible
- Durability: ensure writes are durable before metadata updates
- Atomicity: avoid leaving inconsistent allocator state after crashes
- Exclusive access: require exclusive lock to avoid concurrent writers
- Backwards compatibility: allow non-device disks and maintain pool metadata for both

High-level Layout
- Fixed superblock region at offset 0 (and backups at fixed offsets near start) — 4KB area (or device block size)
- Superblock contents (all fields little-endian):
  - magic: [u8; 8] = b"DFSBLOCK"
  - version: u32
  - device_uuid: uuid (16 bytes)
  - superblock_seq: u64 (monotonic sequence number for updates)
  - allocator_offset: u64 (byte offset of allocator region)
  - allocator_len: u64 (bytes)
  - checksum: u64 (e.g., CRC64 or BLAKE3 truncated; consider CRC64 for speed)
  - reserved/padding to fill superblock area
- Keep N copies of superblock (primary + 2 backups) at well-known offsets (e.g., 0, 64KB, 128KB) to survive write failures

Allocator strategies (tradeoffs)
- Bitmap allocator (preferred for simplicity)
  - Allocation granularity = ALLOC_UNIT (e.g., 4KB or 1MB depending on fragment sizes)
  - Bitmap stored as compact bitarray in allocator region; each bit = one unit
  - Fast, small metadata; simple atomic updates via write+fsync
  - Use case: fast free/used checks per disk; very low CPU overhead
- Free-extent B-tree (complementary)
  - Store free-extent runs keyed by (start_unit -> length) or keyed by length to find best-fit
  - Efficient for finding contiguous runs for large fragments and reducing fragmentation
  - Use a small on-device B-tree index persisted alongside the bitmap to accelerate large allocations
- Extent allocator (generic B-tree / freelist)
  - More complex; supports variable-sized allocations; better fragmentation behavior
  - Consider as the canonical allocator once stable (offers powerful allocation queries)

Allocation
- Primary approach: Bitmap per disk for very fast free/used checks
- Complementary index: Free-extent B-tree to locate contiguous space quickly (best-fit or first-fit strategies)
- Behavior: first attempt to find contiguous run via free-extent B-tree; if not found, fall back to bitmap scanning across units


Allocator metadata
- Header: allocator version, unit_size, total_units, free_count, checksum
- Bitmap data

Fragment placement
- An allocated fragment maps to offset = allocator_base + (unit_index * unit_size)
- Fragment size must be <= allocation unit count * unit_size (enforce fixed fragment size for simplicity: e.g., 1 MiB)

Write & Commit Protocol (SAFETY-CRITICAL)
All writes must follow this sequence for each fragment write:
1. Allocate units in bitmap (atomic update of bitmap in memory)
2. Write fragment data to the allocated offset(s) using device FD
   - If O_DIRECT is used, ensure buffers are aligned and size is a multiple of block_size
   - Use pwrite-like semantics to the exact offset
3. fdatasync(device_fd) — ensure data is durably on device
4. Persist allocator metadata update on device (write bitmap region) and fsync device
5. Update superblock_seq and write superblock copy (primary) with new checksum and fsync
6. Optionally write secondary superblock copy and fsync (for extra safety)

Crash recovery procedure
- On mount, read superblocks (primary + backups), select the highest valid superblock_seq with correct checksum
- Load allocator region from pointed offset/len
- Validate allocator consistency (bitmap counts match free_count). If mismatch, run scan heuristic:
  - Scan allocation region to find fragments whose fragment headers (each fragment starts with small header containing extent UUID and checksum) match and reconcile bitmap
  - Mark any fragments present but not marked allocated as orphans (or add allocated entries) depending on policy
- If no valid superblock found, mount read-only and perform a careful scan to reconstruct metadata (operator-assisted recovery)

Fragment header & verification
- Each fragment stored on-device starts with a small header (aligned) containing:
  - extent_uuid: 16 bytes
  - fragment_index: u32
  - total_length: u64
  - data_checksum: 32 bytes (BLAKE3)
  - header_checksum: u32
- Reader verifies header checksum and data checksum; repair/rebuild uses redundancy

Metadata & Indexing
- Use on-device B-trees for persistent metadata where efficient lookups or range queries are needed:
  - Inode table: B-tree keyed by inode id -> inode metadata (on-disk or pool-level depending on scope)
  - Extent → Fragment mapping: B-tree keyed by extent UUID -> list of fragment placements (disk UUID + unit index + length)
  - Policy metadata: B-tree keyed by extent UUID / timestamps to record policy history and transitions
- Implementation strategy:
  - Start with an in-memory BTreeMap with on-device persistence (write-then-rename or append-log) for correctness and tests
  - Replace with an on-device B-tree (simple persistent B-tree or small embedded crate) when performance or concurrency requirements demand it
- Rationale: B-trees give efficient point/range lookups and are suitable for the types of queries (find fragments for an extent, list extents in a range, query policy by time)

Locking & Exclusive Access
- Use flock on device or an external lockfile (e.g., /var/lock/dynamicfs-<device-uuid>.lock) to prevent concurrent access
- On add-disk, refuse to add device if exclusive lock cannot be acquired (unless `--force` with explicit operator consent)

Testing Plan
- Unit tests:
  - Bitmap allocation correctness (alloc, free, wrap-around, fragmentation)
  - Superblock write/read/validation
  - Header check & data checksum verification
- Integration tests (CI using loopback devices):
  - Add device with `--device` and detect block device path via losetup
  - Allocate fragment → write data → crash between steps (simulate by killing process or using crash_sim) and validate recovery
  - Power-loss scenarios: crash during data write, during allocator commit, during superblock update
  - Alignment tests: O_DIRECT writes require aligned buffers and sizes; verify fallback to O_SYNC works and data persists
- Fuzz tests: random allocation/free/write sequences, followed by simulated crashes and recovery

Operational notes
- Adding a device requires operator privileges; document `--device` semantics and `--force` in README and CLI help
- Keep default behavior conservative (do not enable writes to devices until Phase 18.3 acceptance criteria are met)

Acceptance Criteria for Phase 18.3
- Bitmap allocator implemented and tested in unit & CI with loopback devices
- Fragment writes follow the data->fsync->bitmap->fsync->superblock sequence and survive simulated crashes
- Mount-time recovery can reconcile minor inconsistencies and detect orphan fragments for manual cleaning
- Documentation updated with format and recovery procedures

Open design decisions
- Checksum algorithm for superblock/allocator (CRC64 vs BLAKE3) — CRC64 faster but BLAKE3 stronger; pick CRC for metadata and BLAKE3 for data
- Allocation unit size: choose default 1 MiB if fragment size == 1 MiB, otherwise use device blocksize multiple
- Number/location of superblock backups

Next implementation steps
1. Implement on-disk superblock struct and read/write helpers (with checksum validation)
2. Implement bitmap allocator on device with in-memory caching and on-device persistence
3. Integrate exclusive lock acquisition on `add-disk`
4. Add fragment header structures and write/read helpers that obey alignment constraints
5. Add comprehensive tests (unit + integration with loopback devices)

References & Notes
- Use `losetup --find --show` on CI runners for integration tests (create sparse files, losetup them, run tests)
- Consider using `libc::sync_file_range` or `posix_fadvise` if needed for advanced IO control

