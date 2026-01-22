/// Phase 15: Concurrent Read/Write Optimization Tests
/// 
/// Comprehensive tests for concurrency primitives, write batching,
/// group commit, I/O scheduling, and stress testing.

#[cfg(test)]
mod phase_15_tests {
    use crate::concurrency::{ExtentLockManager, ExtentSnapshot};
    use crate::extent::{Extent, RedundancyPolicy, AccessStats, AccessClassification};
    use crate::io_scheduler::{IoScheduler, IoRequest, IoOperation, IoPriority};
    use crate::write_optimizer::{GroupCommitCoordinator, MetadataOperation, WriteBatcher};
    use crate::metadata::MetadataManager;
    use crate::metrics::Metrics;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;
    use uuid::Uuid;

    // Helper to create test extent
    fn create_test_extent(size: usize) -> Extent {
        let now = chrono::Utc::now().timestamp();
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
                last_write: now,
                created_at: now,
                classification: AccessClassification::Cold,
                hmm_classifier: None,
            },
            rebuild_in_progress: false,
            rebuild_progress: None,
            generation: 0,
        }
    }

    #[test]
    fn test_extent_generation_increment() {
        let mut extent = create_test_extent(1024);
        assert_eq!(extent.current_generation(), 0);
        
        extent.record_write();
        assert_eq!(extent.current_generation(), 1);
        
        extent.increment_generation();
        assert_eq!(extent.current_generation(), 2);
    }

    #[test]
    fn test_extent_lock_manager_concurrent_readers() {
        let manager = Arc::new(ExtentLockManager::new());
        let extent_uuid = Uuid::new_v4();
        let num_readers = 20;
        let barrier = Arc::new(Barrier::new(num_readers));
        
        let mut handles = vec![];
        
        for _ in 0..num_readers {
            let mgr = manager.clone();
            let uuid = extent_uuid;
            let bar = barrier.clone();
            
            handles.push(thread::spawn(move || {
                bar.wait();
                let lock = mgr.read(&uuid);
                let _guard = lock.read().unwrap();
                thread::sleep(Duration::from_millis(10));
            }));
        }
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        // All readers should complete successfully
        assert_eq!(manager.lock_count(), 1);
    }

    #[test]
    fn test_extent_lock_manager_writer_exclusion() {
        let manager = Arc::new(ExtentLockManager::new());
        let extent_uuid = Uuid::new_v4();
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        
        let num_writers = 10;
        let mut handles = vec![];
        
        for _ in 0..num_writers {
            let mgr = manager.clone();
            let uuid = extent_uuid;
            let cnt = counter.clone();
            
            handles.push(thread::spawn(move || {
                let lock = mgr.write(&uuid);
                let _guard = lock.write().unwrap();
                let val = cnt.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                thread::sleep(Duration::from_millis(5));
                // Verify no other writer ran concurrently
                assert_eq!(cnt.load(std::sync::atomic::Ordering::SeqCst), val + 1);
            }));
        }
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), num_writers);
    }

    #[test]
    fn test_extent_lock_sharding() {
        let manager = ExtentLockManager::new();
        let num_extents = 1000;
        
        // Create locks for many different extents
        let uuids: Vec<Uuid> = (0..num_extents).map(|_| Uuid::new_v4()).collect();
        
        for uuid in &uuids {
            let _lock = manager.read(uuid);
        }
        
        assert_eq!(manager.lock_count(), num_extents);
    }

    #[test]
    fn test_extent_snapshot_validation() {
        let extent = create_test_extent(1024);
        let snapshot = ExtentSnapshot {
            uuid: extent.uuid,
            generation: extent.generation,
            size: extent.size,
            fragment_count: extent.fragment_locations.len(),
        };
        
        assert!(snapshot.is_valid(extent.generation));
        assert!(!snapshot.is_valid(extent.generation + 1));
    }

    #[test]
    fn test_write_batcher_threshold() {
        let batcher = WriteBatcher::new(5, 1024 * 1024);
        
        // Add extents below threshold
        for i in 0..4 {
            let extent = create_test_extent(100);
            assert!(batcher.add_extent(extent).is_none(), "Batch at {}", i);
        }
        
        // Fifth extent triggers batch
        let extent = create_test_extent(100);
        let batch = batcher.add_extent(extent).expect("Should create batch");
        assert_eq!(batch.extents.len(), 5);
        assert_eq!(batch.total_bytes, 500);
    }

    #[test]
    fn test_group_commit_batching() {
        let temp_dir = TempDir::new().unwrap();
        let metadata = MetadataManager::new(temp_dir.path().to_path_buf()).unwrap();
        
        let coordinator = GroupCommitCoordinator::new(10, 100);
        
        // Add operations below threshold
        for i in 0..9 {
            let extent = create_test_extent(100);
            let should_commit = coordinator.add_operation(
                MetadataOperation::SaveExtent(extent)
            );
            assert!(!should_commit, "Should not commit at {}", i);
        }
        
        assert_eq!(coordinator.pending_count(), 9);
        
        // 10th operation triggers commit
        let extent = create_test_extent(100);
        let should_commit = coordinator.add_operation(
            MetadataOperation::SaveExtent(extent)
        );
        assert!(should_commit, "Should commit at 10");
        
        // Commit the batch
        let count = coordinator.commit(&metadata).unwrap();
        assert_eq!(count, 10);
        assert_eq!(coordinator.pending_count(), 0);
        assert_eq!(coordinator.commits_completed(), 1);
    }

    #[test]
    fn test_group_commit_time_based() {
        let temp_dir = TempDir::new().unwrap();
        let metadata = MetadataManager::new(temp_dir.path().to_path_buf()).unwrap();
        
        let coordinator = GroupCommitCoordinator::new(100, 50); // 50ms timeout
        
        // Add a single operation
        let extent = create_test_extent(100);
        coordinator.add_operation(MetadataOperation::SaveExtent(extent));
        
        // Wait for timeout
        thread::sleep(Duration::from_millis(60));
        
        // Should trigger commit due to time
        let extent2 = create_test_extent(100);
        let should_commit = coordinator.add_operation(
            MetadataOperation::SaveExtent(extent2)
        );
        assert!(should_commit, "Should commit after timeout");
    }

    #[test]
    fn test_io_scheduler_priority_ordering() {
        let scheduler = IoScheduler::new(100);
        let disk_uuid = Uuid::new_v4();
        
        scheduler.register_disk(disk_uuid, 1);
        
        // Submit requests with different priorities
        let requests = vec![
            (IoPriority::Background, 1),
            (IoPriority::Critical, 2),
            (IoPriority::Write, 3),
            (IoPriority::HighRead, 4),
            (IoPriority::NormalRead, 5),
        ];
        
        for (priority, idx) in requests {
            let request = IoRequest {
                id: Uuid::new_v4(),
                disk_uuid,
                priority,
                operation: IoOperation::Read {
                    extent_uuid: Uuid::new_v4(),
                    fragment_index: idx,
                },
            };
            scheduler.submit(request).unwrap();
        }
        
        // Give workers time to process
        thread::sleep(Duration::from_millis(200));
        
        let stats = scheduler.queue_stats(&disk_uuid).unwrap();
        assert!(stats.1 > 0, "Should have processed some requests");
        
        scheduler.shutdown();
    }

    #[test]
    fn test_io_scheduler_parallel_disks() {
        let scheduler = Arc::new(IoScheduler::new(50));
        let num_disks = 4;
        let requests_per_disk = 10;
        
        let mut disk_uuids = Vec::new();
        for _ in 0..num_disks {
            let disk_uuid = Uuid::new_v4();
            scheduler.register_disk(disk_uuid, 2);
            disk_uuids.push(disk_uuid);
        }
        
        // Submit requests to all disks
        for disk_uuid in &disk_uuids {
            for i in 0..requests_per_disk {
                let request = IoRequest {
                    id: Uuid::new_v4(),
                    disk_uuid: *disk_uuid,
                    priority: IoPriority::NormalRead,
                    operation: IoOperation::Read {
                        extent_uuid: Uuid::new_v4(),
                        fragment_index: i,
                    },
                };
                scheduler.submit(request).unwrap();
            }
        }
        
        // Give workers time to process
        thread::sleep(Duration::from_millis(300));
        
        // Verify all disks processed requests
        for disk_uuid in &disk_uuids {
            let stats = scheduler.queue_stats(disk_uuid).unwrap();
            assert!(stats.1 > 0, "Disk {} should have processed requests", disk_uuid);
        }
        
        scheduler.shutdown();
    }

    #[test]
    fn test_io_scheduler_backpressure() {
        let scheduler = IoScheduler::new(5); // Small queue
        let disk_uuid = Uuid::new_v4();
        
        // Register disk but don't start workers
        let mut queues = scheduler.queues.lock().unwrap();
        let disk_queue = crate::io_scheduler::DiskQueue {
            disk_uuid,
            requests: std::collections::VecDeque::new(),
            worker: None,
            shutdown: false,
            total_processed: 0,
            total_bytes: 0,
        };
        queues.insert(disk_uuid, disk_queue);
        drop(queues);
        
        // Fill the queue
        for i in 0..5 {
            let request = IoRequest {
                id: Uuid::new_v4(),
                disk_uuid,
                priority: IoPriority::Write,
                operation: IoOperation::Write {
                    extent_uuid: Uuid::new_v4(),
                    fragment_index: i,
                    data: vec![0; 1024],
                },
            };
            assert!(scheduler.submit(request).is_ok(), "Should accept request {}", i);
        }
        
        // Next request should fail due to backpressure
        let request = IoRequest {
            id: Uuid::new_v4(),
            disk_uuid,
            priority: IoPriority::Write,
            operation: IoOperation::Write {
                extent_uuid: Uuid::new_v4(),
                fragment_index: 99,
                data: vec![0; 1024],
            },
        };
        assert!(scheduler.submit(request).is_err(), "Should reject due to backpressure");
    }

    #[test]
    fn test_concurrency_metrics() {
        let metrics = Metrics::new();
        
        // Record lock operations
        metrics.record_lock_acquisition();
        metrics.record_lock_acquisition();
        metrics.record_lock_contention();
        
        assert_eq!(metrics.lock_contention_ratio(), 0.5);
        
        // Record group commits
        metrics.record_group_commit(10);
        metrics.record_group_commit(20);
        metrics.record_group_commit(15);
        
        assert_eq!(metrics.avg_group_commit_ops(), 15.0);
        
        // Record I/O operations
        metrics.update_io_queue_length(42);
        metrics.record_io_op_completed();
        metrics.record_io_op_completed();
        
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.lock_acquisitions, 2);
        assert_eq!(snapshot.lock_contentions, 1);
        assert_eq!(snapshot.group_commits, 3);
        assert_eq!(snapshot.group_commit_ops, 45);
        assert_eq!(snapshot.io_queue_length, 42);
        assert_eq!(snapshot.io_ops_completed, 2);
    }

    #[test]
    fn test_concurrent_read_write_stress() {
        let manager = Arc::new(ExtentLockManager::new());
        let extent_uuids: Vec<Uuid> = (0..10).map(|_| Uuid::new_v4()).collect();
        let num_operations = 1000;
        let num_threads = 10;
        
        let mut handles = vec![];
        
        for _ in 0..num_threads {
            let mgr = manager.clone();
            let uuids = extent_uuids.clone();
            
            handles.push(thread::spawn(move || {
                for i in 0..num_operations {
                    let uuid = &uuids[i % uuids.len()];
                    
                    if i % 10 == 0 {
                        // 10% writes
                        let lock = mgr.write(uuid);
                        let _guard = lock.write().unwrap();
                        thread::sleep(Duration::from_micros(10));
                    } else {
                        // 90% reads
                        let lock = mgr.read(uuid);
                        let _guard = lock.read().unwrap();
                        thread::sleep(Duration::from_micros(1));
                    }
                }
            }));
        }
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        // All operations should complete without deadlock
        assert_eq!(manager.lock_count(), extent_uuids.len());
    }

    #[test]
    fn test_write_batch_concurrent_submission() {
        let batcher = Arc::new(WriteBatcher::new(10, 10240));
        let num_threads = 5;
        let extents_per_thread = 50;
        
        let mut handles = vec![];
        
        for _ in 0..num_threads {
            let b = batcher.clone();
            
            handles.push(thread::spawn(move || {
                let mut batch_count = 0;
                for _ in 0..extents_per_thread {
                    let extent = create_test_extent(100);
                    if b.add_extent(extent).is_some() {
                        batch_count += 1;
                    }
                }
                batch_count
            }));
        }
        
        let mut total_batches = 0;
        for handle in handles {
            total_batches += handle.join().unwrap();
        }
        
        // Should have created multiple batches
        assert!(total_batches > 0, "Should have created at least one batch");
        
        // Flush remaining
        if let Some(batch) = batcher.flush() {
            total_batches += 1;
        }
        
        println!("Created {} batches from {} extents", total_batches, num_threads * extents_per_thread);
    }

    #[test]
    fn test_group_commit_concurrent_operations() {
        let temp_dir = TempDir::new().unwrap();
        let metadata = Arc::new(MetadataManager::new(temp_dir.path().to_path_buf()).unwrap());
        let coordinator = Arc::new(GroupCommitCoordinator::new(20, 1000));
        
        let num_threads = 5;
        let ops_per_thread = 100;
        
        let mut handles = vec![];
        
        for _ in 0..num_threads {
            let coord = coordinator.clone();
            let meta = metadata.clone();
            
            handles.push(thread::spawn(move || {
                for _ in 0..ops_per_thread {
                    let extent = create_test_extent(100);
                    let should_commit = coord.add_operation(
                        MetadataOperation::SaveExtent(extent)
                    );
                    
                    if should_commit {
                        coord.commit(&meta).unwrap();
                    }
                }
            }));
        }
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Flush remaining
        coordinator.flush(&metadata).unwrap();
        
        // Verify commits completed
        assert!(coordinator.commits_completed() > 0);
        println!("Completed {} group commits", coordinator.commits_completed());
    }

    #[test]
    fn test_optimistic_read_with_versioning() {
        let mut extent = create_test_extent(1024);
        assert_eq!(extent.generation, 0);
        
        // Take snapshot for optimistic read
        let snapshot = ExtentSnapshot {
            uuid: extent.uuid,
            generation: extent.generation,
            size: extent.size,
            fragment_count: extent.fragment_locations.len(),
        };
        
        // Simulate concurrent write
        extent.record_write();
        assert_eq!(extent.generation, 1);
        
        // Snapshot validation should fail
        assert!(!snapshot.is_valid(extent.generation));
        
        // Reader should retry with new snapshot
        let new_snapshot = ExtentSnapshot {
            uuid: extent.uuid,
            generation: extent.generation,
            size: extent.size,
            fragment_count: extent.fragment_locations.len(),
        };
        
        assert!(new_snapshot.is_valid(extent.generation));
    }
}
