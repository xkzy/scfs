/// Hidden Markov Model for hot/cold classification
/// 
/// Uses a 3-state HMM (Hot, Warm, Cold) with:
/// - Emission probabilities based on observed access frequency
/// - Transition probabilities that smooth state changes
/// - Viterbi decoding for most likely state sequence

use serde::{Deserialize, Serialize};

use crate::extent::AccessClassification;

/// HMM emission states based on access frequency observations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyObservation {
    VeryHigh,  // >50 ops/day
    High,      // 10-50 ops/day
    Medium,    // 1-10 ops/day
    Low,       // <1 op/day
}

/// HMM parameters and state tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HmmClassifier {
    /// Transition matrix: log probabilities for state transitions
    /// Index: [from_state][to_state]
    /// States: Hot=0, Warm=1, Cold=2
    transition_log_probs: [[f64; 3]; 3],
    
    /// Emission matrix: log probabilities for observations given state
    /// Index: [state][observation]
    /// Observations: VeryHigh=0, High=1, Medium=2, Low=3
    emission_log_probs: [[f64; 4]; 3],
    
    /// Recency penalty: recent accesses favor hot states
    recent_access_boost: f64,
    
    /// State history for smoothing transitions
    pub state_history: Vec<(i64, AccessClassification)>,
}

impl Default for HmmClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl HmmClassifier {
    /// Create a new HMM classifier with typical parameters
    pub fn new() -> Self {
        // Transition matrix (log probabilities)
        // States: Hot=0, Warm=1, Cold=2
        // High probability to stay in same state, lower probability to transition
        let transition_log_probs = [
            // From Hot: 70% stay hot, 20% warm, 10% cold
            [0.7_f64.ln(), 0.2_f64.ln(), 0.1_f64.ln()],
            // From Warm: 25% hot, 50% warm, 25% cold
            [0.25_f64.ln(), 0.5_f64.ln(), 0.25_f64.ln()],
            // From Cold: 10% hot, 20% warm, 70% cold
            [0.1_f64.ln(), 0.2_f64.ln(), 0.7_f64.ln()],
        ];
        
        // Emission matrix: probability of observing frequency given state
        // Observations: VeryHigh=0, High=1, Medium=2, Low=3
        let emission_log_probs = [
            // Hot state: likely very high or high frequency
            // [VeryHigh, High, Medium, Low]
            [0.6_f64.ln(), 0.3_f64.ln(), 0.07_f64.ln(), 0.03_f64.ln()],
            // Warm state: likely high or medium frequency
            // [VeryHigh, High, Medium, Low]
            [0.15_f64.ln(), 0.5_f64.ln(), 0.3_f64.ln(), 0.05_f64.ln()],
            // Cold state: likely medium or low frequency
            // [VeryHigh, High, Medium, Low]
            [0.02_f64.ln(), 0.05_f64.ln(), 0.3_f64.ln(), 0.63_f64.ln()],
        ];
        
        Self {
            transition_log_probs,
            emission_log_probs,
            recent_access_boost: 2.0, // 2x boost for recent accesses
            state_history: Vec::new(),
        }
    }
    
    /// Observe frequency and update classification using HMM
    pub fn classify(
        &mut self,
        frequency: f64,
        recency_hours: i64,
        current_state: AccessClassification,
    ) -> AccessClassification {
        let now = chrono::Utc::now().timestamp();
        
        // Determine observation from frequency
        let observation = self.frequency_to_observation(frequency);
        
        // Get emission probabilities for this observation
        let observation_idx = match observation {
            FrequencyObservation::VeryHigh => 0,
            FrequencyObservation::High => 1,
            FrequencyObservation::Medium => 2,
            FrequencyObservation::Low => 3,
        };
        
        // Calculate state probabilities considering:
        // 1. Emission probability (how likely this observation given each state)
        // 2. Transition probability (from current state to next state)
        // 3. Recency bonus (recent accesses favor hot)
        
        let mut state_scores = [f64::NEG_INFINITY; 3];
        
        let current_state_idx = match current_state {
            AccessClassification::Hot => 0,
            AccessClassification::Warm => 1,
            AccessClassification::Cold => 2,
        };
        
        // Score each possible next state
        for next_state in 0..3 {
            // Emission probability: how likely is this observation in this state?
            let emission_score = self.emission_log_probs[next_state][observation_idx];
            
            // Transition probability: how likely is this transition?
            let transition_score = self.transition_log_probs[current_state_idx][next_state];
            
            // Recency bonus: recent accesses boost hot state probability
            let recency_bonus = if recency_hours < 1 && next_state == 0 {
                // Very recent access + hot state
                self.recent_access_boost.ln()
            } else if recency_hours < 24 && next_state <= 1 {
                // Recent access + hot/warm state
                (self.recent_access_boost * 0.5).ln()
            } else {
                0.0
            };
            
            state_scores[next_state] = emission_score + transition_score + recency_bonus;
        }
        
        // Find most likely state
        let next_state_idx = state_scores
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx)
            .unwrap_or(current_state_idx);
        
        let next_state = match next_state_idx {
            0 => AccessClassification::Hot,
            1 => AccessClassification::Warm,
            _ => AccessClassification::Cold,
        };
        
        // Add to history for smoothing
        self.state_history.push((now, next_state));
        
        // Keep only last 100 states for smoothing
        if self.state_history.len() > 100 {
            self.state_history.remove(0);
        }
        
        next_state
    }
    
    /// Convert frequency to observation category
    fn frequency_to_observation(&self, frequency: f64) -> FrequencyObservation {
        if frequency > 50.0 {
            FrequencyObservation::VeryHigh
        } else if frequency > 10.0 {
            FrequencyObservation::High
        } else if frequency > 1.0 {
            FrequencyObservation::Medium
        } else {
            FrequencyObservation::Low
        }
    }
    
    /// Get smoothed classification from recent state history
    /// Uses majority voting over last N observations
    pub fn get_smoothed_classification(&self, window_size: usize) -> Option<AccessClassification> {
        if self.state_history.is_empty() {
            return None;
        }
        
        let window = self.state_history.iter().rev().take(window_size);
        let mut hot_count = 0;
        let mut warm_count = 0;
        let mut cold_count = 0;
        
        for (_, state) in window {
            match state {
                AccessClassification::Hot => hot_count += 1,
                AccessClassification::Warm => warm_count += 1,
                AccessClassification::Cold => cold_count += 1,
            }
        }
        
        Some(
            if hot_count >= warm_count && hot_count >= cold_count {
                AccessClassification::Hot
            } else if warm_count >= cold_count {
                AccessClassification::Warm
            } else {
                AccessClassification::Cold
            },
        )
    }
    
    /// Estimate transition probability between states
    /// Useful for understanding pattern changes
    pub fn estimate_transition_probability(
        &self,
        from: AccessClassification,
        to: AccessClassification,
    ) -> f64 {
        let from_idx = match from {
            AccessClassification::Hot => 0,
            AccessClassification::Warm => 1,
            AccessClassification::Cold => 2,
        };
        let to_idx = match to {
            AccessClassification::Hot => 0,
            AccessClassification::Warm => 1,
            AccessClassification::Cold => 2,
        };
        
        self.transition_log_probs[from_idx][to_idx].exp()
    }
    
    /// Get the most likely state sequence (Viterbi algorithm)
    /// Returns the most probable sequence of states
    pub fn viterbi_sequence(&self, observations: &[FrequencyObservation]) -> Vec<AccessClassification> {
        if observations.is_empty() {
            return vec![];
        }
        
        let n_states = 3;
        let n_obs = observations.len();
        
        // Initialize: viterbi[time][state] = log probability
        let mut viterbi = vec![vec![f64::NEG_INFINITY; n_states]; n_obs];
        let mut backpointer = vec![vec![0; n_states]; n_obs];
        
        // Initial probabilities (assume equal for first observation)
        let initial_prob = (1.0 / n_states as f64).ln();
        
        for state in 0..n_states {
            let obs_idx = match observations[0] {
                FrequencyObservation::VeryHigh => 0,
                FrequencyObservation::High => 1,
                FrequencyObservation::Medium => 2,
                FrequencyObservation::Low => 3,
            };
            viterbi[0][state] = initial_prob + self.emission_log_probs[state][obs_idx];
        }
        
        // Forward pass
        for t in 1..n_obs {
            let obs_idx = match observations[t] {
                FrequencyObservation::VeryHigh => 0,
                FrequencyObservation::High => 1,
                FrequencyObservation::Medium => 2,
                FrequencyObservation::Low => 3,
            };
            
            for curr_state in 0..n_states {
                let mut best_score = f64::NEG_INFINITY;
                let mut best_prev_state = 0;
                
                for prev_state in 0..n_states {
                    let score = viterbi[t - 1][prev_state]
                        + self.transition_log_probs[prev_state][curr_state]
                        + self.emission_log_probs[curr_state][obs_idx];
                    
                    if score > best_score {
                        best_score = score;
                        best_prev_state = prev_state;
                    }
                }
                
                viterbi[t][curr_state] = best_score;
                backpointer[t][curr_state] = best_prev_state;
            }
        }
        
        // Backtrack to find best path
        let mut path = Vec::new();
        let mut state = viterbi[n_obs - 1]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        
        path.push(state);
        
        for t in (1..n_obs).rev() {
            state = backpointer[t][state];
            path.push(state);
        }
        
        path.reverse();
        
        path.iter()
            .map(|&state_idx| match state_idx {
                0 => AccessClassification::Hot,
                1 => AccessClassification::Warm,
                _ => AccessClassification::Cold,
            })
            .collect()
    }
}

