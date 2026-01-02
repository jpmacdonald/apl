#!/bin/bash
# Discover checksum patterns from GitHub releases
# Outputs CSV: Package,Repo,Pattern,ChecksumAsset,SuggestedTemplate

set -e
REGISTRY_DIR="${1:-registry}"

echo "Package,Repo,Pattern,Asset,Template"

find "$REGISTRY_DIR" -name "*.toml" | sort | while read -r toml_file; do
    pkg_name=$(basename "$toml_file" .toml)
    
    # Extract github repo
    github_repo=$(grep -E '^github\s*=' "$toml_file" 2>/dev/null | head -1 | sed 's/.*"\([^"]*\)".*/\1/')
    
    if [ -z "$github_repo" ]; then
        echo "$pkg_name,,NO_GITHUB,,"
        continue
    fi
    
    # Fetch latest release assets
    release_json=$(curl -s "https://api.github.com/repos/$github_repo/releases/latest" 2>/dev/null || echo "{}")
    
    # Check for error
    if echo "$release_json" | grep -q '"message"'; then
        echo "$pkg_name,$github_repo,API_ERROR,,"
        sleep 0.5
        continue
    fi
    
    tag=$(echo "$release_json" | grep -oE '"tag_name":\s*"[^"]*"' | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || true)
    
    # Pattern 1: Single checksum file (checksums.txt, SHA256SUMS, etc)
    single_file=$(echo "$release_json" | grep -oE '"name":\s*"(checksums\.txt|SHA256SUMS|sha256sums\.txt|SHASUMS256\.txt)"' | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || true)
    
    if [ -n "$single_file" ]; then
        template="https://github.com/$github_repo/releases/download/v{{version}}/$single_file"
        echo "$pkg_name,$github_repo,SINGLE_FILE,$single_file,$template"
        sleep 0.3
        continue
    fi
    
    # Pattern 1b: Versioned single file (fzf_0.67.0_checksums.txt)
    versioned_file=$(echo "$release_json" | grep -oE '"name":\s*"[^"]*checksums\.txt"' | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || true)
    
    if [ -n "$versioned_file" ]; then
        # Replace version in filename with template variable
        template_file=$(echo "$versioned_file" | sed "s/${tag#v}/{{version}}/g")
        template="https://github.com/$github_repo/releases/download/v{{version}}/$template_file"
        echo "$pkg_name,$github_repo,VERSIONED_FILE,$versioned_file,$template"
        sleep 0.3
        continue
    fi
    
    # Pattern 2: Per-file checksums (.sha256)
    per_file=$(echo "$release_json" | grep -oE '"name":\s*"[^"]*darwin[^"]*\.sha256"' | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || true)
    
    if [ -n "$per_file" ]; then
        echo "$pkg_name,$github_repo,PER_FILE,$per_file,(needs manual template)"
        sleep 0.3
        continue
    fi
    
    # No checksums found
    echo "$pkg_name,$github_repo,NONE,,"
    sleep 0.3
done
