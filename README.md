# Distill (dl)

**Distill** is a modern, blazingly fast package manager for macOS, written in Rust. It draws inspiration from `uv`, `pacman`, and `apt`, focusing on speed, atomicity, and a great developer experience.

## Features

- 🚀 **Extreme Speed**: Sub-10ms startup.
- 📦 **CAS Storage**: Deduplicated content-addressable storage for binaries.
- 🔗 **Smart Linking**: Automatic binary linking (e.g., `ripgrep` installs as `rg`).
- 🔄 **Dependency Resolution**: Topological sorting for complex package trees.
- 🏗️ **Modern Formulae**: TOML-based package specifications.
- ✨ **Self-Updating**: Keep the tool up-to-date with a single command.
- 🛠️ **Dev Tools**: Native shell completions and formula scaffolding.

## Installation

```bash
curl -sL https://raw.githubusercontent.com/jimmy/distill/main/install.sh | sh
```

## Quick Start

```bash
# Update the index
dl update

# Search for packages
dl search ripgrep

# Install packages (parallel downloads!)
dl install ripgrep bat fd

# Upgrade all
dl upgrade

# Self-update dl
dl self-update
```

## Maintenance

```bash
# Create a new formula
dl formula new my-pkg

# Bump version and re-hash automatically
dl formula bump formulas/my-pkg.toml --version 2.0.0 --url https://...
```

## Architecture

- **State**: SQLite database at `~/.dl/state.db`.
- **Cache**: BLAKE3-hashed CAS at `~/.dl/cache/`.
- **Binaries**: Linked to `~/.dl/bin/` (add this to your PATH).
- **Index**: High-performance Msgpack registry.

---
Built with ❤️ in Rust for macOS.
