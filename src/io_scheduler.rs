/// Phase 15: I/O Scheduler with per-disk worker pools
/// 
/// Provides parallel I/O execution with prioritized scheduling and per-disk
/// work queues to maximize throughput while maintaining fairness.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, Condvar};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use uuid::Uuid;

/// I/O request priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IoPriority {
    /// Critical metadata operations
    Critical = 0,
    /// Read requests for hot data
    HighRead = 1,
    /// Normal read requests
    NormalRead = 2,
    /// Write requests
    Write = 3,
    /// Background operations (scrub, GC, etc.)
    Background = 4,
}

/// An I/O request to be executed
#[derive(Debug, Clone)]
pub struct IoRequest {
    pub id: Uuid,
    pub disk_uuid: Uuid,
    pub priority: IoPriority,
    pub operation: IoOperation,
}

/// Type of I/O operation
#[derive(Debug, Clone)]
pub enum IoOperation {
    Read {
        extent_uuid: Uuid,
        fragment_index: usize,
    },
    Write {
        extent_uuid: Uuid,
        fragment_index: usize,
        data: Vec<u8>,
    },
    Delete {
        extent_uuid: Uuid,
        fragment_index: usize,
    },
}

/// Result of an I/O operation
#[derive(Debug, Clone)]
pub struct IoResult {
    pub request_id: Uuid,
    pub success: bool,
    pub data: Option<Vec<u8>>,
    pub error: Option<String>,
}

/// Per-disk work queue
pub struct DiskQueue {
    /// Disk UUID
    pub disk_uuid: Uuid,
    /// Pending requests sorted by priority
    pub requests: VecDeque<IoRequest>,
    /// Worker thread handle
    pub worker: Option<JoinHandle<()>>,
    /// Shutdown signal
    pub shutdown: bool,
    /// Queue statistics
    pub total_processed: u64,
    pub total_bytes: u64,
}

impl DiskQueue {
    fn new(disk_uuid: Uuid) -> Self {
        DiskQueue {
            disk_uuid,
            requests: VecDeque::new(),
            worker: None,
            shutdown: false,
            total_processed: 0,
            total_bytes: 0,
        }
    }
    
    /// Add request to queue (maintains priority order)
    fn enqueue(&mut self, request: IoRequest) {
        // Find insertion point to maintain priority order
        let pos = self.requests
            .iter()
            .position(|r| r.priority > request.priority)
            .unwrap_or(self.requests.len());
        
        self.requests.insert(pos, request);
    }
    
    /// Remove and return highest priority request
    fn dequeue(&mut self) -> Option<IoRequest> {
        self.requests.pop_front()
    }
    
    /// Get queue length
    fn len(&self) -> usize {
        self.requests.len()
    }
    
    /// Check if queue is empty
    fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}

/// I/O scheduler with per-disk worker pools
pub struct IoScheduler {
    /// Per-disk queues
    pub queues: Arc<Mutex<HashMap<Uuid, DiskQueue>>>,
    /// Condition variable for work availability
    work_available: Arc<Condvar>,
    /// Result callback
    result_handler: Arc<Mutex<Option<Box<dyn Fn(IoResult) + Send>>>>,
    /// Maximum queue size per disk (backpressure)
    max_queue_size: usize,
}

impl IoScheduler {
    /// Create new I/O scheduler
    pub fn new(max_queue_size: usize) -> Self {
        IoScheduler {
            queues: Arc::new(Mutex::new(HashMap::new())),
            work_available: Arc::new(Condvar::new()),
            result_handler: Arc::new(Mutex::new(None)),
            max_queue_size,
        }
    }
    
    /// Register a disk with the scheduler
    pub fn register_disk(&self, disk_uuid: Uuid, workers: usize) {
        let mut queues = self.queues.lock().unwrap();
        
        if queues.contains_key(&disk_uuid) {
            return; // Already registered
        }
        
        let queue = DiskQueue::new(disk_uuid);
        queues.insert(disk_uuid, queue);
        drop(queues);
        
        // Start worker threads for this disk
        for worker_id in 0..workers {
            self.start_worker(disk_uuid, worker_id);
        }
    }
    
    /// Start a worker thread for a disk
    fn start_worker(&self, disk_uuid: Uuid, worker_id: usize) {
        let queues = self.queues.clone();
        let work_available = self.work_available.clone();
        let result_handler = self.result_handler.clone();
        
        thread::Builder::new()
            .name(format!("io-worker-{}-{}", disk_uuid, worker_id))
            .spawn(move || {
                log::debug!("I/O worker {}-{} started", disk_uuid, worker_id);
                
                loop {
                    // Wait for work
                    let request = {
                        let mut qs = queues.lock().unwrap();
                        
                        // Wait for work or shutdown
                        while !qs.get(&disk_uuid).map_or(false, |q| q.shutdown || !q.is_empty()) {
                            qs = work_available.wait(qs).unwrap();
                        }
                        
                        let queue = qs.get_mut(&disk_uuid).unwrap();
                        
                        // Check for shutdown
                        if queue.shutdown && queue.is_empty() {
                            log::debug!("I/O worker {}-{} shutting down", disk_uuid, worker_id);
                            return;
                        }
                        
                        queue.dequeue()
                    };
                    
                    // Execute request
                    if let Some(req) = request {
                        let result = Self::execute_request(&req);
                        
                        // Update stats
                        {
                            let mut qs = queues.lock().unwrap();
                            if let Some(queue) = qs.get_mut(&disk_uuid) {
                                queue.total_processed += 1;
                                if let Some(ref data) = result.data {
                                    queue.total_bytes += data.len() as u64;
                                }
                            }
                        }
                        
                        // Invoke callback
                        if let Some(ref handler) = *result_handler.lock().unwrap() {
                            handler(result);
                        }
                    }
                }
            })
            .expect("Failed to spawn I/O worker thread");
    }
    
    /// Execute an I/O request (stub - actual implementation would interact with disk)
    fn execute_request(request: &IoRequest) -> IoResult {
        // This is a stub - actual implementation would interact with the disk
        log::trace!(
            "Executing I/O request {} on disk {} with priority {:?}",
            request.id,
            request.disk_uuid,
            request.priority
        );
        
        // Simulate I/O latency
        thread::sleep(Duration::from_micros(100));
        
        IoResult {
            request_id: request.id,
            success: true,
            data: None,
            error: None,
        }
    }
    
    /// Submit an I/O request
    /// 
    /// Returns Ok(()) if enqueued, Err if queue is full (backpressure)
    pub fn submit(&self, request: IoRequest) -> Result<(), String> {
        let mut queues = self.queues.lock().unwrap();
        
        let queue = queues.get_mut(&request.disk_uuid)
            .ok_or_else(|| format!("Disk {} not registered", request.disk_uuid))?;
        
        // Check backpressure
        if queue.len() >= self.max_queue_size {
            return Err(format!("Queue full for disk {}", request.disk_uuid));
        }
        
        queue.enqueue(request);
        
        // Signal workers
        drop(queues);
        self.work_available.notify_all();
        
        Ok(())
    }
    
    /// Set result handler callback
    pub fn set_result_handler<F>(&self, handler: F)
    where
        F: Fn(IoResult) + Send + 'static,
    {
        *self.result_handler.lock().unwrap() = Some(Box::new(handler));
    }
    
    /// Get queue statistics for a disk
    pub fn queue_stats(&self, disk_uuid: &Uuid) -> Option<(usize, u64, u64)> {
        let queues = self.queues.lock().unwrap();
        queues.get(disk_uuid).map(|q| (q.len(), q.total_processed, q.total_bytes))
    }
    
    /// Get total queue lengths across all disks
    pub fn total_queue_length(&self) -> usize {
        let queues = self.queues.lock().unwrap();
        queues.values().map(|q| q.len()).sum()
    }
    
    /// Shutdown the scheduler and all workers
    pub fn shutdown(&self) {
        let mut queues = self.queues.lock().unwrap();
        
        for queue in queues.values_mut() {
            queue.shutdown = true;
        }
        
        drop(queues);
        self.work_available.notify_all();
        
        // Wait a bit for workers to finish
        thread::sleep(Duration::from_millis(100));
    }
}

impl Drop for IoScheduler {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
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
}
