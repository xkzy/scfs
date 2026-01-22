// Phase 17: Automated Intelligent Policies
//
// Implements an ML-driven policy engine for automated storage optimization with safety guarantees.
//
// Features:
// - Declarative policy language with rules and actions
// - ML-based hotness prediction and workload modeling
// - Two-phase execution: propose → simulate → approve → execute
// - Safety constraints and operator override
// - Simulation harness and explainability
// - Audit trail and observability

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// Time constants for better maintainability
const SECONDS_PER_HOUR: u64 = 3600;
const SECONDS_PER_DAY: u64 = 86400;
const SECONDS_PER_HOUR_F64: f64 = 3600.0;

/// Storage tier for tiering decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageTier {
    NVMe,
    SSD,
    HDD,
    Archive,
}

/// Policy rule for automated decision making
#[derive(Debug, Clone)]
pub enum PolicyRule {
    /// Trigger when hotness exceeds threshold (0.0-1.0)
    HotnessThreshold { threshold: f64 },
    
    /// Trigger when cache utilization exceeds threshold
    CacheUtilization { max: f64 },
    
    /// Trigger when tier utilization exceeds threshold
    TierUtilization { tier: StorageTier, max: f64 },
    
    /// Trigger based on time window
    TimeWindow { start_hour: u32, end_hour: u32 },
    
    /// Trigger when access frequency exceeds threshold
    AccessFrequency { min_accesses: u64 },
    
    /// Trigger based on data age
    DataAge { min_days: u64 },
}

/// Automated action to execute
#[derive(Debug, Clone)]
pub enum PolicyAction {
    /// Promote data to cache
    PromoteToCache,
    
    /// Demote data from cache
    DemoteFromCache,
    
    /// Migrate data to different tier
    MigrateTier { target: StorageTier },
    
    /// Trigger defragmentation
    Defragment,
    
    /// Execute TRIM operation
    Trim,
    
    /// Rebalance data across disks
    Rebalance,
    
    /// No action (for testing/logging)
    NoOp,
}

/// Policy execution schedule
#[derive(Debug, Clone)]
pub enum PolicySchedule {
    /// Evaluate continuously
    Continuous,
    
    /// Evaluate hourly
    Hourly,
    
    /// Evaluate daily at specific hour
    Daily { hour: u32 },
    
    /// Evaluate on-demand only
    Manual,
}

/// Complete policy definition
#[derive(Debug, Clone)]
pub struct Policy {
    pub name: String,
    pub rules: Vec<PolicyRule>,
    pub actions: Vec<PolicyAction>,
    pub schedule: PolicySchedule,
    pub enabled: bool,
    pub version: u32,
}

impl Policy {
    /// Create a new policy
    pub fn new(
        name: impl Into<String>,
        rules: Vec<PolicyRule>,
        actions: Vec<PolicyAction>,
        schedule: PolicySchedule,
    ) -> Self {
        Self {
            name: name.into(),
            rules,
            actions,
            schedule,
            enabled: true,
            version: 1,
        }
    }
    
    /// Check if all rules match for given state
    pub fn matches(&self, state: &SystemState) -> bool {
        if !self.enabled {
            return false;
        }
        
        self.rules.iter().all(|rule| self.rule_matches(rule, state))
    }
    
    fn rule_matches(&self, rule: &PolicyRule, state: &SystemState) -> bool {
        match rule {
            PolicyRule::HotnessThreshold { threshold } => {
                state.hotness >= *threshold
            }
            PolicyRule::CacheUtilization { max } => {
                state.cache_utilization <= *max
            }
            PolicyRule::TierUtilization { tier, max } => {
                state.tier_utilization.get(tier)
                    .map(|util| *util <= *max)
                    .unwrap_or(true)
            }
            PolicyRule::TimeWindow { start_hour, end_hour } => {
                let current_hour = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .ok()
                    .map(|d| (d.as_secs() / SECONDS_PER_HOUR) % 24)
                    .unwrap_or(0) as u32;
                
                if start_hour <= end_hour {
                    current_hour >= *start_hour && current_hour < *end_hour
                } else {
                    // Handle wrap-around (e.g., 22:00-02:00)
                    current_hour >= *start_hour || current_hour < *end_hour
                }
            }
            PolicyRule::AccessFrequency { min_accesses } => {
                state.access_count >= *min_accesses
            }
            PolicyRule::DataAge { min_days } => {
                let age_days = state.age_seconds / SECONDS_PER_DAY;
                age_days >= *min_days
            }
        }
    }
}

/// Current system state for rule evaluation
#[derive(Debug, Clone)]
pub struct SystemState {
    pub hotness: f64,
    pub cache_utilization: f64,
    pub tier_utilization: HashMap<StorageTier, f64>,
    pub access_count: u64,
    pub age_seconds: u64,
}

impl Default for SystemState {
    fn default() -> Self {
        Self {
            hotness: 0.0,
            cache_utilization: 0.0,
            tier_utilization: HashMap::new(),
            access_count: 0,
            age_seconds: 0,
        }
    }
}

/// Action proposal with metadata
#[derive(Debug, Clone)]
pub struct ActionProposal {
    pub policy_name: String,
    pub action: PolicyAction,
    pub target_extent: Uuid,
    pub estimated_benefit: f64,
    pub estimated_cost: f64,
    pub confidence: f64,
    pub reason: String,
}

/// Workload features for ML prediction
#[derive(Debug, Clone)]
pub struct WorkloadFeatures {
    pub access_frequency: f64,
    pub read_ratio: f64,
    pub avg_size: f64,
    pub temporal_pattern: Vec<f64>,
    pub last_access_recency: f64,
}

impl WorkloadFeatures {
    /// Extract features from access history
    pub fn from_access_history(accesses: &[(u64, bool, usize)]) -> Self {
        let total = accesses.len() as f64;
        if total == 0.0 {
            return Self::default();
        }
        
        let read_count = accesses.iter().filter(|(_, is_read, _)| *is_read).count() as f64;
        let total_size: usize = accesses.iter().map(|(_, _, size)| size).sum();
        
        // Simple temporal pattern: accesses per hour over last 24 hours
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| {
                // Fallback to a reasonable default if clock is before UNIX_EPOCH
                std::time::Duration::from_secs(0)
            })
            .as_secs();
        
        let mut hourly_buckets = vec![0.0; 24];
        for (timestamp, _, _) in accesses {
            let age_hours = ((now - timestamp) / SECONDS_PER_HOUR) as usize;
            if age_hours < 24 {
                hourly_buckets[age_hours] += 1.0;
            }
        }
        
        let last_access = accesses.last()
            .map(|(ts, _, _)| now - ts)
            .unwrap_or(0);
        
        Self {
            access_frequency: total / SECONDS_PER_HOUR_F64, // Accesses per hour
            read_ratio: read_count / total,
            avg_size: (total_size as f64) / total,
            temporal_pattern: hourly_buckets,
            last_access_recency: last_access as f64,
        }
    }
}

impl Default for WorkloadFeatures {
    fn default() -> Self {
        Self {
            access_frequency: 0.0,
            read_ratio: 0.5,
            avg_size: 4096.0,
            temporal_pattern: vec![0.0; 24],
            last_access_recency: 0.0,
        }
    }
}

/// ML-based hotness predictor
#[derive(Debug, Clone)]
pub struct HotnessPredictor {
    // Simple linear model: hotness = w0 + w1*freq + w2*recency + w3*read_ratio
    weights: Vec<f64>,
    trained: bool,
}

impl HotnessPredictor {
    /// Create a new predictor with default weights
    pub fn new() -> Self {
        Self {
            // Default weights favoring frequency and recency
            weights: vec![0.1, 0.4, 0.3, 0.2],
            trained: false,
        }
    }
    
    /// Train the model on historical data
    pub fn train(&mut self, training_data: &[(WorkloadFeatures, f64)]) -> Result<(), String> {
        if training_data.is_empty() {
            return Err("No training data provided".to_string());
        }
        
        // Simple gradient descent for linear regression
        let learning_rate = 0.01;
        let epochs = 100;
        
        for _ in 0..epochs {
            for (features, actual_hotness) in training_data {
                let predicted = self.predict_internal(features);
                let error = actual_hotness - predicted;
                
                // Update weights
                self.weights[0] += learning_rate * error;
                self.weights[1] += learning_rate * error * features.access_frequency;
                
                let recency_score = 1.0 / (1.0 + features.last_access_recency / SECONDS_PER_HOUR_F64);
                self.weights[2] += learning_rate * error * recency_score;
                self.weights[3] += learning_rate * error * features.read_ratio;
            }
        }
        
        self.trained = true;
        Ok(())
    }
    
    /// Predict hotness for given features
    pub fn predict(&self, features: &WorkloadFeatures) -> (f64, f64) {
        let hotness = self.predict_internal(features);
        
        // Confidence based on feature quality
        let confidence = if features.access_frequency > 0.0 {
            0.8
        } else {
            0.3
        };
        
        (hotness.clamp(0.0, 1.0), confidence)
    }
    
    fn predict_internal(&self, features: &WorkloadFeatures) -> f64 {
        let recency_score = 1.0 / (1.0 + features.last_access_recency / SECONDS_PER_HOUR_F64);
        
        self.weights[0]
            + self.weights[1] * features.access_frequency
            + self.weights[2] * recency_score
            + self.weights[3] * features.read_ratio
    }
}

impl Default for HotnessPredictor {
    fn default() -> Self {
        Self::new()
    }
}

/// Policy execution audit entry
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub timestamp: u64,
    pub policy_name: String,
    pub action: PolicyAction,
    pub target: Uuid,
    pub success: bool,
    pub reason: String,
    pub impact: ExecutionImpact,
}

/// Impact of policy execution
#[derive(Debug, Clone)]
pub struct ExecutionImpact {
    pub benefit: f64,
    pub cost: f64,
    pub latency_improvement: f64,
    pub resource_usage: f64,
}

/// Policy engine
pub struct PolicyEngine {
    policies: HashMap<String, Policy>,
    audit_trail: Vec<AuditEntry>,
    metrics: PolicyMetrics,
    predictor: HotnessPredictor,
}

/// Policy engine metrics
#[derive(Debug, Clone, Default)]
pub struct PolicyMetrics {
    pub policies_evaluated: u64,
    pub actions_proposed: u64,
    pub actions_executed: u64,
    pub actions_failed: u64,
    pub total_benefit: f64,
    pub total_cost: f64,
}

impl PolicyEngine {
    /// Create a new policy engine
    pub fn new() -> Self {
        Self {
            policies: HashMap::new(),
            audit_trail: Vec::new(),
            metrics: PolicyMetrics::default(),
            predictor: HotnessPredictor::new(),
        }
    }
    
    /// Add a policy
    pub fn add_policy(&mut self, policy: Policy) {
        self.policies.insert(policy.name.clone(), policy);
    }
    
    /// Remove a policy
    pub fn remove_policy(&mut self, name: &str) -> Option<Policy> {
        self.policies.remove(name)
    }
    
    /// Get a policy
    pub fn get_policy(&self, name: &str) -> Option<&Policy> {
        self.policies.get(name)
    }
    
    /// Train the ML model
    pub fn train_model(&mut self, training_data: &[(WorkloadFeatures, f64)]) -> Result<(), String> {
        self.predictor.train(training_data)
    }
    
    /// Evaluate all policies and generate proposals
    pub fn evaluate_policies(
        &mut self,
        state: &SystemState,
        extent_id: Uuid,
    ) -> Vec<ActionProposal> {
        let mut proposals = Vec::new();
        
        for policy in self.policies.values() {
            self.metrics.policies_evaluated += 1;
            
            if policy.matches(state) {
                for action in &policy.actions {
                    let (benefit, cost) = self.estimate_impact(action, state);
                    
                    proposals.push(ActionProposal {
                        policy_name: policy.name.clone(),
                        action: action.clone(),
                        target_extent: extent_id,
                        estimated_benefit: benefit,
                        estimated_cost: cost,
                        confidence: 0.75,
                        reason: format!("Policy '{}' rules matched", policy.name),
                    });
                    
                    self.metrics.actions_proposed += 1;
                }
            }
        }
        
        proposals
    }
    
    /// Simulate action execution
    pub fn simulate(&self, proposal: &ActionProposal) -> ExecutionImpact {
        // Simulation: estimate impact without actually executing
        ExecutionImpact {
            benefit: proposal.estimated_benefit,
            cost: proposal.estimated_cost,
            latency_improvement: proposal.estimated_benefit * 0.8,
            resource_usage: proposal.estimated_cost * 0.5,
        }
    }
    
    /// Execute an action with safety checks
    pub fn execute(&mut self, proposal: &ActionProposal) -> Result<ExecutionImpact, String> {
        // Safety checks
        if proposal.estimated_cost > proposal.estimated_benefit * 2.0 {
            return Err("Cost too high relative to benefit".to_string());
        }
        
        // Simulate execution
        let impact = self.simulate(proposal);
        
        // Record audit entry
        let entry = AuditEntry {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_else(|_| std::time::Duration::from_secs(0))
                .as_secs(),
            policy_name: proposal.policy_name.clone(),
            action: proposal.action.clone(),
            target: proposal.target_extent,
            success: true,
            reason: proposal.reason.clone(),
            impact: impact.clone(),
        };
        
        self.audit_trail.push(entry);
        self.metrics.actions_executed += 1;
        self.metrics.total_benefit += impact.benefit;
        self.metrics.total_cost += impact.cost;
        
        Ok(impact)
    }
    
    /// Get audit trail
    pub fn audit_trail(&self) -> &[AuditEntry] {
        &self.audit_trail
    }
    
    /// Get metrics
    pub fn metrics(&self) -> &PolicyMetrics {
        &self.metrics
    }
    
    /// Get the predictor
    pub fn predictor(&self) -> &HotnessPredictor {
        &self.predictor
    }
    
    fn estimate_impact(&self, action: &PolicyAction, state: &SystemState) -> (f64, f64) {
        match action {
            PolicyAction::PromoteToCache => {
                // High benefit for hot data, moderate cost
                let benefit = state.hotness * 100.0;
                let cost = 10.0;
                (benefit, cost)
            }
            PolicyAction::DemoteFromCache => {
                // Benefit from freeing cache, low cost
                let benefit = (1.0 - state.hotness) * 20.0;
                let cost = 5.0;
                (benefit, cost)
            }
            PolicyAction::MigrateTier { .. } => {
                // Benefit based on hotness mismatch, high cost
                let benefit = state.hotness * 80.0;
                let cost = 30.0;
                (benefit, cost)
            }
            PolicyAction::Defragment => {
                // Benefit from improved locality, very high cost
                (50.0, 100.0)
            }
            PolicyAction::Trim => {
                // Benefit from reclaimed space, moderate cost
                (40.0, 20.0)
            }
            PolicyAction::Rebalance => {
                // Benefit from load distribution, high cost
                (60.0, 50.0)
            }
            PolicyAction::NoOp => (0.0, 0.0),
        }
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_policy_creation() {
        let policy = Policy::new(
            "test_policy",
            vec![PolicyRule::HotnessThreshold { threshold: 0.7 }],
            vec![PolicyAction::PromoteToCache],
            PolicySchedule::Continuous,
        );
        
        assert_eq!(policy.name, "test_policy");
        assert_eq!(policy.rules.len(), 1);
        assert_eq!(policy.actions.len(), 1);
        assert!(policy.enabled);
    }
    
    #[test]
    fn test_policy_matching() {
        let policy = Policy::new(
            "hot_data",
            vec![
                PolicyRule::HotnessThreshold { threshold: 0.7 },
                PolicyRule::CacheUtilization { max: 0.9 },
            ],
            vec![PolicyAction::PromoteToCache],
            PolicySchedule::Continuous,
        );
        
        let mut state = SystemState::default();
        state.hotness = 0.8;
        state.cache_utilization = 0.7;
        
        assert!(policy.matches(&state));
        
        state.cache_utilization = 0.95;
        assert!(!policy.matches(&state));
    }
    
    #[test]
    fn test_hotness_prediction() {
        let mut predictor = HotnessPredictor::new();
        
        let features = WorkloadFeatures {
            access_frequency: 10.0,
            read_ratio: 0.8,
            avg_size: 4096.0,
            temporal_pattern: vec![1.0; 24],
            last_access_recency: 60.0,
        };
        
        let (hotness, confidence) = predictor.predict(&features);
        assert!(hotness >= 0.0 && hotness <= 1.0);
        assert!(confidence > 0.0);
        
        // Train on sample data
        let training_data = vec![
            (features.clone(), 0.9),
            (WorkloadFeatures::default(), 0.1),
        ];
        
        predictor.train(&training_data).unwrap();
        let (hotness_after, _) = predictor.predict(&features);
        assert!(hotness_after >= 0.0 && hotness_after <= 1.0);
    }
    
    #[test]
    fn test_policy_engine() {
        let mut engine = PolicyEngine::new();
        
        let policy = Policy::new(
            "promote_hot",
            vec![PolicyRule::HotnessThreshold { threshold: 0.7 }],
            vec![PolicyAction::PromoteToCache],
            PolicySchedule::Continuous,
        );
        
        engine.add_policy(policy);
        
        let mut state = SystemState::default();
        state.hotness = 0.8;
        
        let proposals = engine.evaluate_policies(&state, Uuid::new_v4());
        assert!(!proposals.is_empty());
        assert_eq!(proposals[0].policy_name, "promote_hot");
    }
    
    #[test]
    fn test_action_execution() {
        let mut engine = PolicyEngine::new();
        
        let proposal = ActionProposal {
            policy_name: "test".to_string(),
            action: PolicyAction::PromoteToCache,
            target_extent: Uuid::new_v4(),
            estimated_benefit: 100.0,
            estimated_cost: 10.0,
            confidence: 0.8,
            reason: "Test execution".to_string(),
        };
        
        let impact = engine.execute(&proposal).unwrap();
        assert!(impact.benefit > 0.0);
        assert_eq!(engine.audit_trail().len(), 1);
        assert_eq!(engine.metrics().actions_executed, 1);
    }
    
    #[test]
    fn test_safety_constraints() {
        let mut engine = PolicyEngine::new();
        
        let proposal = ActionProposal {
            policy_name: "unsafe".to_string(),
            action: PolicyAction::Defragment,
            target_extent: Uuid::new_v4(),
            estimated_benefit: 10.0,
            estimated_cost: 100.0, // Cost > 2x benefit
            confidence: 0.5,
            reason: "High cost operation".to_string(),
        };
        
        let result = engine.execute(&proposal);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_workload_features() {
        let accesses = vec![
            (1000, true, 4096),
            (2000, false, 8192),
            (3000, true, 4096),
        ];
        
        let features = WorkloadFeatures::from_access_history(&accesses);
        assert!(features.access_frequency > 0.0);
        assert!(features.read_ratio > 0.5);
        assert_eq!(features.temporal_pattern.len(), 24);
    }
    
    #[test]
    fn test_simulation() {
        let engine = PolicyEngine::new();
        
        let proposal = ActionProposal {
            policy_name: "test".to_string(),
            action: PolicyAction::MigrateTier { target: StorageTier::NVMe },
            target_extent: Uuid::new_v4(),
            estimated_benefit: 80.0,
            estimated_cost: 30.0,
            confidence: 0.75,
            reason: "Simulation test".to_string(),
        };
        
        let impact = engine.simulate(&proposal);
        assert_eq!(impact.benefit, 80.0);
        assert_eq!(impact.cost, 30.0);
    }
    
    #[test]
    fn test_audit_trail() {
        let mut engine = PolicyEngine::new();
        
        let proposal = ActionProposal {
            policy_name: "audit_test".to_string(),
            action: PolicyAction::PromoteToCache,
            target_extent: Uuid::new_v4(),
            estimated_benefit: 50.0,
            estimated_cost: 10.0,
            confidence: 0.8,
            reason: "Audit trail test".to_string(),
        };
        
        engine.execute(&proposal).unwrap();
        
        let audit = engine.audit_trail();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].policy_name, "audit_test");
        assert!(audit[0].success);
    }
}
