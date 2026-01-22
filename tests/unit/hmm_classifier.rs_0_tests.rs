// moved from src/hmm_classifier.rs
use super::*;
    
    #[test]
    fn test_hmm_classifier_new() {
        let classifier = HmmClassifier::new();
        assert_eq!(classifier.state_history.len(), 0);
    }
    
    #[test]
    fn test_frequency_to_observation() {
        let classifier = HmmClassifier::new();
        
        assert_eq!(classifier.frequency_to_observation(100.0), FrequencyObservation::VeryHigh);
        assert_eq!(classifier.frequency_to_observation(25.0), FrequencyObservation::High);
        assert_eq!(classifier.frequency_to_observation(5.0), FrequencyObservation::Medium);
        assert_eq!(classifier.frequency_to_observation(0.5), FrequencyObservation::Low);
    }
    
    #[test]
    fn test_hmm_classify_hot() {
        let mut classifier = HmmClassifier::new();
        
        // Start cold, classify with high frequency and recent access
        let classification = classifier.classify(120.0, 0, AccessClassification::Cold);
        
        // Should transition to Hot due to high frequency and recent access
        assert_eq!(classification, AccessClassification::Hot);
    }
    
    #[test]
    fn test_hmm_classify_warm_to_cold() {
        let mut classifier = HmmClassifier::new();
        
        // Start warm, classify with low frequency and old access
        let classification = classifier.classify(0.5, 72, AccessClassification::Warm);
        
        // Should transition to Cold due to low frequency
        assert_eq!(classification, AccessClassification::Cold);
    }
    
    #[test]
    fn test_state_history() {
        let mut classifier = HmmClassifier::new();
        
        classifier.classify(120.0, 0, AccessClassification::Cold);
        classifier.classify(110.0, 1, AccessClassification::Hot);
        classifier.classify(5.0, 12, AccessClassification::Hot);
        
        assert_eq!(classifier.state_history.len(), 3);
        assert_eq!(classifier.state_history[0].1, AccessClassification::Hot);
        assert_eq!(classifier.state_history[2].1, AccessClassification::Warm);
    }
    
    #[test]
    fn test_smoothed_classification() {
        let mut classifier = HmmClassifier::new();
        
        // Add multiple hot classifications
        for _ in 0..5 {
            classifier.classify(120.0, 0, AccessClassification::Hot);
        }
        
        let smoothed = classifier.get_smoothed_classification(3);
        assert_eq!(smoothed, Some(AccessClassification::Hot));
    }
    
    #[test]
    fn test_viterbi_sequence() {
        let classifier = HmmClassifier::new();
        
        let observations = vec![
            FrequencyObservation::VeryHigh,
            FrequencyObservation::High,
            FrequencyObservation::Medium,
            FrequencyObservation::Low,
        ];
        
        let path = classifier.viterbi_sequence(&observations);
        
        assert_eq!(path.len(), 4);
        // Should start with hot and move towards cold
        assert_eq!(path[0], AccessClassification::Hot);
        assert_eq!(path[3], AccessClassification::Cold);
    }
