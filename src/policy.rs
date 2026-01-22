use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::metadata_btree::PersistedBTree;

/// Policy metadata types that can be stored persistently
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyMetadata {
    HmmClassifier(crate::hmm_classifier::HmmClassifier),
    TieringPolicy(TieringPolicy),
    DefragPolicy(DefragPolicy),
    ScrubPolicy(ScrubPolicy),
}

/// Tiering policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TieringPolicy {
    pub hot_threshold_days: u32,
    pub warm_threshold_days: u32,
    pub migration_batch_size: usize,
    pub max_parallel_migrations: usize,
}

/// Defragmentation policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefragPolicy {
    pub fragmentation_threshold: f64,
    pub max_defrag_time_seconds: u64,
    pub defrag_intensity: DefragIntensity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DefragIntensity {
    Low,
    Medium,
    High,
}

/// Scrubbing policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrubPolicy {
    pub scrub_interval_hours: u64,
    pub max_scrub_time_seconds: u64,
    pub scrub_intensity: ScrubIntensity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScrubIntensity {
    Low,
    Medium,
    High,
}

/// Persistent policy metadata store using B-tree
pub struct PolicyStore {
    store: PersistedBTree<String, PolicyMetadata>,
}

impl PolicyStore {
    pub fn new(persist_path: Option<PathBuf>) -> Result<Self> {
        let store = PersistedBTree::new(persist_path)?;
        Ok(PolicyStore { store })
    }

    /// Store HMM classifier state
    pub fn set_hmm_classifier(&self, classifier: crate::hmm_classifier::HmmClassifier) -> Result<()> {
        self.store.insert("hmm_classifier".to_string(), PolicyMetadata::HmmClassifier(classifier))
    }

    /// Get HMM classifier state
    pub fn get_hmm_classifier(&self) -> Option<crate::hmm_classifier::HmmClassifier> {
        match self.store.get(&"hmm_classifier".to_string()) {
            Some(PolicyMetadata::HmmClassifier(c)) => Some(c),
            _ => None,
        }
    }

    /// Store tiering policy
    pub fn set_tiering_policy(&self, policy: TieringPolicy) -> Result<()> {
        self.store.insert("tiering_policy".to_string(), PolicyMetadata::TieringPolicy(policy))
    }

    /// Get tiering policy
    pub fn get_tiering_policy(&self) -> Option<TieringPolicy> {
        match self.store.get(&"tiering_policy".to_string()) {
            Some(PolicyMetadata::TieringPolicy(p)) => Some(p),
            _ => None,
        }
    }

    /// Store defrag policy
    pub fn set_defrag_policy(&self, policy: DefragPolicy) -> Result<()> {
        self.store.insert("defrag_policy".to_string(), PolicyMetadata::DefragPolicy(policy))
    }

    /// Get defrag policy
    pub fn get_defrag_policy(&self) -> Option<DefragPolicy> {
        match self.store.get(&"defrag_policy".to_string()) {
            Some(PolicyMetadata::DefragPolicy(p)) => Some(p),
            _ => None,
        }
    }

    /// Store scrub policy
    pub fn set_scrub_policy(&self, policy: ScrubPolicy) -> Result<()> {
        self.store.insert("scrub_policy".to_string(), PolicyMetadata::ScrubPolicy(policy))
    }

    /// Get scrub policy
    pub fn get_scrub_policy(&self) -> Option<ScrubPolicy> {
        match self.store.get(&"scrub_policy".to_string()) {
            Some(PolicyMetadata::ScrubPolicy(p)) => Some(p),
            _ => None,
        }
    }

    /// Store custom policy metadata
    pub fn set_custom(&self, key: String, metadata: PolicyMetadata) -> Result<()> {
        self.store.insert(key, metadata)
    }

    /// Get custom policy metadata
    pub fn get_custom(&self, key: &str) -> Option<&PolicyMetadata> {
        self.store.get(&key.to_string())
    }

    /// Remove policy metadata
    pub fn remove(&self, key: &str) -> Result<Option<PolicyMetadata>> {
        self.store.remove(&key.to_string())
    }

    /// List all stored policy keys
    pub fn list_keys(&self) -> Vec<String> {
        self.store.list_keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_policy_store_basic() -> Result<()> {
        let tmp = NamedTempFile::new()?;
        let store = PolicyStore::new(Some(tmp.path().to_path_buf()))?;

        // Test tiering policy
        let tier_policy = TieringPolicy {
            hot_threshold_days: 7,
            warm_threshold_days: 30,
            migration_batch_size: 100,
            max_parallel_migrations: 4,
        };
        store.set_tiering_policy(tier_policy.clone())?;
        let retrieved = store.get_tiering_policy().unwrap();
        assert_eq!(retrieved.hot_threshold_days, 7);

        // Test defrag policy
        let defrag_policy = DefragPolicy {
            fragmentation_threshold: 0.3,
            max_defrag_time_seconds: 3600,
            defrag_intensity: DefragIntensity::Medium,
        };
        store.set_defrag_policy(defrag_policy.clone())?;
        let retrieved = store.get_defrag_policy().unwrap();
        assert_eq!(retrieved.fragmentation_threshold, 0.3);

        Ok(())
    }
}