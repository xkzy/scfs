// moved from src/redundancy.rs
use super::*;
    
    #[test]
    fn test_replication() {
        let data = b"Hello, World!";
        let policy = RedundancyPolicy::Replication { copies: 3 };
        
        let fragments = encode(data, policy).unwrap();
        assert_eq!(fragments.len(), 3);
        assert_eq!(fragments[0], data);
        assert_eq!(fragments[1], data);
        assert_eq!(fragments[2], data);
        
        // Decode with all fragments
        let options: Vec<Option<Vec<u8>>> = fragments.into_iter().map(Some).collect();
        let decoded = decode(&options, policy).unwrap();
        assert_eq!(decoded, data);
        
        // Decode with only first fragment
        let options = vec![Some(data.to_vec()), None, None];
        let decoded = decode(&options, policy).unwrap();
        assert_eq!(decoded, data);
    }
    
    #[test]
    fn test_erasure_coding() {
        let data = b"Hello, World! This is a test of erasure coding.";
        let policy = RedundancyPolicy::ErasureCoding {
            data_shards: 4,
            parity_shards: 2,
        };
        
        let fragments = encode(data, policy).unwrap();
        assert_eq!(fragments.len(), 6);
        
        // Decode with all fragments
        let options: Vec<Option<Vec<u8>>> = fragments.iter().map(|f| Some(f.clone())).collect();
        let decoded = decode(&options, policy).unwrap();
        assert_eq!(&decoded[..data.len()], data);
        
        // Decode with missing fragments (simulate 2 disk failures)
        let mut options = options;
        options[1] = None; // Missing fragment
        options[4] = None; // Missing fragment
        let decoded = decode(&options, policy).unwrap();
        assert_eq!(&decoded[..data.len()], data);
    }
