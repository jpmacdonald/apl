#!/bin/bash
# Complete checksum discovery for all packages
# Usage: GITHUB_TOKEN=xxx ./scripts/complete_checksum_discovery.sh

set -e
REGISTRY_DIR="${1:-registry}"

# Packages that had API_ERROR or NONE in previous run
PACKAGES=(
"BurntSushi/ripgrep" "ClementTsang/bottom" "GoogleContainerTools/skaffold"
"Homebrew/brew" "Kitware/CMake" "Wilfred/difftastic" "abiosoft/colima"
"ajeetdsouza/zoxide" "alacritty/alacritty" "antonmedv/fx" "aquasecurity/trivy"
"asdf-vm/asdf" "bootandy/dust" "charmbracelet/mods" "charmbracelet/soft-serve"
"charmbracelet/vhs" "chmln/sd" "containers/podman" "crate-ci/typos"
"dalance/amber" "dalance/procs" "dandavison/delta" "denoland/deno"
"derailed/k9s" "ducaale/xh" "eza-community/eza" "fastfetch-cli/fastfetch"
"git-lfs/git-lfs" "gitui-org/gitui" "gokcehan/lf" "google/osv-scanner"
"gruntwork-io/terragrunt" "hashicorp/terraform" "helix-editor/helix"
"homeport/termshot" "imsnif/bandwhich" "jqlang/jq" "kovidgoyal/kitty"
"logseq/logseq" "lsd-rs/lsd" "maaslalani/slides" "marp-team/marp-cli"
"mikefarah/yq" "mozilla/sccache" "mpv-player/mpv" "ms-jpq/sad"
"neovim/neovim" "ninja-build/ninja" "open-policy-agent/opa" "opentofu/opentofu"
"orf/gping" "pemistahl/grex" "pulumi/pulumi" "rclone/rclone"
"sachaos/viddy" "sharkdp/bat" "sharkdp/diskus" "sharkdp/fd"
"sharkdp/hexyl" "sharkdp/hyperfine" "sharkdp/pastel" "slsa-framework/slsa-verifier"
"starship/starship" "syncthing/syncthing" "tealdeer-rs/tealdeer"
"transmission/transmission" "trufflesecurity/trufflehog" "volta-cli/volta"
"watchexec/watchexec" "wezterm/wezterm" "zed-industries/zed" "zyedidia/micro"
"golang-migrate/migrate" "hadolint/hadolint" "ogham/exa"
)

echo "Package,Repo,Pattern,Asset,Template"

for github_repo in "${PACKAGES[@]}"; do
    pkg_name=$(echo "$github_repo" | cut -d'/' -f2)
    
    # Fetch latest release assets with auth
    release_json=$(curl -s -H "Authorization: token $GITHUB_TOKEN" \
        "https://api.github.com/repos/$github_repo/releases/latest" 2>/dev/null || echo "{}")
    
    # Check for error
    if echo "$release_json" | grep -q '"message"'; then
        msg=$(echo "$release_json" | grep -oE '"message":\s*"[^"]*"' | head -1)
        echo "$pkg_name,$github_repo,ERROR,$msg,"
        sleep 0.2
        continue
    fi
    
    tag=$(echo "$release_json" | grep -oE '"tag_name":\s*"[^"]*"' | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || true)
    
    # Pattern 1: Single checksum file
    single_file=$(echo "$release_json" | grep -oE '"name":\s*"(checksums\.txt|SHA256SUMS|sha256sums\.txt|SHASUMS256\.txt)"' | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || true)
    
    if [ -n "$single_file" ]; then
        template="https://github.com/$github_repo/releases/download/{{tag}}/$single_file"
        template=$(echo "$template" | sed "s/$tag/v{{version}}/g")
        echo "$pkg_name,$github_repo,SINGLE_FILE,$single_file,$template"
        sleep 0.2
        continue
    fi
    
    # Pattern 1b: Versioned single file
    versioned_file=$(echo "$release_json" | grep -oE '"name":\s*"[^"]*checksums\.txt"' | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || true)
    
    if [ -n "$versioned_file" ]; then
        ver=$(echo "$tag" | sed 's/^v//')
        template_file=$(echo "$versioned_file" | sed "s/$ver/{{version}}/g")
        template="https://github.com/$github_repo/releases/download/v{{version}}/$template_file"
        echo "$pkg_name,$github_repo,VERSIONED_FILE,$versioned_file,$template"
        sleep 0.2
        continue
    fi
    
    # Pattern 2: Per-file checksums (.sha256)
    per_file=$(echo "$release_json" | grep -oE '"name":\s*"[^"]*darwin[^"]*\.sha256"' | head -1 | sed 's/.*"\([^"]*\)".*/\1/' || true)
    
    if [ -n "$per_file" ]; then
        echo "$pkg_name,$github_repo,PER_FILE,$per_file,(manual template needed)"
        sleep 0.2
        continue
    fi
    
    # No checksums found
    echo "$pkg_name,$github_repo,NONE,,"
    sleep 0.2
done
