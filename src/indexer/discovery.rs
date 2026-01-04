use super::sources::traits::ReleaseInfo;
use crate::package::Package;
use crate::types::Sha256Digest;
use anyhow::Result;
use std::collections::HashMap;

/// Internal version type enum for parsing
#[derive(Debug, Clone, PartialEq)]
enum VersionType {
    SemVer,
    Sequential,
    Snapshot,
    CalVer,
}

pub async fn resolve_digest(
    client: &reqwest::Client,
    release: &ReleaseInfo,
    asset_filename: &str,
) -> Result<Sha256Digest> {
    // Priority 1: Check if the asset itself has a digest field (already validated at deserialization)
    if let Some(asset) = release.assets.iter().find(|a| a.name == asset_filename) {
        if let Some(digest) = &asset.digest {
            return Ok(digest.clone());
        }
    }

    // Look for checksum assets in the release
    for asset in &release.assets {
        let name = asset.name.to_lowercase();
        if name.contains("checksum")
            || name.contains("sha256")
            || name.contains("shasums")
            || name.ends_with(".intoto.jsonl")
        {
            let download_url = &asset.download_url;
            if !download_url.is_empty() {
                // Try to fetch and parse this checksum file
                let resp = client.get(download_url).send().await?;
                if resp.status().is_success() {
                    let text = resp.text().await?;
                    // Search for the target filename in the text
                    if let Some(hash) = scan_text_for_hash(&text, asset_filename) {
                        return Ok(Sha256Digest::new(hash)?);
                    }

                    // Specific handling for JSON/JSONL (e.g. SLSA provenance)
                    if name.ends_with(".json") || name.ends_with(".jsonl") {
                        // Look for the target filename and a 64-char hex string nearby
                        if text.contains(asset_filename) {
                            // Try to find a sha256 pattern
                            let re = regex::Regex::new(r#"[0-9a-fA-F]{64}"#)?;
                            if let Some(m) = re.find(&text) {
                                // This is a bit greedy but works for single-subject JSONs
                                return Sha256Digest::new(m.as_str().to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: Check release body
    if !release.body.is_empty() {
        if let Some(hash) = scan_text_for_hash(&release.body, asset_filename) {
            return Sha256Digest::new(hash);
        }
    }

    anyhow::bail!(
        "Digest for asset '{}' not found in release {}",
        asset_filename,
        release.tag_name
    )
}

pub fn scan_text_for_hash(text: &str, asset_filename: &str) -> Option<String> {
    let text = text.trim();

    // Case 1: The entire file is just a 64-char hex string (common for .sha256 files)
    if text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(text.to_string());
    }

    // Case 2: Standard sha256sum format or similar
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            // Check first part as hash, second (or rest) as filename
            if let Some(hash) = find_hash_in_parts(&parts, asset_filename) {
                return Some(hash);
            }

            // Check if reversed (filename hash)
            let reversed: Vec<&str> = parts.iter().rev().cloned().collect();
            if let Some(hash) = find_hash_in_parts(&reversed, asset_filename) {
                return Some(hash);
            }
        } else if parts.len() == 1 {
            // Single word line - could be just the hash
            let hash = parts[0];
            if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
                // If the file only has one word, we assume it's the hash for the requested asset
                // (e.g. filename.sha256 extension style)
                return Some(hash.to_string());
            }
        }
    }
    None
}

fn find_hash_in_parts(parts: &[&str], asset_filename: &str) -> Option<String> {
    let hash = parts[0];
    // Check for common separators like ':' at the end of parts[0]
    let hash = hash.trim_end_matches(':');

    if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        for &file_part in &parts[1..] {
            let file = file_part.trim_start_matches('*');
            if file == asset_filename
                || file.ends_with(asset_filename)
                || asset_filename.ends_with(file)
            {
                return Some(hash.to_string());
            }
        }
    }
    None
}

pub fn extract_version_from_tag(tag: &str, pattern: &str) -> String {
    if pattern == "{{version}}" {
        tag.strip_prefix('v').unwrap_or(tag).to_string()
    } else {
        tag.replace(&pattern.replace("{{version}}", ""), "")
    }
}

fn parse_version_by_type(tag: &str, v_type: &VersionType) -> Option<String> {
    match v_type {
        VersionType::SemVer => {
            // Basic valid check
            if semver::Version::parse(tag).is_ok() {
                Some(tag.to_string())
            } else {
                None
            }
        }
        VersionType::Sequential => {
            // "r40" -> "40.0.0" (extract first integer)
            let num_str: String = tag
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();

            if let Ok(major) = num_str.parse::<u64>() {
                Some(format!("{}.0.0", major))
            } else {
                None
            }
        }
        VersionType::Snapshot => {
            // "2024.01.01" -> "2024.1.1"
            // "2024-01-01" -> "2024.1.1"
            // "20240101-123456-hash" -> "20240101.123456.0"

            // Strategy: Split by common separators, take all leading parts that contain numbers.
            let parts: Vec<&str> = tag.split(|c| c == '.' || c == '-' || c == '_').collect();
            let nums: Vec<u64> = parts
                .iter()
                .map(|s| {
                    s.chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                })
                .filter(|s| !s.is_empty())
                .map(|s| s.parse::<u64>())
                .take_while(|r| r.is_ok())
                .map(|r| r.unwrap())
                .collect();

            if !nums.is_empty() {
                match nums.len() {
                    1 => Some(format!("{}.0.0", nums[0])),
                    2 => Some(format!("{}.{}.0", nums[0], nums[1])),
                    _ => Some(format!("{}.{}.{}", nums[0], nums[1], nums[2])),
                }
            } else {
                None
            }
        }
        VersionType::CalVer => {
            // CalVer: "25.07.1" -> "25.7.1", "24.04" -> "24.4.0"
            // YY.MM or YY.MM.PATCH format
            let parts: Vec<&str> = tag.split('.').collect();
            let nums: Vec<u64> = parts.iter().filter_map(|s| s.parse::<u64>().ok()).collect();

            match nums.len() {
                2 => Some(format!("{}.{}.0", nums[0], nums[1])),
                3 => Some(format!("{}.{}.{}", nums[0], nums[1], nums[2])),
                _ => None,
            }
        }
    }
}

/// Auto-detect version type and parse the tag.
/// Tries parsers in order of strictness: SemVer → CalVer → Sequential → Snapshot.
/// Returns the first successful parse result.
pub fn auto_parse_version(tag: &str) -> Option<String> {
    // Try SemVer first (strictest: X.Y.Z)
    if let Some(v) = parse_version_by_type(tag, &VersionType::SemVer) {
        return Some(v);
    }

    // Try CalVer (YY.MM or YY.MM.PATCH - must be 2 or 3 dot-separated numbers)
    if let Some(v) = parse_version_by_type(tag, &VersionType::CalVer) {
        return Some(v);
    }

    // Try Sequential (r40, build-123 - has a leading non-digit prefix)
    // Only use if the tag starts with non-digit characters
    if tag
        .chars()
        .next()
        .map(|c| !c.is_ascii_digit())
        .unwrap_or(false)
    {
        if let Some(v) = parse_version_by_type(tag, &VersionType::Sequential) {
            return Some(v);
        }
    }

    // Try Snapshot (date-based like 20240203-110809-hash)
    if let Some(v) = parse_version_by_type(tag, &VersionType::Snapshot) {
        return Some(v);
    }

    None
}

pub fn guess_github_repo(url: &str) -> Option<String> {
    if url.contains("github.com") {
        let parts: Vec<&str> = url.split('/').collect();
        if parts.len() >= 5 {
            return Some(format!("{}/{}", parts[3], parts[4]));
        }
    }
    None
}

pub fn guess_url_template(url: &str, version: &str, _repo: &str) -> String {
    url.replace(version, "{{version}}")
}

pub fn guess_targets(pkg: &Package) -> Option<HashMap<String, String>> {
    let mut targets = HashMap::new();
    for (arch, binary) in &pkg.targets {
        let arch_name = arch.as_str();
        // Deduce target string from URL
        // Search for the arch in the filename
        let filename = crate::filename_from_url(&binary.url);
        if filename.contains("aarch64") {
            targets.insert(arch_name.to_string(), "aarch64".to_string());
        } else if filename.contains("x86_64") {
            targets.insert(arch_name.to_string(), "x86_64".to_string());
        }
    }

    if targets.is_empty() {
        None
    } else {
        Some(targets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_text_for_hash() {
        let text = "
            not a hash
            a8f5f167f44f4964e6c998dee827110c976e3f55c5ec3ce2332e98c96ec7263b  test.tar.gz
            invalid_hash  test.tar.gz
        ";
        assert_eq!(
            scan_text_for_hash(text, "test.tar.gz"),
            Some("a8f5f167f44f4964e6c998dee827110c976e3f55c5ec3ce2332e98c96ec7263b".to_string())
        );

        // Test fallback name matching (ends_with)
        assert_eq!(
            scan_text_for_hash(text, "dir/test.tar.gz"),
            Some("a8f5f167f44f4964e6c998dee827110c976e3f55c5ec3ce2332e98c96ec7263b".to_string())
        );

        assert_eq!(scan_text_for_hash(text, "other.tar.gz"), None);
    }

    #[test]
    fn test_extract_version() {
        assert_eq!(extract_version_from_tag("v1.2.3", "v{{version}}"), "1.2.3");
        assert_eq!(
            extract_version_from_tag("release-1.2.3", "release-{{version}}"),
            "1.2.3"
        );
        assert_eq!(extract_version_from_tag("v1.2.3", "{{version}}"), "1.2.3"); // special strip 'v' case
        assert_eq!(extract_version_from_tag("1.2.3", "{{version}}"), "1.2.3");
    }

    #[test]
    fn test_parse_version_type_semver() {
        assert_eq!(
            parse_version_by_type("1.0.0", &VersionType::SemVer),
            Some("1.0.0".to_string())
        );
        assert_eq!(parse_version_by_type("v1.0.0", &VersionType::SemVer), None);
        assert_eq!(parse_version_by_type("invalid", &VersionType::SemVer), None);
    }

    #[test]
    fn test_parse_version_type_sequential() {
        assert_eq!(
            parse_version_by_type("r40", &VersionType::Sequential),
            Some("40.0.0".to_string())
        );
        assert_eq!(
            parse_version_by_type("build-123", &VersionType::Sequential),
            Some("123.0.0".to_string())
        );
        assert_eq!(
            parse_version_by_type("v40beta", &VersionType::Sequential),
            Some("40.0.0".to_string())
        );
    }

    #[test]
    fn test_parse_version_type_snapshot() {
        assert_eq!(
            parse_version_by_type("2024.01.01", &VersionType::Snapshot),
            Some("2024.1.1".to_string())
        );
        assert_eq!(
            parse_version_by_type("2024-01-01", &VersionType::Snapshot),
            Some("2024.1.1".to_string())
        );
        // "20240101-123456-hash" -> "20240101.123456.0"
        assert_eq!(
            parse_version_by_type("20240101-123456-abcdef", &VersionType::Snapshot),
            Some("20240101.123456.0".to_string())
        );
    }
}
