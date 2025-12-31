# APL

A package manager for macOS.

## Installation

```bash
curl -fsSL https://apl.dev/install.sh | sh
```

Or build from source:

```bash
git clone https://github.com/user/apl.git
cd apl
cargo build --release
cp target/release/apl ~/.local/bin/
```

## Usage

### Install packages

```bash
apl install ripgrep neovim
```

### Remove packages

```bash
apl remove ripgrep
```

### List installed packages

```bash
apl list
```

### Search for packages

```bash
apl search editor
```

### View package information

```bash
apl info neovim
```

### Update the package index

```bash
apl update
```

### Upgrade installed packages

```bash
apl upgrade
```

## Registry Maintenance

## Registry Maintenance

For APL maintainers, the `apl-pkg` tool handles the package lifecycle (adding, updating, and indexing).

See the [Maintainer's Guide](docs/maintainer_guide.md) and [Registry Architecture](docs/registry_architecture.md) for more details.

```bash
# Add a new package
cargo run --release --bin apl-pkg -- add owner/repo

# Update all packages and regenerate index
cargo run --release --bin apl-pkg -- update
```

## Configuration

APL stores all data in `~/.apl/`:

```
~/.apl/
├── bin/          # Symlinks to installed binaries
├── store/        # Installed packages (versioned)
├── cache/        # Downloaded archives
├── index.bin     # Package index
└── state.db      # Installation database
```

Add `~/.apl/bin` to your PATH:

```bash
export PATH="$HOME/.apl/bin:$PATH"
```

## Shell Completions

APL supports completions for bash, zsh, fish, elvish, and powershell.

```bash
# Zsh
apl completions zsh > ~/.zfunc/_apl

# Bash
apl completions bash > /etc/bash_completion.d/apl

# Fish
apl completions fish > ~/.config/fish/completions/apl.fish
```

## Package Format

Packages are defined in TOML:

```toml
[package]
name = "ripgrep"
version = "14.1.1"
description = "Line-oriented search tool"
type = "cli"

[source]
url = "https://github.com/BurntSushi/ripgrep/releases/..."
blake3 = "abc123..."

[binary.arm64]
url = "https://github.com/BurntSushi/ripgrep/releases/..."
blake3 = "def456..."

[install]
bin = ["rg"]
```

## Requirements

- macOS 14.0 or later
- Apple Silicon (native) or Intel processor (Rosetta 2)

## License

MIT
