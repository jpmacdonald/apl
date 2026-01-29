# Architecture

## Repositories

APL is split across three repos:

| Repo | Purpose |
|------|---------|
| `apl` | CLI binary and core libraries |
| `apl-packages` | Package registry (TOML templates) |
| `apl-ports` | Build-from-source definitions |

### How they work together

```
apl-ports         builds source packages (Python, Ruby, etc.)
    |             uploads artifacts to R2
    v
apl-packages      indexes all packages (GitHub + ports)
    |             signs index, uploads to R2
    v
apl               downloads index, installs packages
```

**apl-ports** (producer):
- Contains build scripts for packages that need compilation
- CI runs daily, builds new versions, uploads to `apl.pub/ports/<name>/`
- Output: tar.zst archives + metadata JSON

**apl-packages** (registry):
- Contains TOML templates describing how to find packages
- CI runs on push, queries GitHub API + ports bucket
- Generates binary index, signs with Ed25519, uploads to `apl.pub/index`

**apl** (client):
- User-facing CLI
- Downloads index, resolves versions, downloads artifacts
- Never builds from source - everything is prebuilt

## Crates

```
apl-cli        CLI binary, UI, database
  └─ apl-core    resolver, downloader, builder
       └─ apl-schema  types, index format

apl-pkg        index generator (separate binary)
  └─ apl-core
       └─ apl-schema
```

| Crate | Binary | Purpose |
|-------|--------|---------|
| apl-schema | - | `PackageName`, `Arch`, `Sha256Hash`, index serialization |
| apl-core | apl-builder | resolver, discovery, download, extract, build |
| apl-cli | apl | CLI commands, UI, SQLite state |
| apl-pkg | apl-pkg | index generation, Ed25519 signing |

## Index

Binary format using Postcard + Zstd compression.

- Load time: <1ms (memory-mapped)
- Lookup: O(log n) binary search
- Size: ~2KB for 100 packages

## Install flow

```
1. Index lookup     index.find("ripgrep") -> version, url, hash
2. Download         HTTP stream -> cache file + SHA-256 verification
3. Extract          decompress -> unpack to temp dir
4. Install          move to ~/.apl/store/ripgrep/14.1.1/
5. Link             symlink bin/rg -> ~/.apl/bin/rg
6. Record           SQLite: package, version, files
```

Download and hash verification happen in parallel (no TOCTOU).

## Build flow (ports)

For packages built from source (Python, Ruby, OpenSSL):

```
1. Create sysroot   APFS clonefile (copy-on-write)
2. Mount deps       clone deps into sysroot/deps/
3. Run script       sandboxed build (no network, no /usr/local)
4. Extract output   move sysroot/usr/local -> output
5. Upload           tar.zst -> R2
```

Sandbox blocks:
- Network access
- `/usr/local`, `/opt/homebrew`
- `~/.ssh`, `~/.aws`, `~/.gnupg`

## Storage

```
~/.apl/
├── bin/           symlinks to active binaries
├── store/         installed packages (versioned)
│   └── ripgrep/
│       └── 14.1.1/
├── cache/         downloaded archives
├── logs/          build logs
├── index          package index
└── state.db       SQLite database
```

R2 bucket (`apl.pub`):
```
/index             binary package index
/index.sig         Ed25519 signature
/ports/<pkg>/      port artifacts and metadata
```

## Database

```sql
packages (name, version, sha256, active, installed_at, size_bytes)
installed_files (path, package, sha256)
history (package, action, from_version, to_version, timestamp)
```

## UI

Message-passing actor on dedicated thread. Commands send `UiEvent` via mpsc channel, actor renders to terminal.

```
Command -> Output -> mpsc -> UI Actor -> Terminal
```

## Security

| Feature | Implementation |
|---------|----------------|
| Index integrity | Ed25519 signature |
| Artifact integrity | SHA-256 |
| Transport | HTTPS |
| Verification | during download (parallel) |
| Code signing | ad-hoc re-sign after relink |

## Mach-O relinking

macOS binaries have hardcoded library paths. After extraction, APL patches them:

```
@rpath/../lib/libfoo.dylib -> @executable_path/../lib/libfoo.dylib
```

Uses `install_name_tool` + `codesign -f -s -` for ad-hoc signing.

## CI

All repos use GitHub Actions with pinned `macos-14` runners for reproducible builds.

| Workflow | Trigger | Action |
|----------|---------|--------|
| apl-ports/update-ports | daily | build new port versions, upload to R2 |
| apl-packages/update-registry | on push, dispatch | regenerate index, sign, upload |
| apl/ci | on push | test, lint |
| apl/release | on tag | build binaries, publish release |
