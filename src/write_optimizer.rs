use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Condvar};
use std::time::{Duration, Instant};
use uuid::Uuid;
use crate::extent::Extent;
use crate::metadata::MetadataManager;

/// Write batch containing multiple extents ready for concurrent placement
#[derive(Debug, Clone)]
pub struct WriteBatch {
    pub id: Uuid,
    pub extents: Vec<Extent>,
    pub total_bytes: u64,
}

/// Batches writes for concurrent disk placement with load balancing
pub struct WriteBatcher {
    /// Maximum extents per batch
    pub max_batch_size: usize,
    /// Maximum bytes per batch
    pub max_batch_bytes: u64,
    /// Current pending extents
    pending: Arc<Mutex<VecDeque<Extent>>>,
}

impl WriteBatcher {
    pub fn new(max_batch_size: usize, max_batch_bytes: u64) -> Self {
        WriteBatcher {
            max_batch_size,
            max_batch_bytes,
            pending: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Add extent to pending queue, returns batch if ready
    pub fn add_extent(&self, extent: Extent) -> Option<WriteBatch> {
        let mut pending = self.pending.lock().unwrap();
        pending.push_back(extent);

        // Check if we should create a batch
        if pending.len() >= self.max_batch_size {
            return Some(self.drain_batch(&mut pending));
        }

        let total_bytes: u64 = pending.iter().map(|e| e.size as u64).sum();
        if total_bytes >= self.max_batch_bytes && !pending.is_empty() {
            return Some(self.drain_batch(&mut pending));
        }

        None
    }

    /// Force flush pending extents as a batch
    pub fn flush(&self) -> Option<WriteBatch> {
        let mut pending = self.pending.lock().unwrap();
        if pending.is_empty() {
            return None;
        }
        Some(self.drain_batch(&mut pending))
    }

    /// Get pending extent count
    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }

    /// Drain batch from pending queue
    fn drain_batch(&self, pending: &mut VecDeque<Extent>) -> WriteBatch {
        let mut extents = Vec::new();
        let mut total_bytes = 0u64;

        // Take up to max_batch_size extents or until bytes limit
        while let Some(extent) = pending.pop_front() {
            total_bytes += extent.size as u64;
            extents.push(extent);

            if extents.len() >= self.max_batch_size || total_bytes >= self.max_batch_bytes {
                break;
            }
        }

        WriteBatch {
            id: Uuid::new_v4(),
            extents,
            total_bytes,
        }
    }
}

/// Metadata cache for frequently accessed extents
pub struct MetadataCache {
    /// Maximum cached extents
    pub capacity: usize,
    /// LRU cache: (extent_uuid, extent)
    cache: Arc<Mutex<VecDeque<(Uuid, Extent)>>>,
}

impl MetadataCache {
    pub fn new(capacity: usize) -> Self {
        MetadataCache {
            capacity,
            cache: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Get extent from cache if present
    pub fn get(&self, uuid: &Uuid) -> Option<Extent> {
        let mut cache = self.cache.lock().unwrap();
        
        // Find and move to front (LRU)
        if let Some(pos) = cache.iter().position(|(id, _)| id == uuid) {
            let (id, extent) = cache.remove(pos).unwrap();
            cache.push_front((id, extent.clone()));
            return Some(extent);
        }
        None
    }

    /// Store extent in cache (evict LRU if at capacity)
    pub fn put(&self, uuid: Uuid, extent: Extent) {
        let mut cache = self.cache.lock().unwrap();
        
        // Remove if already present
        if let Some(pos) = cache.iter().position(|(id, _)| id == &uuid) {
            cache.remove(pos);
        }

        // Add to front
        cache.push_front((uuid, extent));

        // Evict LRU if over capacity
        if cache.len() > self.capacity {
            cache.pop_back();
        }
    }

    /// Clear cache
    pub fn clear(&self) {
        self.cache.lock().unwrap().clear();
    }

    /// Get cache hit count (for testing)
    pub fn len(&self) -> usize {
        self.cache.lock().unwrap().len()
    }
}

/// Coalesces small writes into larger extents for better efficiency
pub struct WriteCoalescer {
    /// Minimum size to trigger coalescing
    pub min_coalesce_size: usize,
    /// Maximum coalesced extent size
    pub max_coalesced_size: usize,
    /// Pending small writes
    pending: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
}

impl WriteCoalescer {
    pub fn new(min_coalesce_size: usize, max_coalesced_size: usize) -> Self {
        WriteCoalescer {
            min_coalesce_size,
            max_coalesced_size,
            pending: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Try to coalesce a write with pending writes
    /// Returns Some(coalesced_data) if coalescing occurred, None otherwise
    pub fn try_coalesce(&self, data: &[u8]) -> Option<Vec<u8>> {
        if data.len() >= self.min_coalesce_size {
            return None; // Already large enough
        }

        let mut pending = self.pending.lock().unwrap();
        
        // Check if adding this write would exceed capacity
        let total_size: usize = pending.iter().map(|(_, d)| d.len()).sum::<usize>() + data.len();
        if total_size > self.max_coalesced_size {
            return None;
        }

        // Try to coalesce with next write if it's also small
        if pending.is_empty() {
            pending.push((0, data.to_vec()));
            return None;
        }

        // If pending writes exist and total is now large enough, return coalesced
        if total_size >= self.min_coalesce_size {
            let mut coalesced = Vec::with_capacity(total_size);
            for (_, d) in pending.drain(..) {
                coalesced.extend_from_slice(&d);
            }
            coalesced.extend_from_slice(data);
            return Some(coalesced);
        }

        None
    }

    /// Flush all pending writes
    pub fn flush(&self) -> Option<Vec<u8>> {
        let mut pending = self.pending.lock().unwrap();
        if pending.is_empty() {
            return None;
        }

        let mut coalesced = Vec::new();
        for (_, data) in pending.drain(..) {
            coalesced.extend_from_slice(&data);
        }
        Some(coalesced)
    }
}

/// Phase 15: Group commit coordinator for metadata updates
/// 
/// Batches multiple metadata updates into a single fsync to amortize
/// the cost of durable commits. This dramatically improves write throughput
/// by reducing fsync frequency from per-update to per-batch.
pub struct GroupCommitCoordinator {
    /// Maximum updates per commit batch
    max_batch_size: usize,
    /// Maximum time to wait before forcing a commit (milliseconds)
    max_batch_time_ms: u64,
    /// Pending metadata updates
    pending: Arc<Mutex<PendingCommits>>,
    /// Condition variable for signaling commit completion
    commit_signal: Arc<Condvar>,
}

/// Pending commits state
struct PendingCommits {
    /// Operations waiting to be committed
    operations: Vec<MetadataOperation>,
    /// Time when the first operation was added
    batch_start: Option<Instant>,
    /// Number of completed commits
    commits_completed: u64,
}

/// A metadata operation to be committed
#[derive(Debug, Clone)]
pub enum MetadataOperation {
    SaveExtent(Extent),
    UpdateInode(u64, u64), // (ino, new_size)
    SaveExtentMap(u64, Vec<Uuid>), // (ino, extent_uuids)
}

impl GroupCommitCoordinator {
    /// Create new group commit coordinator
    pub fn new(max_batch_size: usize, max_batch_time_ms: u64) -> Self {
        GroupCommitCoordinator {
            max_batch_size,
            max_batch_time_ms,
            pending: Arc::new(Mutex::new(PendingCommits {
                operations: Vec::new(),
                batch_start: None,
                commits_completed: 0,
            })),
            commit_signal: Arc::new(Condvar::new()),
        }
    }
    
    /// Add a metadata operation to the batch
    /// 
    /// Returns true if a commit should be triggered immediately
    pub fn add_operation(&self, op: MetadataOperation) -> bool {
        let mut pending = self.pending.lock().unwrap();
        
        if pending.operations.is_empty() {
            pending.batch_start = Some(Instant::now());
        }
        
        pending.operations.push(op);
        
        // Check if we should commit now
        if pending.operations.len() >= self.max_batch_size {
            return true;
        }
        
        // Check if batch has been waiting too long
        if let Some(start) = pending.batch_start {
            let elapsed = start.elapsed();
            if elapsed >= Duration::from_millis(self.max_batch_time_ms) {
                return true;
            }
        }
        
        false
    }
    
    /// Commit all pending operations (blocking)
    /// 
    /// Returns the number of operations committed
    pub fn commit(&self, metadata: &MetadataManager) -> anyhow::Result<usize> {
        let operations = {
            let mut pending = self.pending.lock().unwrap();
            if pending.operations.is_empty() {
                return Ok(0);
            }
            
            let ops = std::mem::take(&mut pending.operations);
            pending.batch_start = None;
            ops
        };
        
        let count = operations.len();
        
        // Use a consistent timestamp for all operations in this batch
        let batch_timestamp = chrono::Utc::now().timestamp();
        
        // Execute all operations
        for op in operations {
            match op {
                MetadataOperation::SaveExtent(extent) => {
                    metadata.save_extent(&extent)?;
                }
                MetadataOperation::UpdateInode(ino, size) => {
                    let mut inode = metadata.load_inode(ino)?;
                    inode.size = size;
                    inode.mtime = batch_timestamp; // Use consistent timestamp
                    metadata.save_inode(&inode)?;
                }
                MetadataOperation::SaveExtentMap(ino, extent_uuids) => {
                    let extent_map = crate::metadata::ExtentMap {
                        ino,
                        extents: extent_uuids,
                        checksum: None,
                    };
                    metadata.save_extent_map(&extent_map)?;
                }
            }
        }
        
        // Signal completion
        {
            let mut pending = self.pending.lock().unwrap();
            pending.commits_completed += 1;
        }
        self.commit_signal.notify_all();
        
        Ok(count)
    }
    
    /// Get number of pending operations
    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap().operations.len()
    }
    
    /// Get total commits completed
    pub fn commits_completed(&self) -> u64 {
        self.pending.lock().unwrap().commits_completed
    }
    
    /// Force flush all pending operations
    pub fn flush(&self, metadata: &MetadataManager) -> anyhow::Result<usize> {
        self.commit(metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_batcher_simple() {
        let batcher = WriteBatcher::new(2, 1024);
        let extent1 = create_test_extent(100);
        
        // First extent doesn't trigger batch
        assert!(batcher.add_extent(extent1.clone()).is_none());
        
        // Second extent triggers batch
        let extent2 = create_test_extent(100);
        let batch = batcher.add_extent(extent2).unwrap();
        assert_eq!(batch.extents.len(), 2);
        assert_eq!(batch.total_bytes, 200);
    }

    #[test]
    fn test_metadata_cache_lru() {
        let cache = MetadataCache::new(2);
        let uuid1 = Uuid::new_v4();
        let extent1 = create_test_extent(100);

        // Add first extent
        cache.put(uuid1, extent1.clone());
        assert_eq!(cache.len(), 1);
        
        // Get it back
        assert!(cache.get(&uuid1).is_some());
        
        // Add second extent
        let uuid2 = Uuid::new_v4();
        let extent2 = create_test_extent(100);
        cache.put(uuid2, extent2);
        assert_eq!(cache.len(), 2);
        
        // Add third (should evict first)
        let uuid3 = Uuid::new_v4();
        let extent3 = create_test_extent(100);
        cache.put(uuid3, extent3);
        assert_eq!(cache.len(), 2);
        
        // uuid1 should be gone
        assert!(cache.get(&uuid1).is_none());
    }

    fn create_test_extent(size: usize) -> Extent {
        use crate::extent::{AccessStats, RedundancyPolicy, AccessClassification};
        use chrono::Utc;
        Extent {
            uuid: Uuid::new_v4(),
            size,
            checksum: [0; 32],
            redundancy: RedundancyPolicy::Replication { copies: 3 },
            fragment_locations: Vec::new(),
            previous_policy: None,
            policy_transitions: Vec::new(),
            last_policy_change: None,
            access_stats: AccessStats {
                read_count: 0,
                write_count: 0,
                last_read: 0,
                last_write: 0,
                created_at: Utc::now().timestamp(),
                classification: AccessClassification::Cold,
                hmm_classifier: None,
            },
            rebuild_in_progress: false,
            rebuild_progress: None,
            generation: 0,
        }
    }

    #[test]
    fn test_write_coalescer() {
        let coalescer = WriteCoalescer::new(100, 500);
        
        // Small write doesn't coalesce
        let result1 = coalescer.try_coalesce(&[0; 50]);
        assert!(result1.is_none());
        
        // Another small write triggers coalescing
        let result2 = coalescer.try_coalesce(&[0; 60]);
        assert!(result2.is_some());
        let coalesced = result2.unwrap();
        assert_eq!(coalesced.len(), 110);
    }
}
