// moved from src/io_scheduler.rs
use super::*;
    
    #[test]
    fn test_disk_queue_priority() {
        let mut queue = DiskQueue::new(Uuid::new_v4());
        let disk_uuid = queue.disk_uuid;
        
        // Add requests with different priorities
        queue.enqueue(IoRequest {
            id: Uuid::new_v4(),
            disk_uuid,
            priority: IoPriority::Write,
            operation: IoOperation::Write {
                extent_uuid: Uuid::new_v4(),
                fragment_index: 0,
                data: vec![0; 100],
            },
        });
        
        queue.enqueue(IoRequest {
            id: Uuid::new_v4(),
            disk_uuid,
            priority: IoPriority::Critical,
            operation: IoOperation::Read {
                extent_uuid: Uuid::new_v4(),
                fragment_index: 0,
            },
        });
        
        queue.enqueue(IoRequest {
            id: Uuid::new_v4(),
            disk_uuid,
            priority: IoPriority::NormalRead,
            operation: IoOperation::Read {
                extent_uuid: Uuid::new_v4(),
                fragment_index: 0,
            },
        });
        
        // Dequeue should return in priority order
        assert_eq!(queue.dequeue().unwrap().priority, IoPriority::Critical);
        assert_eq!(queue.dequeue().unwrap().priority, IoPriority::NormalRead);
        assert_eq!(queue.dequeue().unwrap().priority, IoPriority::Write);
        assert!(queue.is_empty());
    }
    
    #[test]
    fn test_io_scheduler_basic() {
        let scheduler = IoScheduler::new(100);
        let disk_uuid = Uuid::new_v4();
        
        // Register disk with 2 workers
        scheduler.register_disk(disk_uuid, 2);
        
        // Submit some requests
        for i in 0..10 {
            let priority = if i % 2 == 0 {
                IoPriority::HighRead
            } else {
                IoPriority::Write
            };
            
            let request = IoRequest {
                id: Uuid::new_v4(),
                disk_uuid,
                priority,
                operation: IoOperation::Read {
                    extent_uuid: Uuid::new_v4(),
                    fragment_index: i,
                },
            };
            
            scheduler.submit(request).unwrap();
        }
        
        // Give workers time to process
        thread::sleep(Duration::from_millis(200));
        
        // Check stats
        let stats = scheduler.queue_stats(&disk_uuid).unwrap();
        assert!(stats.1 > 0); // total_processed > 0
        
        scheduler.shutdown();
    }
    
    #[test]
    fn test_backpressure() {
        let scheduler = IoScheduler::new(5); // Small queue
        let disk_uuid = Uuid::new_v4();
        
        // Don't register disk (no workers) to fill queue
        scheduler.queues.lock().unwrap().insert(disk_uuid, DiskQueue::new(disk_uuid));
        
        // Fill queue
        for i in 0..5 {
            let request = IoRequest {
                id: Uuid::new_v4(),
                disk_uuid,
                priority: IoPriority::Write,
                operation: IoOperation::Write {
                    extent_uuid: Uuid::new_v4(),
                    fragment_index: i,
                    data: vec![0; 100],
                },
            };
            assert!(scheduler.submit(request).is_ok());
        }
        
        // Next request should fail (backpressure)
        let request = IoRequest {
            id: Uuid::new_v4(),
            disk_uuid,
            priority: IoPriority::Write,
            operation: IoOperation::Write {
                extent_uuid: Uuid::new_v4(),
                fragment_index: 99,
                data: vec![0; 100],
            },
        };
        assert!(scheduler.submit(request).is_err());
    }
