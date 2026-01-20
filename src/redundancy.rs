use anyhow::{anyhow, Context, Result};
use reed_solomon_erasure::galois_8::ReedSolomon;

use crate::extent::RedundancyPolicy;

/// Encode data according to redundancy policy
pub fn encode(data: &[u8], policy: RedundancyPolicy) -> Result<Vec<Vec<u8>>> {
    match policy {
        RedundancyPolicy::Replication { copies } => encode_replication(data, copies),
        RedundancyPolicy::ErasureCoding { data_shards, parity_shards } => {
            encode_erasure_coding(data, data_shards, parity_shards)
        }
    }
}

/// Decode data from fragments
pub fn decode(fragments: &[Option<Vec<u8>>], policy: RedundancyPolicy) -> Result<Vec<u8>> {
    match policy {
        RedundancyPolicy::Replication { .. } => decode_replication(fragments),
        RedundancyPolicy::ErasureCoding { data_shards, parity_shards } => {
            decode_erasure_coding(fragments, data_shards, parity_shards)
        }
    }
}

/// Re-encode data from old policy to new policy
/// This is used when changing redundancy policies on existing extents
pub fn reencode(
    fragments: &[Option<Vec<u8>>],
    old_policy: RedundancyPolicy,
    new_policy: RedundancyPolicy,
) -> Result<Vec<Vec<u8>>> {
    // First, decode with old policy
    let original_data = decode(fragments, old_policy)?;
    
    // Then, encode with new policy
    encode(&original_data, new_policy)
}

/// Get the number of fragments needed to reconstruct with a policy
pub fn min_fragments_for_policy(policy: RedundancyPolicy) -> usize {
    policy.min_fragments()
}

/// Get total fragment count for a policy
pub fn total_fragments_for_policy(policy: RedundancyPolicy) -> usize {
    policy.fragment_count()
}

/// Replication encoding: create N copies
fn encode_replication(data: &[u8], copies: usize) -> Result<Vec<Vec<u8>>> {
    let mut fragments = Vec::new();
    for _ in 0..copies {
        fragments.push(data.to_vec());
    }
    Ok(fragments)
}

/// Replication decoding: return first available copy
fn decode_replication(fragments: &[Option<Vec<u8>>]) -> Result<Vec<u8>> {
    for fragment in fragments {
        if let Some(data) = fragment {
            return Ok(data.clone());
        }
    }
    Err(anyhow!("No fragments available for replication decode"))
}

/// Erasure coding encoding using Reed-Solomon
fn encode_erasure_coding(
    data: &[u8],
    data_shards: usize,
    parity_shards: usize,
) -> Result<Vec<Vec<u8>>> {
    let rs = ReedSolomon::new(data_shards, parity_shards)
        .context("Failed to create Reed-Solomon encoder")?;
    
    // Calculate shard size
    let shard_size = (data.len() + data_shards - 1) / data_shards;
    
    // Create shards
    let mut shards: Vec<Vec<u8>> = Vec::new();
    
    // Split data into data shards
    for i in 0..data_shards {
        let start = i * shard_size;
        let end = std::cmp::min(start + shard_size, data.len());
        
        let mut shard = if start < data.len() {
            data[start..end].to_vec()
        } else {
            Vec::new()
        };
        
        // Pad to shard_size
        shard.resize(shard_size, 0);
        shards.push(shard);
    }
    
    // Create empty parity shards
    for _ in 0..parity_shards {
        shards.push(vec![0u8; shard_size]);
    }
    
    // Encode
    rs.encode(&mut shards)
        .context("Failed to encode with Reed-Solomon")?;
    
    Ok(shards)
}

/// Erasure coding decoding using Reed-Solomon
fn decode_erasure_coding(
    fragments: &[Option<Vec<u8>>],
    data_shards: usize,
    parity_shards: usize,
) -> Result<Vec<u8>> {
    let rs = ReedSolomon::new(data_shards, parity_shards)
        .context("Failed to create Reed-Solomon decoder")?;
    
    // Check we have enough fragments
    let available = fragments.iter().filter(|f| f.is_some()).count();
    if available < data_shards {
        return Err(anyhow!(
            "Not enough fragments for decode: {} < {}",
            available,
            data_shards
        ));
    }
    
    // Convert to format expected by reed-solomon-erasure
    let mut shards: Vec<Option<Vec<u8>>> = fragments.to_vec();
    
    // Ensure we have the right number of shards
    while shards.len() < data_shards + parity_shards {
        shards.push(None);
    }
    
    // Reconstruct
    rs.reconstruct(&mut shards)
        .context("Failed to reconstruct with Reed-Solomon")?;
    
    // Concatenate data shards
    let mut result = Vec::new();
    for i in 0..data_shards {
        if let Some(shard) = &shards[i] {
            result.extend_from_slice(shard);
        } else {
            return Err(anyhow!("Failed to reconstruct data shard {}", i));
        }
    }
    
    Ok(result)
}

#[cfg(test)]
mod tests {
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
}
