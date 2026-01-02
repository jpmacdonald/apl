# APL

A fast, minimal package manager for macOS.

## Installation

Since APL is currently in development, you can install it by cloning the repository and running the bootstrap script:

```bash
# 1. Clone the repository
git clone https://github.com/jpmacdonald/apl.git
cd apl

# 2. Build from source
cargo build --release

# 3. Run the installer
./install.sh
```

The installer will copy the binary to `~/.apl/bin` and help you set up your PATH.

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

- **Algorithmic Registry** - Dynamic version discovery via GitHub (zero manual maintenance)
- **Fast** - Sub-second installs with streaming downloads
- **Simple** - TOML packages, no DSLs
- **Secure** - BLAKE3 hash verification with vendor checksum support
- **Portable** - Automatic binary relinking for macOS
- **Version control** - Multiple versions, history, rollback

## Requirements

- macOS 14.0 or later
- Apple Silicon (native) or Intel (Rosetta 2)

## License

MIT
