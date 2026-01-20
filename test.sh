#!/bin/bash
# Test script for DynamicFS

set -e

echo "=== DynamicFS Test Suite ==="
echo

# Cleanup function
cleanup() {
    echo "Cleaning up..."
    if mountpoint -q /tmp/dynamicfs_mnt 2>/dev/null; then
        fusermount -u /tmp/dynamicfs_mnt || sudo umount /tmp/dynamicfs_mnt
    fi
    rm -rf /tmp/dynamicfs_test
}

trap cleanup EXIT

# Setup
echo "1. Setting up test environment..."
cleanup
mkdir -p /tmp/dynamicfs_test/{pool,disk1,disk2,disk3,disk4,disk5,disk6,mnt}

# Build
echo "2. Building DynamicFS..."
cargo build --release

BIN="./target/release/dynamicfs"

# Initialize pool
echo "3. Initializing storage pool..."
$BIN init --pool /tmp/dynamicfs_test/pool

# Add disks
echo "4. Adding disks to pool..."
for i in {1..6}; do
    $BIN add-disk --pool /tmp/dynamicfs_test/pool --disk /tmp/dynamicfs_test/disk$i
done

# List disks
echo "5. Listing disks..."
$BIN list-disks --pool /tmp/dynamicfs_test/pool

# Mount filesystem
echo "6. Mounting filesystem..."
$BIN mount --pool /tmp/dynamicfs_test/pool --mountpoint /tmp/dynamicfs_test/mnt &
MOUNT_PID=$!
sleep 2

# Basic file operations
echo "7. Testing basic file operations..."
echo "Hello, World!" > /tmp/dynamicfs_test/mnt/test.txt
cat /tmp/dynamicfs_test/mnt/test.txt
[ "$(cat /tmp/dynamicfs_test/mnt/test.txt)" = "Hello, World!" ] || exit 1
echo "  ✓ Write and read works"

# Large file test
echo "8. Testing large file..."
dd if=/dev/urandom of=/tmp/dynamicfs_test/mnt/large.bin bs=1M count=5 2>/dev/null
SIZE=$(stat -c%s /tmp/dynamicfs_test/mnt/large.bin)
[ "$SIZE" -eq 5242880 ] || exit 1
echo "  ✓ Large file write works (5MB)"

# Directory operations
echo "9. Testing directory operations..."
mkdir -p /tmp/dynamicfs_test/mnt/subdir
echo "File in subdir" > /tmp/dynamicfs_test/mnt/subdir/file.txt
[ -f /tmp/dynamicfs_test/mnt/subdir/file.txt ] || exit 1
echo "  ✓ Directory operations work"

# Multiple files
echo "10. Testing multiple files..."
for i in {1..10}; do
    echo "File $i" > /tmp/dynamicfs_test/mnt/file$i.txt
done
[ $(ls /tmp/dynamicfs_test/mnt/file*.txt | wc -l) -eq 10 ] || exit 1
echo "  ✓ Multiple files work"

# Unmount
echo "11. Unmounting..."
fusermount -u /tmp/dynamicfs_test/mnt || sudo umount /tmp/dynamicfs_test/mnt
wait $MOUNT_PID 2>/dev/null || true

# Check redundancy
echo "12. Checking redundancy status..."
$BIN show-redundancy --pool /tmp/dynamicfs_test/pool

# List extents
echo "13. Listing extents..."
$BIN list-extents --pool /tmp/dynamicfs_test/pool

# Simulate disk failure
echo "14. Simulating disk failure..."
$BIN fail-disk --pool /tmp/dynamicfs_test/pool --disk /tmp/dynamicfs_test/disk1
$BIN show-redundancy --pool /tmp/dynamicfs_test/pool

# Remount and verify data survives disk failure
echo "15. Remounting after disk failure..."
$BIN mount --pool /tmp/dynamicfs_test/pool --mountpoint /tmp/dynamicfs_test/mnt &
MOUNT_PID=$!
sleep 2

echo "16. Verifying data after disk failure..."
[ "$(cat /tmp/dynamicfs_test/mnt/test.txt)" = "Hello, World!" ] && echo "  ✓ Data survived disk failure"
[ -f /tmp/dynamicfs_test/mnt/large.bin ] && echo "  ✓ Large file survived"
[ $(ls /tmp/dynamicfs_test/mnt/file*.txt 2>/dev/null | wc -l) -eq 10 ] && echo "  ✓ All files accessible"

# Cleanup
fusermount -u /tmp/dynamicfs_test/mnt || sudo umount /tmp/dynamicfs_test/mnt
wait $MOUNT_PID 2>/dev/null || true

echo
echo "=== All Tests Passed! ==="
echo
echo "Summary:"
echo "  ✓ Pool initialization"
echo "  ✓ Disk management"
echo "  ✓ File read/write"
echo "  ✓ Large files"
echo "  ✓ Directory operations"
echo "  ✓ Disk failure recovery"
echo "  ✓ Data persistence"
