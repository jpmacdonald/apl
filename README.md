# APL

A fast, minimal package manager for macOS.

## Installation

```bash
curl -fsSL https://<some domain i havent setup yet>/install.sh | sh
```

Or build from source:

```bash
git clone https://github.com/jpmacdonald/apl.git
cd apl
cargo build --release
cp target/release/apl ~/.local/bin/
```

After installing, add `~/.apl/bin` to your PATH:

```bash
export PATH="$HOME/.apl/bin:$PATH"
```

## Quick Start

```bash
# Update the package index
apl update

# Install packages
apl install ripgrep fd bat

# List installed packages
apl list

# Upgrade all packages
apl upgrade
```

## Commands

| Command | Description |
|---------|-------------|
| `apl install <pkg>` | Install packages |
| `apl remove <pkg>` | Remove packages |
| `apl list` | List installed packages |
| `apl search <query>` | Search for packages |
| `apl info <pkg>` | Show package details |
| `apl update` | Update package index |
| `apl upgrade` | Upgrade installed packages |
| `apl status` | Check for updates |

See the [User Guide](docs/user-guide.md) for complete command reference.

## Documentation

- [Getting Started](docs/getting-started.md) - Installation and first steps
- [User Guide](docs/user-guide.md) - Complete command reference
- [Package Format](docs/package-format.md) - Create your own packages
- [Architecture](docs/architecture.md) - Technical overview
- [Contributing](docs/contributing.md) - How to contribute

## Features

- **Fast** - Sub-second installs with streaming downloads
- **Simple** - TOML packages, no DSLs
- **Secure** - BLAKE3 hash verification
- **Portable** - Automatic binary relinking for macOS
- **Version control** - Multiple versions, history, rollback

## Requirements

- macOS 14.0 or later
- Apple Silicon (native) or Intel (Rosetta 2)

## License

MIT
