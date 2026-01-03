#!/bin/bash
# Use gh api for authenticated requests
set -e

PACKAGES=(
"BurntSushi/ripgrep" "sharkdp/bat" "sharkdp/fd" "sharkdp/hexyl" "sharkdp/hyperfine"
"sharkdp/diskus" "sharkdp/pastel" "ClementTsang/bottom" "starship/starship"
"ajeetdsouza/zoxide" "alacritty/alacritty" "helix-editor/helix" "neovim/neovim"
"denoland/deno" "dandavison/delta" "eza-community/eza" "lsd-rs/lsd"
"bootandy/dust" "crate-ci/typos" "jqlang/jq" "mikefarah/yq"
"derailed/k9s" "aquasecurity/trivy" "hashicorp/terraform" "opentofu/opentofu"
"gruntwork-io/terragrunt" "pulumi/pulumi" "containers/podman" "rclone/rclone"
"syncthing/syncthing" "charmbracelet/mods" "charmbracelet/vhs" "charmbracelet/soft-serve"
)

echo "Package,Repo,Pattern,Asset,Template"

for repo in "${PACKAGES[@]}"; do
    pkg=$(echo "$repo" | cut -d'/' -f2)
    
    # Use gh api which handles auth
    assets=$(gh api "repos/$repo/releases/latest" --jq '.assets[].name' 2>/dev/null || echo "ERROR")
    
    if [ "$assets" = "ERROR" ]; then
        echo "$pkg,$repo,ERROR,,"
        continue
    fi
    
    # Check for single checksum files
    checksum=$(echo "$assets" | grep -iE '^(checksums\.txt|SHA256SUMS|sha256sums\.txt)$' | head -1)
    if [ -n "$checksum" ]; then
        echo "$pkg,$repo,SINGLE_FILE,$checksum,https://github.com/$repo/releases/download/v{{version}}/$checksum"
        continue
    fi
    
    # Check for versioned checksum files
    versioned=$(echo "$assets" | grep -i 'checksums\.txt' | head -1)
    if [ -n "$versioned" ]; then
        echo "$pkg,$repo,VERSIONED_FILE,$versioned,(needs version substitution)"
        continue
    fi
    
    # Check for per-file checksums
    perfile=$(echo "$assets" | grep -iE '\.sha256$' | head -1)
    if [ -n "$perfile" ]; then
        echo "$pkg,$repo,PER_FILE,$perfile,(per-file template needed)"
        continue
    fi
    
    echo "$pkg,$repo,NONE,,"
done
