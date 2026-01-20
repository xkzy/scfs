# Architecture Deep Dive

## System Components

### 1. Disk Abstraction Layer (`disk.rs`)

Each disk is represented by a directory on the host filesystem:

```
/disk1/
  ├── disk.json          # Metadata (UUID, capacity, health)
  └── fragments/         # Fragment storage
      ├── {extent-uuid}-0.frag
      ├── {extent-uuid}-1.frag
      └── ...
```

**Health States:**
- `Healthy`: Normal operation
- `Draining`: Being removed, no new writes
- `Failed`: Unavailable

**Key Operations:**
- `write_fragment()`: Atomic write with temp file + rename
- `read_fragment()`: Direct read from fragment file
- `update_usage()`: Scan fragments directory for disk usage
- `has_space()`: Check if disk can accept new fragment

### 2. Extent Model (`extent.rs`)

Files are split into 1MB immutable extents. Each extent has:

```rust
struct Extent {
    uuid: Uuid,              // Unique identifier
    size: usize,             // Actual data size
    checksum: [u8; 32],      // BLAKE3 hash
    redundancy: RedundancyPolicy,
    fragment_locations: Vec<FragmentLocation>,
}
```

**Redundancy Policies:**

**Replication (3 copies):** Used for small files and metadata
```
[Data] → [Fragment0] [Fragment1] [Fragment2]
           ↓           ↓           ↓
         Disk1       Disk2       Disk3
```

**Erasure Coding (4+2):** Used for large files
```
[Data] → Split into 4 data shards
      → Generate 2 parity shards
      → [D0] [D1] [D2] [D3] [P0] [P1]
         ↓    ↓    ↓    ↓    ↓    ↓
        Disk1 Disk2 Disk3 Disk4 Disk5 Disk6
```

Can reconstruct from any 4 of 6 fragments.

### 3. HMM Classifier (`hmm_classifier.rs`)

Advanced probabilistic classification for hot/cold/warm states using Hidden Markov Models:

**State Space:** 3 states (Hot, Warm, Cold) with probabilistic transitions

```
          Transition Probabilities
    ┌──────────────────────────────┐
    │  Hot: 70% stay, 20% warm,    │
    │       10% cold               │
    │  Warm: 25% hot, 50% stay,    │
    │        25% cold              │
    │  Cold: 10% hot, 20% warm,    │
    │        70% stay              │
    └──────────────────────────────┘
```

**Observations:** Frequency categories (VeryHigh >50, High 10-50, Medium 1-10, Low <1 ops/day)

**Features:**
- Prevents oscillation between states (inertia)
- Smooth transitions based on probability
- Recency boost (recent accesses favor hot)
- State history tracking (100 recent observations)
- Viterbi algorithm for optimal state sequences

**Benefits over thresholds:**
- No abrupt jumps between states
- Handles transient spikes gracefully
- Probabilistic confidence in classifications
- Historical state smoothing

### 4. Redundancy Engine (`redundancy.rs`)

**Replication:**
- Encode: Copy data N times
- Decode: Return first available copy

**Erasure Coding (Reed-Solomon):**
- Encode: Split data into k shards, generate m parity shards
- Decode: Reconstruct original from any k shards
- Benefits: Storage efficiency (1.5x vs 3x for replication)

### 5. Placement Engine (`placement.rs`)

Responsible for distributing fragments across disks:

**Selection Criteria:**
1. Never place two fragments of same extent on same disk
2. Prefer healthy disks only
3. Prefer disks with more free space
4. Load balance across available disks

**Rebuild Process:**
1. Detect missing fragments
2. Decode original data from available fragments
3. Re-encode to generate missing fragments
4. Place on different healthy disks

### 6. Metadata System (`metadata.rs`)

**Three-level metadata hierarchy:**

```
/pool/
  ├── metadata/
  │   └── next_ino       # Next inode number
  ├── inodes/
  │   ├── 1              # Root directory
  │   ├── 2              # User file/dir
  │   └── ...
  ├── extent_maps/
  │   ├── 2              # Maps ino → [extent UUIDs]
  │   └── ...
  └── extents/
      ├── {extent-uuid}  # Extent metadata
      └── ...
```

**Inode Structure:**
```rust
struct Inode {
    ino: u64,
    parent_ino: u64,
    file_type: FileType,  // RegularFile or Directory
    name: String,
    size: u64,
    atime, mtime, ctime: i64,
    uid, gid, mode: u32,
}
```

**Atomicity:** All metadata updates use write-to-temp + rename pattern for crash consistency.

### 6. Storage Engine (`storage.rs`)

Orchestrates all components:

**Write Path:**
```
write_file()
  ├─→ split_into_extents()        # 1MB chunks
  ├─→ For each extent:
  │    ├─→ redundancy::encode()   # Create fragments
  │    ├─→ placement::place()     # Distribute to disks
  │    ├─→ verify checksums
  │    └─→ save_extent()          # Commit metadata
  └─→ save_extent_map()           # Atomic commit
```

**Read Path:**
```
read_file()
  ├─→ load_extent_map()           # Get extent list
  ├─→ For each extent:
  │    ├─→ record_read()          # Update access stats
  │    ├─→ read_fragments()       # From disks
  │    ├─→ redundancy::decode()   # Reconstruct with current policy
  │    ├─→ verify_checksum()      # BLAKE3
  │    ├─→ Check if lazy migration needed
  │    │    └─→ If classification differs from policy:
  │    │        └─→ rebundle_extent() to new policy
  │    ├─→ Check availability
  │    └─→ rebuild_if_degraded()  # Lazy rebuild if needed
  └─→ Concatenate results
```

### 7. FUSE Interface (`fuse_impl.rs`)

Implements standard POSIX operations:
- `lookup()`: Find file by name
- `getattr()`: Get file attributes
- `readdir()`: List directory
- `read()`: Read file data
- `write()`: Write file data
- `create()`: Create file
- `mkdir()`: Create directory
- `unlink()`: Delete file
- `rmdir()`: Delete directory
- `setattr()`: Update attributes

## Key Design Decisions

### CoW (Copy-on-Write)

Extents are **immutable**. Updates create new extents:
- Simplifies consistency
- Enables lazy cleanup
- No in-place corruption risk

### Lazy Rebuild

Rebuilds happen **per-extent on read**, not globally:
- Faster recovery after disk failure
- No blocking rebuild jobs
- Only rebuild what's actually accessed

### Per-Object Redundancy

Each extent can have different redundancy:
- Small files: Use replication (lower latency)
- Large files: Use EC (better efficiency)
- Metadata: Always replicated for reliability

### Checksums Everywhere

BLAKE3 checksums on all data:
- Detect corruption immediately
- Enable scrubbing
- Verify after rebuild

### Lazy Migration on Read

Extents are automatically migrated to optimal redundancy policies based on access patterns:

**Classification Thresholds:**
- **Hot**: >100 ops/day OR <1 hour since last access → **Replication (3 copies)** for fast I/O
- **Warm**: >10 ops/day OR <24 hours since last access → **Replication (3 copies)**
- **Cold**: ≤10 ops/day AND ≥24 hours since last access → **Erasure Coding (4+2)** for efficiency

**Lazy Migration Process:**
1. On first read of an extent, record read access
2. Decode data with current redundancy policy
3. After successful decode, check if classification recommends a different policy
4. If migration is needed, trigger rebundle operation asynchronously
5. Next read will use new policy if migration completed

**Benefits:**
- **Automatic optimization**: No manual policy selection needed
- **Non-blocking**: Migration happens after data is successfully returned to user
- **Cost reduction**: Hot→Cold migration saves storage; Cold→Hot migration improves access latency
- **Responsive**: Only affects data that's being actively accessed

**Example:**
```
File written (256MB) → Initial EC policy (4+2)
├─ 1st read: Decode with EC, check classification (Cold)
│  └─ No migration (already optimal)
│
├─ 2nd-10th reads: EC remains optimal
│
└─ 11th+ reads: If recent reads → classification becomes Hot
   └─ Lazy migration triggered: EC (4+2) → Replication (3x)
   └─ Subsequent reads: Fast replication-based access
```

## Failure Scenarios

### Single Disk Failure (Replication)

```
Before:  [Disk1: A] [Disk2: A] [Disk3: A]
Failed:  [Disk1: X] [Disk2: A] [Disk3: A]
Result:  Still readable (2/3 copies)
Rebuild: Create new copy on Disk4
```

### Two Disk Failures (EC 4+2)

```
Before:  [D0][D1][D2][D3][P0][P1]  (6 fragments)
Failed:  [D0][X ][D2][D3][X ][P1]  (4 remaining)
Result:  Still readable (4 = minimum)
Rebuild: Reconstruct D1, P0 on new disks
```

### Three Disk Failures (EC 4+2)

```
Before:  [D0][D1][D2][D3][P0][P1]  (6 fragments)
Failed:  [D0][X ][X ][D3][X ][P1]  (3 remaining)
Result:  UNREADABLE (3 < 4 minimum)
```

## Performance Characteristics

**Write Amplification:**
- Replication (3x): 3× data written
- EC (4+2): 1.5× data written

**Read Performance:**
- Replication: 1× read from any disk
- EC (full): 4× read (data shards only)
- EC (degraded): 4× read + reconstruction overhead

**Space Efficiency:**
- Replication: 33% efficient (1/3)
- EC (4+2): 67% efficient (4/6)

## Future Optimizations

1. **Caching:** Add memory cache for hot extents
2. **Background scrubbing:** Proactive corruption detection
3. **Compression:** ZSTD on extents before encoding
4. **Small file optimization:** Pack multiple files into single extent
5. **Tiering:** Hot data on SSDs, cold on HDDs
6. **Parallel I/O:** Concurrent fragment reads
7. **Write batching:** Combine small writes

## Comparison to Traditional RAID

| Feature | Traditional RAID | DynamicFS |
|---------|-----------------|-----------|
| Geometry | Fixed (RAID 5/6) | Dynamic |
| Disk sizes | Must match | Any size |
| Add disk | Restripe entire array | Immediate |
| Remove disk | Complex rebalance | Lazy rebuild |
| Rebuild | Full disk scan | Per-extent on demand |
| Mixed redundancy | No | Yes (per file) |
| Online growth | Limited | Full support |

## Production Readiness Checklist

- [x] Core functionality
- [x] Crash consistency
- [x] Checksums
- [x] Failure recovery
- [ ] Performance tuning
- [ ] Background scrubbing
- [ ] Monitoring/metrics
- [ ] Stress testing
- [ ] Documentation
- [ ] Operational tools

This prototype demonstrates the architecture. Production would need significant additional work on performance, reliability, and operations.
