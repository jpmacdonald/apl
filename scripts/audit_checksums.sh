#!/bin/bash
# Audit checksum assets for all registry packages
# Requires: GITHUB_TOKEN environment variable

set -e

REGISTRY_DIR="${1:-registry}"
OUTPUT_FILE="${2:-checksum_audit.txt}"

echo "Auditing checksum assets in $REGISTRY_DIR..." > "$OUTPUT_FILE"
echo "" >> "$OUTPUT_FILE"

find "$REGISTRY_DIR" -name "*.toml" | while read -r toml_file; do
    # Extract github repo from discovery section
    github_repo=$(grep -E '^github\s*=' "$toml_file" 2>/dev/null | head -1 | sed 's/.*=\s*"\([^"]*\)".*/\1/')
    
    if [ -z "$github_repo" ]; then
        echo "SKIP: $toml_file (no github repo)" >> "$OUTPUT_FILE"
        continue
    fi
    
    # Check current checksum config
    has_url_template=$(grep -E 'url_template\s*=' "$toml_file" 2>/dev/null | grep -v '^#' | head -1 || true)
    skip_checksums=$(grep -E 'skip\s*=\s*true' "$toml_file" 2>/dev/null | head -1 || true)
    
    if [ -n "$has_url_template" ]; then
        # Already has a checksum template
        echo "OK: $toml_file (has url_template)" >> "$OUTPUT_FILE"
        continue
    fi
    
    if [ -n "$skip_checksums" ]; then
        echo "SKIP: $toml_file (checksums.skip = true)" >> "$OUTPUT_FILE"
        continue
    fi
    
    # Fetch latest release to check assets
    echo "Checking $github_repo..." >&2
    release_json=$(curl -s -H "Authorization: token $GITHUB_TOKEN" \
        "https://api.github.com/repos/$github_repo/releases/latest" 2>/dev/null || echo "{}")
    
    # Check for common checksum asset names
    checksum_asset=""
    for pattern in "checksums" "sha256" "SHA256" "SHASUMS" "sum"; do
        found=$(echo "$release_json" | grep -oE '"name":\s*"[^"]*'"$pattern"'[^"]*"' | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || true)
        if [ -n "$found" ]; then
            checksum_asset="$found"
            break
        fi
    done
    
    if [ -n "$checksum_asset" ]; then
        echo "NEEDS_TEMPLATE: $toml_file | repo=$github_repo | found_asset=$checksum_asset" >> "$OUTPUT_FILE"
    else
        # List all assets for manual review
        assets=$(echo "$release_json" | grep -oE '"name":\s*"[^"]*"' | head -10 | sed 's/"name":\s*"\([^"]*\)"/\1/' | tr '\n' ', ' || true)
        echo "NO_CHECKSUM: $toml_file | repo=$github_repo | assets=$assets" >> "$OUTPUT_FILE"
    fi
    
    # Rate limit protection
    sleep 0.5
done

echo ""
echo "Audit complete. Results in $OUTPUT_FILE"
cat "$OUTPUT_FILE"
