# APL

A fast, minimal package manager for macOS. APL uses Content-Addressable Storage and an algorithmic registry to deliver sub-second installs with high security.

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

- **Algorithmic Registry** - Discovers versions dynamically via the GitHub Tags API to eliminate manual package maintenance.
- **Hybrid Distribution** - Streamlines downloads by prioritizing Cloudflare R2, with automatic fallback to GitHub Releases.
- **Source Hydration** - Compiles releases from source automatically when pre-built binaries are unavailable for the host architecture.
- **Secure by Default** - Enforces Ed25519 index signatures and SHA-256 artifact verification on every operation.
- **Fast Installation** - Streams downloads directly to disk, avoiding intermediate temp files for sub-second performance.
- **Simplicity** - Uses standard TOML for package definitions. No complex DSLs or hidden logic.
- **Portability** - Retargets Mach-O binaries automatically to ensuring they run from any location.
- **Version Control** - Supports parallel installation of multiple versions and atomic rollback.

## Requirements

- macOS 14.0 or later
- Apple Silicon or Intel architecture

## License

MIT
