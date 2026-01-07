use anyhow::Result;
use std::io::Write;
use zstd::dict::DecoderDictionary;
use zstd::dict::EncoderDictionary;

/// Generate a binary delta using zstd dictionary compression.
/// The `old_data` serves as the dictionary for compressing `new_data`.
pub fn generate_delta(old_data: &[u8], new_data: &[u8], compression_level: i32) -> Result<Vec<u8>> {
    let dict = EncoderDictionary::copy(old_data, compression_level);
    let mut encoder = zstd::stream::Encoder::with_prepared_dictionary(Vec::new(), &dict)?;
    encoder.write_all(new_data)?;
    let compressed = encoder.finish()?;
    Ok(compressed)
}

/// Apply a binary delta using zstd dictionary decompression.
/// The `old_data` is the same dictionary used during generation.
pub fn apply_delta(old_data: &[u8], patch_data: &[u8]) -> Result<Vec<u8>> {
    let dict = DecoderDictionary::copy(old_data);
    let mut decoder = zstd::stream::Decoder::with_prepared_dictionary(patch_data, &dict)?;
    let mut decompressed = Vec::new();
    std::io::copy(&mut decoder, &mut decompressed)?;
    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_roundtrip() {
        let old = b"hello world this is a test string for binary deltas";
        let new =
            b"hello world this is a test string for binary deltas with some new content at the end";

        let delta = generate_delta(old, new, 3).unwrap();
        assert!(delta.len() < new.len()); // Should be compressed

        let reconstructed = apply_delta(old, &delta).unwrap();
        assert_eq!(reconstructed, new);
    }
}
