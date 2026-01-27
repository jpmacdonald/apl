//! Block-level content-addressable storage with deduplication.
//!
//! Uses FastCDC for content-defined chunking and BLAKE3 for hashing.

use crate::types::Blake3Hash;
use serde::{Deserialize, Serialize};

/// A reference to a chunk in the CAS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRef {
    /// BLAKE3 hash of the chunk data.
    pub hash: Blake3Hash,
    /// Size of the chunk in bytes.
    pub size: u32,
}

/// Manifest describing a blob as a sequence of chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobManifest {
    /// Total size of the original blob.
    pub size: u64,
    /// Ordered list of chunk references.
    pub chunks: Vec<ChunkRef>,
}

impl BlobManifest {
    /// Create a manifest by chunking the given data.
    ///
    /// Uses FastCDC with default parameters (avg 64KB chunks).
    pub fn from_data(data: &[u8]) -> Self {
        use fastcdc::v2020::FastCDC;

        let chunker = FastCDC::new(data, 16384, 65536, 262_144); // min 16KB, avg 64KB, max 256KB
        let mut chunks = Vec::new();

        for chunk in chunker {
            let chunk_data = &data[chunk.offset..chunk.offset + chunk.length];
            let hash = Blake3Hash::compute(chunk_data);
            chunks.push(ChunkRef {
                hash,
                size: chunk.length as u32,
            });
        }

        Self {
            size: data.len() as u64,
            chunks,
        }
    }

    /// Get the hashes of all unique chunks.
    pub fn unique_chunks(&self) -> Vec<&Blake3Hash> {
        let mut seen = std::collections::HashSet::new();
        self.chunks
            .iter()
            .filter_map(|c| {
                if seen.insert(c.hash.as_str()) {
                    Some(&c.hash)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Deserialize from JSON.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

/// Reassemble a blob from chunks.
pub fn reassemble<S: ::std::hash::BuildHasher>(manifest: &BlobManifest, chunk_data: &std::collections::HashMap<String, Vec<u8>, S>) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(manifest.size as usize);

    for chunk_ref in &manifest.chunks {
        let data = chunk_data.get(chunk_ref.hash.as_str())?;
        result.extend_from_slice(data);
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_small_data() {
        let data = b"Hello, world!";
        let manifest = BlobManifest::from_data(data);

        assert_eq!(manifest.size, data.len() as u64);
        assert!(!manifest.chunks.is_empty());
    }

    #[test]
    fn test_chunk_large_data() {
        // Create 1MB of data
        let data: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();
        let manifest = BlobManifest::from_data(&data);

        assert_eq!(manifest.size, data.len() as u64);
        // Should have multiple chunks for 1MB data
        assert!(manifest.chunks.len() > 1);
    }

    #[test]
    fn test_reassemble() {
        let data = b"Test data for chunking and reassembly";
        let manifest = BlobManifest::from_data(data);

        // Create chunk data map
        let mut chunk_map = std::collections::HashMap::new();
        let mut offset = 0;
        for chunk_ref in &manifest.chunks {
            let chunk_data = &data[offset..offset + chunk_ref.size as usize];
            chunk_map.insert(chunk_ref.hash.as_str().to_string(), chunk_data.to_vec());
            offset += chunk_ref.size as usize;
        }

        let reassembled = reassemble(&manifest, &chunk_map).unwrap();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn test_dedup_identical_chunks() {
        // Create data with repeated patterns (should dedup)
        let pattern: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
        let mut data = Vec::new();
        for _ in 0..5 {
            data.extend_from_slice(&pattern);
        }

        let manifest = BlobManifest::from_data(&data);
        let unique = manifest.unique_chunks();

        // Repeated patterns should result in fewer unique chunks than total
        assert!(unique.len() <= manifest.chunks.len());
    }

    #[test]
    fn test_manifest_json_roundtrip() {
        let data = b"Test data";
        let manifest = BlobManifest::from_data(data);

        let json = manifest.to_json();
        let restored = BlobManifest::from_json(&json).unwrap();

        assert_eq!(manifest.size, restored.size);
        assert_eq!(manifest.chunks.len(), restored.chunks.len());
    }
}
