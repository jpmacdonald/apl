# APL (apl.pub)

A fast, minimal package manager for macOS, powered by a Content-Addressable Storage (CAS) and an algorithmic registry.

## Installation

```bash
curl -fsSL https://apl.pub/install | sh
```

## Quick Start

```bash
# Update the package index from apl.pub
apl update

# Install packages
apl install ripgrep fd bat

# List installed packages
apl list
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
- **Hybrid Distribution** - Fast binary downloads from Cloudflare R2 + GitHub fallback
- **Source Hydration** - Automatically builds complex packages from source if binaries are missing
- **Secure by Default** - Ed25519 index signing and BLAKE3 artifact verification
- **Fast** - Sub-second installs with streaming downloads
- **Simple** - TOML packages, no DSLs
- **Portable** - Automatic binary relinking for macOS
- **Version control** - Multiple versions, history, rollback

## Requirements

- macOS 14.0 or later
- Apple Silicon (native) or Intel (Rosetta 2)

## License

MIT
