# APL: Advanced Package Layer

A modern package manager for macOS, built in Rust with a focus on hermetic installations, strict versioning, and cryptographic verification.

## Quick Install

```bash
curl -fsSL https://apl.pub/install.sh | sh
```

## Features

- **Hermetic Installations** - Packages are installed in isolation, preventing conflicts
- **Dual Architecture** - Native support for both Apple Silicon (ARM64) and Intel (x86_64)
- **Signed Index** - Ed25519 signature verification for the package registry
- **Fast Updates** - ZSTD-compressed binary index for efficient synchronization

## Workspace Structure

| Crate | Binary | Description |
|-------|--------|-------------|
| `apl-schema` | - | Core types, versioning, and index serialization |
| `apl-core` | `apl-builder` | Core library: indexer, resolver, and discovery engine |
| `apl-cli` | `apl` | User-facing CLI and state management |
| `apl-indexer` | `apl-pkg` | Index generation and registry maintenance |

## Commands

```bash
apl install <package>     # Install a package
apl remove <package>      # Remove a package  
apl search <query>        # Search available packages
apl list                  # List installed packages
apl status                # Check for updates
apl upgrade               # Upgrade outdated packages
apl update                # Refresh package index
```

## Development

```bash
# Build all crates
cargo build --workspace

# Run the CLI
cargo run -p apl-cli -- search jq

# Run tests
cargo test --workspace

# Run lints
cargo clippy --workspace --all-targets -- -D warnings
```

## Related Repositories

- **[apl-packages](../apl-packages)** - Package definitions (TOML files for GitHub-sourced packages)
- **[apl-ports](../apl-ports)** - Port definitions (for vendor-specific sources like HashiCorp, AWS, etc.)

## Architecture

See [docs/architecture.md](docs/architecture.md) for details on the crate structure and data flow.

## License

MIT
