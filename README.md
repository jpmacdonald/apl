# APL

A package manager for macOS.

## Install

```bash
curl -fsSL https://apl.pub/install | sh
```

## Usage

```bash
apl install ripgrep       # install a package
apl install rg@14         # install specific version
apl remove ripgrep        # remove a package
apl list                  # list installed packages
apl search jq             # search for packages
apl info fd               # show package details
apl update                # refresh package index
apl upgrade               # upgrade outdated packages
```

## How it works

- Packages are prebuilt binaries downloaded from a CDN
- Index is signed with Ed25519, artifacts verified with SHA-256
- Installs are hermetic (isolated in `~/.apl/store/<pkg>/<version>/`)
- Binaries are symlinked to `~/.apl/bin/`
- Supports both ARM64 and x86_64

## Repository structure

```
apl/           CLI and core libraries (this repo)
apl-packages/  Package registry (TOML templates)
apl-ports/     Build-from-source definitions
```

## Crates

| Crate | Binary | Purpose |
|-------|--------|---------|
| apl-schema | - | Types, versioning, index format |
| apl-core | apl-builder | Resolver, downloader, builder |
| apl-cli | apl | CLI interface |
| apl-pkg | apl-pkg | Index generation |

## Development

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Setup git hooks (run once)
git config core.hooksPath .githooks
```

## License

MIT
