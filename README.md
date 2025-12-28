# Distill (dl)

A **purely experimental** package manager for macOS CLI tools. Written in Rust.

## What It Does

- Installs single-binary CLI tools (like `jq`, `ripgrep`, `fzf`)
- Uses a content-addressable store for deduplication
- Supports transient execution with `dl run` (no global install)
- Never runs post-install scripts

## Installation

```bash
curl -sL https://raw.githubusercontent.com/jpmacdonald/distill/main/install.sh | sh
```

Then add `~/.dl/bin` to your PATH.

## Usage

```bash
# Fetch the package index
dl update

# Install tools
dl install ripgrep bat fd

# Run a tool without installing
dl run jq -- '.key' file.json

# List installed packages
dl list

# Remove a package
dl remove ripgrep

# Upgrade all packages
dl upgrade
```

## Formulas

Packages are defined in TOML files. Example:

```toml
[package]
name = "jq"
version = "1.7.1"

[bottle.arm64]
url = "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-macos-arm64"
blake3 = "..."

[install]
bin = ["jq"]
```

## Limitations

- Only supports macOS (arm64 and x86_64)
- Only handles single-binary tools
- Tools needing shell init (like `zoxide`) require manual `.zshrc` setup
- Does not support `nvm`, `pyenv`, or similar shell managers

## Architecture

- **Index**: `~/.dl/index.bin` (postcard+zstd compressed)
- **Database**: `~/.dl/state.db` (SQLite)
- **Cache**: `~/.dl/cache/` (BLAKE3-hashed blobs)
- **Binaries**: `~/.dl/bin/`

---

MIT License
