# Package Format

APL packages are defined in TOML files. This guide covers the complete package schema.

## Quick Example

A minimal binary package:

```toml
[package]
name = "ripgrep"
version = "14.1.1"
description = "Line-oriented search tool"
type = "cli"

[source]
url = "https://github.com/BurntSushi/ripgrep"

[binary.arm64]
url = "https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/ripgrep-14.1.1-aarch64-apple-darwin.tar.gz"
blake3 = "abc123..."
format = "tar.gz"

[binary.x86_64]
url = "https://github.com/BurntSushi/ripgrep/releases/download/14.1.1/ripgrep-14.1.1-x86_64-apple-darwin.tar.gz"
blake3 = "def456..."
format = "tar.gz"

[install]
bin = ["rg"]
```

---

## Package Section

Required metadata about the package.

```toml
[package]
name = "ripgrep"           # Required: Package name (must match filename)
version = "14.1.1"         # Required: Semantic version
description = "Fast grep"  # Required: Short description
type = "cli"               # Required: "cli" or "app"
homepage = "https://..."   # Optional: Project homepage
license = "MIT"            # Optional: SPDX license identifier
```

### Package Types

| Type | Description | Install Location |
|------|-------------|------------------|
| `cli` | Command-line tool | `~/.apl/store/` with symlinks in `~/.apl/bin/` |
| `app` | macOS application | `~/Applications/` |

---

## Source Section

Where the package comes from (used for source builds and metadata).

```toml
[source]
url = "https://github.com/owner/repo"  # Repository URL
```

---

## Binary Section

Pre-compiled binaries for each architecture. **Both architectures should be provided.**

```toml
[binary.arm64]
url = "https://..."       # Direct download URL
blake3 = "hash..."        # BLAKE3 hash of the file
format = "tar.gz"         # Archive format (see below)

[binary.x86_64]
url = "https://..."
blake3 = "hash..."
format = "tar.gz"
```

### Supported Formats

| Format | Description |
|--------|-------------|
| `tar.gz` | Gzip-compressed tarball |
| `tar.zst` | Zstandard-compressed tarball |
| `zip` | ZIP archive |
| `dmg` | macOS disk image |
| `bin` | Raw binary (no archive) |

---

## Install Section

How to install the package.

```toml
[install]
strategy = "link"          # Optional: "link" (default), "app", or "pkg"
bin = ["rg"]               # Binaries to symlink to ~/.apl/bin/
app = "Firefox.app"        # For app packages: name of .app bundle
```

### Binary Mapping

Map source paths to different names:

```toml
[install]
bin = [
    "rg",                  # Simple: symlink as-is
    "bin/ripgrep:rg",      # Mapping: source:target
]
```

### Install Strategies

| Strategy | Description |
|----------|-------------|
| `link` | Extract to store, symlink binaries (default) |
| `app` | Copy .app bundle to ~/Applications |
| `pkg` | Run macOS .pkg installer |

---

## Build Section (Source Packages)

For packages that build from source:

```toml
[build]
dependencies = ["cmake", "ninja"]  # Build-time dependencies
script = """
mkdir build && cd build
cmake .. -DCMAKE_INSTALL_PREFIX=$APL_PREFIX
make -j$(nproc)
make install
"""
```

### Build Environment

These environment variables are available during builds:

| Variable | Description |
|----------|-------------|
| `$APL_PREFIX` | Installation prefix (where to install) |
| `$APL_SOURCE` | Source directory |
| `$APL_JOBS` | Number of parallel jobs |

---

## Dependencies Section

Runtime dependencies:

```toml
[dependencies]
runtime = ["libssl", "libcrypto"]  # Required at runtime
```

---

## Complete Example: CLI Tool

```toml
[package]
name = "fd"
version = "10.2.0"
description = "Simple, fast alternative to find"
type = "cli"
homepage = "https://github.com/sharkdp/fd"
license = "MIT"

[source]
url = "https://github.com/sharkdp/fd"

[binary.arm64]
url = "https://github.com/sharkdp/fd/releases/download/v10.2.0/fd-v10.2.0-aarch64-apple-darwin.tar.gz"
blake3 = "a1b2c3d4..."
format = "tar.gz"

[binary.x86_64]
url = "https://github.com/sharkdp/fd/releases/download/v10.2.0/fd-v10.2.0-x86_64-apple-darwin.tar.gz"
blake3 = "e5f6g7h8..."
format = "tar.gz"

[install]
bin = ["fd"]
```

## Complete Example: macOS App

```toml
[package]
name = "firefox"
version = "121.0"
description = "Mozilla Firefox web browser"
type = "app"

[source]
url = "https://www.mozilla.org/firefox"

[binary.arm64]
url = "https://download.mozilla.org/?product=firefox-121.0-SSL&os=osx&lang=en-US"
blake3 = "..."
format = "dmg"

[binary.x86_64]
url = "https://download.mozilla.org/?product=firefox-121.0-SSL&os=osx&lang=en-US"
blake3 = "..."
format = "dmg"

[install]
strategy = "app"
app = "Firefox.app"
```

---

## Adding Packages to the Registry

### Using apl-pkg (Recommended)

The `apl-pkg` tool automates package creation:

```bash
# Add a package from GitHub
cargo run --release --bin apl-pkg -- add owner/repo

# This will:
# 1. Fetch the latest release
# 2. Download binaries for both architectures
# 3. Compute BLAKE3 hashes
# 4. Generate packages/<name>.toml
```

### Manual Creation

1. Create `packages/<name>.toml`
2. Fill in all required fields
3. Compute BLAKE3 hashes: `apl hash <file>`
4. Validate: `cargo run --bin apl-pkg -- check`
5. Regenerate index: `cargo run --bin apl-pkg -- index`

### Computing Hashes

```bash
# Use APL's built-in hash command
apl hash path/to/file.tar.gz
```

---

## Validation

Validate a package definition:

```bash
cargo run --release --bin apl-pkg -- check
```

This checks:
- Required fields are present
- Version is valid semver
- URLs are reachable
- Hashes are correct format
