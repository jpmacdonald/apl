//! Merkle Tree for index integrity verification.
//!
//! Provides cryptographic proof that the entire package index is untampered.

use crate::Blake3Hash;
use serde::{Deserialize, Serialize};

/// A Merkle tree for verifying package index integrity.
///
/// Leaves are BLAKE3 hashes of serialized `IndexEntry` structs.
/// The root hash can be signed and distributed for verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleTree {
    /// Root hash of the tree.
    root: Blake3Hash,
    /// Leaf hashes (one per package).
    leaves: Vec<Blake3Hash>,
}

impl MerkleTree {
    /// Build a Merkle tree from a list of leaf hashes.
    ///
    /// Each leaf should be the BLAKE3 hash of a serialized package entry.
    pub fn from_leaves(leaves: Vec<Blake3Hash>) -> Self {
        if leaves.is_empty() {
            return Self {
                root: Blake3Hash::compute(b"empty"),
                leaves: vec![],
            };
        }

        let root = Self::compute_root(&leaves);
        Self { root, leaves }
    }

    /// Get the root hash.
    pub fn root(&self) -> &Blake3Hash {
        &self.root
    }

    /// Get the number of leaves.
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    /// Check if the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    /// Verify that a leaf at a given index matches the expected hash.
    pub fn verify_leaf(&self, index: usize, expected: &Blake3Hash) -> bool {
        self.leaves.get(index) == Some(expected)
    }

    /// Compute the root hash from leaves using binary tree structure.
    fn compute_root(leaves: &[Blake3Hash]) -> Blake3Hash {
        if leaves.len() == 1 {
            return leaves[0].clone();
        }

        // Pair up leaves and hash them together
        let mut next_level: Vec<Blake3Hash> = Vec::with_capacity(leaves.len().div_ceil(2));

        for chunk in leaves.chunks(2) {
            let combined = if chunk.len() == 2 {
                format!("{}{}", chunk[0].as_str(), chunk[1].as_str())
            } else {
                // Odd leaf: duplicate it
                format!("{}{}", chunk[0].as_str(), chunk[0].as_str())
            };
            next_level.push(Blake3Hash::compute(combined.as_bytes()));
        }

        Self::compute_root(&next_level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tree() {
        let tree = MerkleTree::from_leaves(vec![]);
        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);
    }

    #[test]
    fn single_leaf() {
        let leaf = Blake3Hash::compute(b"package1");
        let tree = MerkleTree::from_leaves(vec![leaf.clone()]);
        assert_eq!(tree.len(), 1);
        assert!(tree.verify_leaf(0, &leaf));
    }

    #[test]
    fn multiple_leaves() {
        let leaves: Vec<Blake3Hash> = (0..4)
            .map(|i| Blake3Hash::compute(format!("package{i}").as_bytes()))
            .collect();

        let tree = MerkleTree::from_leaves(leaves.clone());
        assert_eq!(tree.len(), 4);

        // Verify each leaf
        for (i, leaf) in leaves.iter().enumerate() {
            assert!(tree.verify_leaf(i, leaf));
        }
    }

    #[test]
    fn root_is_deterministic() {
        let leaves: Vec<Blake3Hash> = (0..3)
            .map(|i| Blake3Hash::compute(format!("pkg{i}").as_bytes()))
            .collect();

        let tree1 = MerkleTree::from_leaves(leaves.clone());
        let tree2 = MerkleTree::from_leaves(leaves);

        assert_eq!(tree1.root(), tree2.root());
    }

    #[test]
    fn different_leaves_different_root() {
        let leaves1 = vec![Blake3Hash::compute(b"a"), Blake3Hash::compute(b"b")];
        let leaves2 = vec![Blake3Hash::compute(b"c"), Blake3Hash::compute(b"d")];

        let tree1 = MerkleTree::from_leaves(leaves1);
        let tree2 = MerkleTree::from_leaves(leaves2);

        assert_ne!(tree1.root(), tree2.root());
    }
}
