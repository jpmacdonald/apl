# apl - A Package Layer

> **A fast binary package manager for macOS.**  
> Reproducible. Secure. Blazingly fast.

`apl` is a modern package manager written in Rust, designed for speed and reliability. It treats your CLI tools and Mac Apps as immutable artifacts.

## Features

- ⚡️ **Fast**: Parallel downloads and zstd compression.
- 🔒 **Secure**: Sandboxed extraction and "Zip Slip" protection.
- 📦 **Apps**: Installs CLI tools (`ripgrep`) and GUI Apps (`Ghostty.app`) uniformly.
- 💎 **Reproducible**: `apl.lock` pins exact versions and hashes for every install.
- ☁️ **Self-Healing**: Automated index updates via GitHub Actions.

## Installation

```bash
curl -sL https://raw.githubusercontent.com/jpmacdonald/distill/main/install.sh | sh
```

**Important**: Add the binary directory to your shell configuration (`~/.zshrc`):
```bash
export PATH="$HOME/.apl/bin:$PATH"
```

## Usage

### Basics

```bash
# Update the package index
apl update

# Install packages (generates/updates apl.lock)
apl install ripgrep bat fd

# Install a GUI App
apl install ghostty

# Remove a package
apl remove ripgrep
```

### Reproducibility (Lockfiles)

`apl` automatically maintains an `apl.lock` file in your current directory. This file pins the exact version, URL, and BLAKE3 hash of every installed package.

To install exactly what's in the lockfile (ignoring index updates):

```bash
apl install --locked
```

### Transient Execution

Run a tool once without installing it globally:

```bash
apl run jq -- '.key' file.json
```

## Architecture

- **Index**: Hosted on GitHub Pages (`gh-pages` branch), updated automatically via CI.
- **Store**: Content-addressable storage in `~/.apl/cache`. Files are deduplicated by hash.
- **State**: SQLite database at `~/.apl/state.db`.

## Contributing

Add a new formula in `formulas/<name>.toml`:

```toml
[package]
name = "my-tool"
version = "1.0.0"
description = "A great tool"
type = "cli" # or "app"

[bottle.arm64]
url = "https://example.com/tool.tar.gz"
blake3 = "..."

[install]
bin = ["tool"]
```

Push to `main`. The GitHub Action will automatically build and publish the new index.

---

MIT License
