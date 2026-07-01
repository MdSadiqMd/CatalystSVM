//! Hashing and Merkle tree utilities for deterministic commitments
use sha2::{Digest, Sha256};

/// Compute SHA-256 hash of bytes, return hex string
pub fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Compute SHA-256 hash of a string, return hex string
pub fn hash_str(s: &str) -> String {
    hash_bytes(s.as_bytes())
}

/// Compute Merkle root from a list of leaf hashes (already hex-encoded)
/// Uses a simple binary tree construction with SHA-256
pub fn merkle_root(leaves: &[String]) -> String {
    if leaves.is_empty() {
        return hash_bytes(b"empty");
    }
    if leaves.len() == 1 {
        return leaves[0].clone();
    }

    let mut current_level: Vec<String> = leaves.to_vec();

    while current_level.len() > 1 {
        let mut next_level = Vec::with_capacity((current_level.len() + 1) / 2);

        for chunk in current_level.chunks(2) {
            let combined = if chunk.len() == 2 {
                format!("{}{}", chunk[0], chunk[1])
            } else {
                format!("{}{}", chunk[0], chunk[0])
            };
            next_level.push(hash_str(&combined));
        }

        current_level = next_level;
    }

    current_level.into_iter().next().unwrap_or_default()
}

/// Compute state root from a sorted map of account -> value
pub fn state_root(state: &std::collections::BTreeMap<String, u64>) -> String {
    let leaves: Vec<String> = state
        .iter()
        .map(|(k, v)| hash_str(&format!("{}:{}", k, v)))
        .collect();

    if leaves.is_empty() {
        hash_bytes(b"empty_state")
    } else {
        merkle_root(&leaves)
    }
}

/// Hash a trace entry for inclusion in trace Merkle tree
pub fn hash_trace_entry(
    tx_id: &str,
    success: bool,
    compute_used: u64,
    read_set: &[String],
    write_set: &[String],
) -> String {
    let reads = read_set.join(",");
    let writes = write_set.join(",");
    let data = format!(
        "tx:{}:success:{}:cu:{}:reads:{}:writes:{}",
        tx_id, success, compute_used, reads, writes
    );
    hash_str(&data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_hash_bytes_deterministic() {
        let h1 = hash_bytes(b"hello");
        let h2 = hash_bytes(b"hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn test_hash_str() {
        let h = hash_str("hello");
        assert_eq!(h.len(), 64);
    }

    #[test]
    fn test_merkle_root_empty() {
        let root = merkle_root(&[]);
        assert!(!root.is_empty());
        assert_eq!(root, hash_bytes(b"empty"));
    }

    #[test]
    fn test_merkle_root_single() {
        let leaf = hash_str("leaf");
        let root = merkle_root(&[leaf.clone()]);
        assert_eq!(root, leaf);
    }

    #[test]
    fn test_merkle_root_two() {
        let a = hash_str("a");
        let b = hash_str("b");
        let root = merkle_root(&[a.clone(), b.clone()]);
        let expected = hash_str(&format!("{}{}", a, b));
        assert_eq!(root, expected);
    }

    #[test]
    fn test_merkle_root_odd() {
        let a = hash_str("a");
        let b = hash_str("b");
        let c = hash_str("c");
        let root = merkle_root(&[a.clone(), b.clone(), c.clone()]);
        // ab and cc at level 1, then combine
        let ab = hash_str(&format!("{}{}", a, b));
        let cc = hash_str(&format!("{}{}", c, c));
        let expected = hash_str(&format!("{}{}", ab, cc));
        assert_eq!(root, expected);
    }

    #[test]
    fn test_state_root_deterministic() {
        let mut state = BTreeMap::new();
        state.insert("alice".to_string(), 100);
        state.insert("bob".to_string(), 200);

        let r1 = state_root(&state);
        let r2 = state_root(&state);
        assert_eq!(r1, r2);
    }

    #[test]
    fn test_state_root_order_independent() {
        let mut state1 = BTreeMap::new();
        state1.insert("alice".to_string(), 100);
        state1.insert("bob".to_string(), 200);

        let mut state2 = BTreeMap::new();
        state2.insert("bob".to_string(), 200);
        state2.insert("alice".to_string(), 100);

        assert_eq!(state_root(&state1), state_root(&state2));
    }

    #[test]
    fn test_hash_trace_entry() {
        let h1 = hash_trace_entry("tx1", true, 1000, &["a".into()], &["b".into()]);
        let h2 = hash_trace_entry("tx1", true, 1000, &["a".into()], &["b".into()]);
        assert_eq!(h1, h2);

        let h3 = hash_trace_entry("tx1", false, 1000, &["a".into()], &["b".into()]);
        assert_ne!(h1, h3);
    }
}
