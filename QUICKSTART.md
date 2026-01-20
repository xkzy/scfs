# Quick Start Guide

## Prerequisites

- Linux system with FUSE support
- Rust toolchain (1.70+)
- libfuse3-dev installed

```bash
sudo apt-get install libfuse3-dev  # Debian/Ubuntu
sudo dnf install fuse3-devel       # Fedora
```

## Installation

```bash
cd "New FS"
cargo build --release
```

The binary will be at `target/release/dynamicfs`.

## Example Walkthrough

### 1. Initialize Storage

Create directories for your disks and pool:

```bash
mkdir -p ~/dynamicfs/{pool,disk1,disk2,disk3,disk4,disk5,disk6}
```

Initialize the pool:

```bash
./target/release/dynamicfs init --pool ~/dynamicfs/pool
```

### 2. Add Disks

Add all 6 disks to the pool:

```bash
for i in {1..6}; do
    ./target/release/dynamicfs add-disk \
        --pool ~/dynamicfs/pool \
        --disk ~/dynamicfs/disk$i
done
```

Verify disks were added:

```bash
./target/release/dynamicfs list-disks --pool ~/dynamicfs/pool
```

### 3. Mount the Filesystem

Create a mount point and mount:

```bash
mkdir -p ~/dynamicfs/mnt
./target/release/dynamicfs mount \
    --pool ~/dynamicfs/pool \
    --mountpoint ~/dynamicfs/mnt
```

The filesystem is now available at `~/dynamicfs/mnt`. In another terminal:

```bash
cd ~/dynamicfs/mnt
echo "Hello, DynamicFS!" > test.txt
cat test.txt
```

### 4. Test Redundancy

Create a large file:

```bash
dd if=/dev/urandom of=~/dynamicfs/mnt/large.bin bs=1M count=10
```

Check redundancy status:

```bash
# In another terminal
./target/release/dynamicfs show-redundancy --pool ~/dynamicfs/pool
./target/release/dynamicfs list-extents --pool ~/dynamicfs/pool
```

### 5. Simulate Disk Failure

Fail a disk:

```bash
./target/release/dynamicfs fail-disk \
    --pool ~/dynamicfs/pool \
    --disk ~/dynamicfs/disk1
```

The file system continues to work! Check your file:

```bash
cat ~/dynamicfs/mnt/test.txt
# Still works!
```

Check redundancy again:

```bash
./target/release/dynamicfs show-redundancy --pool ~/dynamicfs/pool
# Shows degraded state
```

### 6. Graceful Disk Removal

To remove a disk gracefully:

```bash
# First, unmount
fusermount -u ~/dynamicfs/mnt

# Remove disk
./target/release/dynamicfs remove-disk \
    --pool ~/dynamicfs/pool \
    --disk ~/dynamicfs/disk2

# Remount
./target/release/dynamicfs mount \
    --pool ~/dynamicfs/pool \
    --mountpoint ~/dynamicfs/mnt
```

### 7. Cleanup

```bash
# Unmount
fusermount -u ~/dynamicfs/mnt

# Clean up
rm -rf ~/dynamicfs
```

## Testing Erasure Coding

Small files (< 1MB) use 3-way replication:

```bash
echo "Small file" > ~/dynamicfs/mnt/small.txt
```

Large files (>= 1MB) use EC (4+2):

```bash
dd if=/dev/urandom of=~/dynamicfs/mnt/big.bin bs=1M count=5
```

With EC (4+2), the system can survive **2 simultaneous disk failures**!

## Automated Testing

Run the comprehensive test suite:

```bash
./test.sh
```

This will:
- Initialize a test pool
- Add 6 disks
- Mount the filesystem
- Test file operations
- Simulate disk failures
- Verify data survives

## Performance Notes

This is a **prototype** focused on correctness over performance. Expected characteristics:

**Write Performance:**
- Small files: ~3x slower than single disk (replication overhead)
- Large files: ~1.5x slower than single disk (EC overhead)

**Read Performance:**
- Normal reads: Similar to single disk
- Degraded reads: Slower due to reconstruction

**Space Efficiency:**
- Small files: 33% (3-way replication)
- Large files: 67% (4+2 EC)

## Troubleshooting

### Mount fails with permission error

Add your user to the `fuse` group:

```bash
sudo usermod -a -G fuse $USER
newgrp fuse
```

### Cannot unmount

```bash
# Force unmount
fusermount -u ~/dynamicfs/mnt
# or
sudo umount ~/dynamicfs/mnt
```

### Disk shows as failed unexpectedly

Check disk metadata:

```bash
cat ~/dynamicfs/disk1/disk.json
```

Reset disk health manually if needed (edit JSON file).

### Data corruption detected

The system will log checksum failures. To investigate:

```bash
# Check extent status
./target/release/dynamicfs list-extents --pool ~/dynamicfs/pool

# Check redundancy
./target/release/dynamicfs show-redundancy --pool ~/dynamicfs/pool
```

## Advanced Usage

### Adding Disks Online

You can add disks while the filesystem is mounted:

```bash
# Create new disk
mkdir ~/dynamicfs/disk7

# Add it (in another terminal)
./target/release/dynamicfs add-disk \
    --pool ~/dynamicfs/pool \
    --disk ~/dynamicfs/disk7

# It's immediately available for new writes!
```

### Monitoring Disk Usage

```bash
# List disks with usage stats
./target/release/dynamicfs list-disks --pool ~/dynamicfs/pool
```

### Checking Fragment Distribution

```bash
# See how fragments are distributed
./target/release/dynamicfs list-extents --pool ~/dynamicfs/pool

# Each extent shows which disks have its fragments
```

## Limitations

Current prototype limitations:

1. **Write offset**: Only supports full file rewrites (offset 0)
2. **No partial updates**: Entire file rewritten on change
3. **Single node**: No network distribution
4. **Synchronous I/O**: No async operations
5. **No caching**: All reads go to disk
6. **No background scrub**: No proactive corruption detection

These are intentional for the prototype. A production system would address all of these.

## Next Steps

To extend this prototype:

1. **Add caching**: Use an LRU cache for hot extents
2. **Implement partial writes**: Support arbitrary offsets
3. **Add compression**: Compress extents before encoding
4. **Background scrubbing**: Periodically verify checksums
5. **Performance tuning**: Parallel I/O, batching
6. **Monitoring**: Add metrics and dashboards
7. **Network support**: Distribute across nodes

See [ARCHITECTURE.md](ARCHITECTURE.md) for implementation details.
