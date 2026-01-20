use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::hmm_classifier::HmmClassifier;

/// Redundancy policy for an extent
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RedundancyPolicy {
    /// Store N complete replicas
    Replication { copies: usize },
    /// Reed-Solomon erasure coding with k data + m parity shards
    ErasureCoding { data_shards: usize, parity_shards: usize },
}

impl RedundancyPolicy {
    /// Get the total number of fragments for this policy
    pub fn fragment_count(&self) -> usize {
        match self {
            RedundancyPolicy::Replication { copies } => *copies,
            RedundancyPolicy::ErasureCoding { data_shards, parity_shards } => {
                data_shards + parity_shards
            }
        }
    }
    
    /// Get the minimum number of fragments needed to reconstruct
    pub fn min_fragments(&self) -> usize {
        match self {
            RedundancyPolicy::Replication { .. } => 1,
            RedundancyPolicy::ErasureCoding { data_shards, .. } => *data_shards,
        }
    }
    
    /// Check if we can upgrade/downgrade between policies
    pub fn can_transition_from(&self, _other: RedundancyPolicy) -> bool {
        // Any policy can transition from any other policy - we're re-encoding the data
        true
    }
}

/// Track policy change history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyTransition {
    pub from_policy: RedundancyPolicy,
    pub to_policy: RedundancyPolicy,
    pub timestamp: i64,
    pub status: TransitionStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransitionStatus {
    Pending,      // Change initiated, not yet applied
    InProgress,   // Currently reencoding
    Committed,    // Successfully applied
    RolledBack,   // Change reverted
}

/// Hot/Cold classification based on access patterns
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AccessClassification {
    Hot,   // Frequently accessed
    Warm,  // Moderately accessed
    Cold,  // Rarely accessed
}

/// Access statistics for an extent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessStats {
    pub read_count: u64,
    pub write_count: u64,
    pub last_read: i64,
    pub last_write: i64,
    pub created_at: i64,
    pub classification: AccessClassification,
    /// HMM classifier for more sophisticated state transitions
    #[serde(skip)]
    pub hmm_classifier: Option<HmmClassifier>,
}

/// Represents an immutable extent (chunk of file data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extent {
    pub uuid: Uuid,
    pub size: usize,
    pub checksum: [u8; 32], // BLAKE3 hash
    pub redundancy: RedundancyPolicy,
    pub fragment_locations: Vec<FragmentLocation>,
    
    // New fields for policy changes
    pub previous_policy: Option<RedundancyPolicy>,
    pub policy_transitions: Vec<PolicyTransition>,
    pub last_policy_change: Option<i64>,
    
    // New fields for hot/cold classification
    pub access_stats: AccessStats,
}

/// Location of a fragment on a disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentLocation {
    pub disk_uuid: Uuid,
    pub fragment_index: usize,
}

impl Extent {
    pub fn new(data: &[u8], redundancy: RedundancyPolicy) -> Self {
        let uuid = Uuid::new_v4();
        let size = data.len();
        let now = chrono::Utc::now().timestamp();
        
        // Compute checksum
        let checksum = blake3::hash(data);
        
        Extent {
            uuid,
            size,
            checksum: checksum.into(),
            redundancy,
            fragment_locations: Vec::new(),
            previous_policy: None,
            policy_transitions: Vec::new(),
            last_policy_change: None,
            access_stats: AccessStats {
                read_count: 0,
                write_count: 1, // Initialized with 1 for the initial write
                last_read: 0,
                last_write: now,
                created_at: now,
                classification: AccessClassification::Cold,
                hmm_classifier: Some(HmmClassifier::new()),
            },
        }
    }
    
    /// Verify checksum against data
    pub fn verify_checksum(&self, data: &[u8]) -> bool {
        let computed = blake3::hash(data);
        computed.as_bytes() == &self.checksum
    }
    
    /// Check if we have minimum fragments for reconstruction
    pub fn is_readable(&self) -> bool {
        self.fragment_locations.len() >= self.redundancy.min_fragments()
    }
    
    /// Check if we have all fragments
    pub fn is_complete(&self) -> bool {
        self.fragment_locations.len() == self.redundancy.fragment_count()
    }
    
    /// Check if extent is in the middle of a policy change
    pub fn is_transitioning(&self) -> bool {
        self.policy_transitions.iter().any(|t| t.status == TransitionStatus::InProgress)
    }
    
    /// Initiate a policy change
    pub fn initiate_policy_change(&mut self, new_policy: RedundancyPolicy) -> anyhow::Result<()> {
        if !new_policy.can_transition_from(self.redundancy) {
            return Err(anyhow!("Cannot transition to policy with fewer minimum fragments"));
        }
        
        if self.is_transitioning() {
            return Err(anyhow!("Extent is already in the middle of a policy transition"));
        }
        
        let now = chrono::Utc::now().timestamp();
        
        self.policy_transitions.push(PolicyTransition {
            from_policy: self.redundancy,
            to_policy: new_policy,
            timestamp: now,
            status: TransitionStatus::Pending,
        });
        
        Ok(())
    }
    
    /// Mark policy change as in-progress
    pub fn mark_transition_in_progress(&mut self) {
        if let Some(transition) = self.policy_transitions.last_mut() {
            if transition.status == TransitionStatus::Pending {
                transition.status = TransitionStatus::InProgress;
            }
        }
    }
    
    /// Commit a policy change
    pub fn commit_policy_change(&mut self, new_policy: RedundancyPolicy) -> anyhow::Result<()> {
        if !self.is_transitioning() {
            return Err(anyhow::anyhow!("No policy transition in progress"));
        }
        
        self.previous_policy = Some(self.redundancy);
        self.redundancy = new_policy;
        self.last_policy_change = Some(chrono::Utc::now().timestamp());
        
        if let Some(transition) = self.policy_transitions.last_mut() {
            transition.status = TransitionStatus::Committed;
        }
        
        // NOTE: Do NOT clear fragment_locations here - they were just updated with new fragments
        // during the rebundle process and need to be persisted
        
        Ok(())
    }
    
    /// Rollback a policy change
    pub fn rollback_policy_change(&mut self) {
        if let Some(transition) = self.policy_transitions.last_mut() {
            if transition.status == TransitionStatus::InProgress {
                transition.status = TransitionStatus::RolledBack;
            }
        }
    }
    
    /// Get policy change history
    pub fn get_policy_history(&self) -> Vec<(RedundancyPolicy, i64)> {
        std::iter::once((self.redundancy, self.last_policy_change.unwrap_or(0)))
            .chain(
                self.policy_transitions
                    .iter()
                    .filter(|t| t.status == TransitionStatus::Committed)
                    .map(|t| (t.from_policy, t.timestamp))
            )
            .collect()
    }
    
    /// Record a read access
    pub fn record_read(&mut self) {
        self.access_stats.read_count += 1;
        self.access_stats.last_read = chrono::Utc::now().timestamp();
        self.reclassify();
    }
    
    /// Record a write access
    pub fn record_write(&mut self) {
        self.access_stats.write_count += 1;
        self.access_stats.last_write = chrono::Utc::now().timestamp();
        self.reclassify();
    }
    
    /// Get access frequency (operations per day)
    pub fn access_frequency(&self) -> f64 {
        let now = chrono::Utc::now().timestamp();
        let age_seconds = (now - self.access_stats.created_at).max(1);
        let age_days = age_seconds as f64 / 86400.0;
        
        let total_ops = (self.access_stats.read_count + self.access_stats.write_count) as f64;
        total_ops / age_days.max(1.0)
    }
    
    /// Reclassify the extent as hot/warm/cold based on access patterns
    /// Uses HMM for more sophisticated state transitions
    pub fn reclassify(&mut self) {
        let now = chrono::Utc::now().timestamp();
        
        // Recency score: lower is better (more recent)
        let recency_hours = (now - self.access_stats.last_read.max(self.access_stats.last_write)) / 3600;
        
        // Frequency score: operations per day
        let frequency = self.access_frequency();
        
        // Use HMM for classification if available
        if let Some(ref mut hmm) = self.access_stats.hmm_classifier {
            let new_classification = hmm.classify(
                frequency,
                recency_hours,
                self.access_stats.classification,
            );
            
            self.access_stats.classification = new_classification;
        } else {
            // Fallback to simple threshold-based classification
            self.access_stats.classification = if frequency > 100.0 || recency_hours < 1 {
                AccessClassification::Hot
            } else if frequency > 10.0 || recency_hours < 24 {
                AccessClassification::Warm
            } else {
                AccessClassification::Cold
            };
        }
    }
    
    /// Get current classification
    pub fn classification(&self) -> AccessClassification {
        self.access_stats.classification
    }
    
    /// Get access statistics
    pub fn access_stats(&self) -> &AccessStats {
        &self.access_stats
    }
    
    /// Get recommended policy based on classification
    /// Hot/Warm data uses replication for fast access
    /// Cold data uses erasure coding for storage efficiency
    pub fn recommended_policy(&self) -> RedundancyPolicy {
        match self.access_stats.classification {
            AccessClassification::Hot | AccessClassification::Warm => {
                // Hot/Warm: Use replication for fast reads
                RedundancyPolicy::Replication { copies: 3 }
            }
            AccessClassification::Cold => {
                // Cold: Use erasure coding for efficiency
                RedundancyPolicy::ErasureCoding {
                    data_shards: 4,
                    parity_shards: 2,
                }
            }
        }
    }
    
    /// Check if extent should be migrated based on current classification
    pub fn should_migrate(&self) -> bool {
        let recommended = self.recommended_policy();
        // Migrate if recommended policy differs from current
        recommended != self.redundancy
    }
}

/// Default extent size: 1 MB
pub const DEFAULT_EXTENT_SIZE: usize = 1024 * 1024;

/// Split data into extents
pub fn split_into_extents(data: &[u8], redundancy: RedundancyPolicy) -> Vec<Extent> {
    let mut extents = Vec::new();
    
    for chunk in data.chunks(DEFAULT_EXTENT_SIZE) {
        extents.push(Extent::new(chunk, redundancy));
    }
    
    // Handle empty data
    if extents.is_empty() {
        extents.push(Extent::new(&[], redundancy));
    }
    
    extents
}
