# DynamicFS - Project Structure

## Quick Navigation

- **New to the project?** Start with [README.md](README.md)
- **Want to try it?** Follow [QUICKSTART.md](QUICKSTART.md)
- **Understanding design?** Read [ARCHITECTURE.md](ARCHITECTURE.md)
- **Project summary?** See [SUMMARY.md](SUMMARY.md)

## File Organization

```
New FS/
├── README.md                 # Main documentation
├── QUICKSTART.md             # Step-by-step tutorial
├── ARCHITECTURE.md           # Deep technical dive
├── SUMMARY.md               # Implementation summary
├── Cargo.toml               # Rust dependencies
├── test.sh                  # Integration test script
│
└── src/
    ├── main.rs              # Entry point and CLI handlers
    ├── cli.rs               # Command-line argument parsing
    ├── disk.rs              # Disk abstraction (391 lines)
    ├── extent.rs            # Extent/fragment model (103 lines)
    ├── redundancy.rs        # Replication + EC engine (178 lines)
    ├── placement.rs         # Fragment placement logic (204 lines)
    ├── metadata.rs          # Metadata management (234 lines)
    ├── storage.rs           # Storage engine + tests (420 lines)
    ├── fuse_impl.rs         # FUSE filesystem (358 lines)
    └── storage_tests.rs     # Additional tests (deprecated)
```

## Module Dependencies

```
main.rs
  ├── cli.rs
  ├── disk.rs
  ├── fuse_impl.rs
  │     └── storage.rs
  │           ├── metadata.rs
  │           │     └── extent.rs
  │           ├── disk.rs
  │           ├── placement.rs
  │           │     ├── disk.rs
  │           │     └── extent.rs
  │           └── redundancy.rs
  │                 └── extent.rs
  └── metadata.rs
```

## Key Components

### 1. Disk Layer
**File:** `src/disk.rs`
**Purpose:** Virtual disk abstraction backed by directories
**Key types:** `Disk`, `DiskPool`, `DiskHealth`

### 2. Extent Layer
**File:** `src/extent.rs`
**Purpose:** Immutable data chunks with checksums
**Key types:** `Extent`, `FragmentLocation`, `RedundancyPolicy`

### 3. Redundancy Layer
**File:** `src/redundancy.rs`
**Purpose:** Encode/decode with replication or erasure coding
**Key functions:** `encode()`, `decode()`

### 4. Placement Layer
**File:** `src/placement.rs`
**Purpose:** Decide where to place fragments
**Key type:** `PlacementEngine`

### 5. Metadata Layer
**File:** `src/metadata.rs`
**Purpose:** Manage filesystem metadata (inodes, extent maps)
**Key type:** `MetadataManager`

### 6. Storage Layer
**File:** `src/storage.rs`
**Purpose:** Orchestrate all layers for read/write operations
**Key type:** `StorageEngine`

### 7. FUSE Layer
**File:** `src/fuse_impl.rs`
**Purpose:** Expose POSIX filesystem interface
**Key type:** `DynamicFS`

### 8. CLI Layer
**Files:** `src/cli.rs`, `src/main.rs`
**Purpose:** Command-line interface and entry point
**Commands:** init, add-disk, mount, list-*, etc.

## Data Flow

### Write Path
```
User writes file
  ↓
FUSE (fuse_impl.rs)
  ↓
StorageEngine::write_file() (storage.rs)
  ↓
Split into Extents (extent.rs)
  ↓
Encode with Redundancy (redundancy.rs)
  ↓
PlacementEngine selects disks (placement.rs)
  ↓
Write fragments to Disks (disk.rs)
  ↓
Save Metadata (metadata.rs)
  ↓
Success!
```

### Read Path
```
User reads file
  ↓
FUSE (fuse_impl.rs)
  ↓
StorageEngine::read_file() (storage.rs)
  ↓
Load extent map (metadata.rs)
  ↓
For each extent:
    Read fragments (disk.rs)
    Check if rebuild needed
    Decode data (redundancy.rs)
    Verify checksum (extent.rs)
  ↓
Concatenate and return
```

## Testing

### Unit Tests
Located in each module:
- `redundancy.rs`: Replication and EC tests
- `placement.rs`: Disk selection tests
- `storage.rs`: End-to-end storage tests

Run with: `cargo test`

### Integration Test
**File:** `test.sh`
Tests complete workflow including disk failures

Run with: `./test.sh`

## Building

### Debug Build
```bash
cargo build
./target/debug/dynamicfs --help
```

### Release Build
```bash
cargo build --release
./target/release/dynamicfs --help
```

### Run Tests
```bash
cargo test                    # Unit tests
cargo test --release          # Optimized tests
./test.sh                     # Integration test
```

## Code Statistics

| File | Lines | Purpose |
|------|-------|---------|
| disk.rs | 391 | Disk management |
| storage.rs | 420 | Storage engine + tests |
| fuse_impl.rs | 358 | FUSE interface |
| metadata.rs | 234 | Metadata system |
| main.rs | 228 | CLI handlers |
| placement.rs | 204 | Placement logic |
| redundancy.rs | 178 | Redundancy engine |
| extent.rs | 103 | Extent model |
| cli.rs | 78 | CLI parsing |
| **Total** | **~2,200** | **Source lines** |

## External Dependencies

- **fuser** - FUSE bindings for Rust
- **reed-solomon-erasure** - Erasure coding library
- **blake3** - Fast cryptographic hash
- **uuid** - UUID generation
- **serde/serde_json** - Serialization
- **clap** - CLI parsing
- **anyhow** - Error handling
- **chrono** - Time handling
- **nix** - Unix system calls

## Development Workflow

1. **Make changes** to source files
2. **Check compilation**: `cargo check`
3. **Run tests**: `cargo test`
4. **Build release**: `cargo build --release`
5. **Test manually**: Use CLI commands
6. **Integration test**: `./test.sh`

## Extending the System

### Add New Redundancy Scheme

1. Add variant to `RedundancyPolicy` in `extent.rs`
2. Implement encode/decode in `redundancy.rs`
3. Update `StorageEngine` to use new scheme
4. Add tests

### Add New CLI Command

1. Add variant to `Commands` in `cli.rs`
2. Implement handler in `main.rs`
3. Update README with usage

### Improve Performance

Focus areas:
- Add caching in `storage.rs`
- Parallel I/O in `read_fragments()`
- Async operations throughout
- Write batching in `disk.rs`

## Troubleshooting Development

### Compilation Errors
```bash
cargo clean
cargo build
```

### Test Failures
```bash
cargo test -- --nocapture  # See output
RUST_LOG=debug cargo test  # With logging
```

### FUSE Issues
```bash
fusermount -u /tmp/mnt     # Unmount
dmesg | tail               # Check kernel logs
```

## Documentation

- **Code comments**: Inline documentation
- **README.md**: User guide
- **ARCHITECTURE.md**: Design documentation
- **QUICKSTART.md**: Tutorial
- **SUMMARY.md**: Project overview

## License

This is a prototype implementation for educational purposes.

## Contributing

This is a prototype/reference implementation. For improvements:

1. Understand the architecture (ARCHITECTURE.md)
2. Make focused changes
3. Add/update tests
4. Update documentation
5. Ensure `cargo test` passes

## Getting Help

- Read the documentation (start with README.md)
- Check ARCHITECTURE.md for design details
- Review code comments
- Examine test cases for examples

---

**Happy coding!** 🚀
