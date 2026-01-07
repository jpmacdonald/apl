# Package Format

APL uses **Algorithmic Templates** to discover release assets dynamically.

## Example: `ripgrep.toml`

```toml
[package]
name = "ripgrep"
version = "0.0.0" # Version for template compatibility
description = "Recursively searches directories for a regex pattern"
type = "cli"

[discovery]
github = "BurntSushi/ripgrep"
tag_pattern = "{{version}}" # Discovery pattern for tags

[assets]
# GitHub is currently the only supported binary source


[assets.targets]
arm64 = "aarch64-apple-darwin"
x86_64 = "x86_64-apple-darwin"

[checksums]
# Construct checksum URL from template
url_template = "https://github.com/BurntSushi/ripgrep/releases/download/{{version}}/ripgrep-{{version}}-{{target}}.tar.gz.sha256"
vendor_type = "sha256"

[install]
bin = ["rg"]
```

---

## `[package]` Section

Metadata about the package.

| Field | Description |
|-------|-------------|
| `name` | Unique package identifier |
| `version` | Placeholder version (usually 0.0.0 for templates) |
| `description` | Short summary |
| `type` | `cli` or `app` |

---

## `[discovery]` Section

Tells APL how to find new versions.

```toml
[discovery]
github = "owner/repo"
tag_pattern = "v{{version}}"  # Matches tags like v1.2.3
semver_only = true             # Only accept valid semver tags
include_prereleases = false    # Hide beta/rc versions by default
```

### Discovery Types
APL currently supports **GitHub Releases** for binary distribution. Manual version listing is supported only for packages built from source.

---

## `[assets]` Section

Defines how to construct download URLs for different architectures.

| Field | Description |
|-------|-------------|
| `universal` | If true, ignore `targets` and use same URL for all (requires `universal-macos` selector) |
| `select` | Map of architecture to asset selector (suffix/regex) |

### `[assets.targets]` Mapping
Maps APL architectures to vendor-specific strings used in URLs.

```toml
[assets.targets]
arm64 = "aarch64-apple-darwin"
x86_64 = "x86_64-apple-darwin"
```

---

## `[checksums]` Section

APL prioritizes vendor checksums to avoid downloading full binaries during index generation.

```toml
[checksums]
url_template = "..."   # URL to the .sha256 or .txt checksum file
vendor_type = "sha256" # Algorithms: sha256, blake3, sha512
skip = false           # If true, don't verify checksums
```

> [!TIP]
> If `url_template` is omitted or the file is missing, APL automatically falls back to downloading the binary and computing a BLAKE3 hash for the index.

---

### `[source]` (Optional)

Defines how to fetch the source code for building.

| Field | Type | Description |
| :--- | :--- | :--- |
| `url` | string | URL template for source code (e.g. `{{github}}/archive/{{tag}}.tar.gz`) |
| `format` | string | Archive format (`tar.gz`, `zip`, etc.) |
| `sha256` | string | (Optional) SHA256 of the source archive |

### `[build]` (Optional)

Defines build instructions. Presence of this section triggers **Registry Hydration**.

| Field | Type | Description |
| :--- | :--- | :--- |
| `dependencies` | array | Build-time dependencies (e.g. `cmake`, `rust`) |
| `script` | string | Multi-line shell script to run in the `Sysroot`. |

**Zero Fallback Note**: If a `[build]` section is present, the registry will attempt to build and hydrate the package into the artifact store. If hydration fails, the version is skipped. Clients never build from source; they only consume the hydrated binaries.

---

## `[install]` Section

Instructions for linking the package into your system.

```toml
[install]
strategy = "link"          # link (cli), app (applications)
bin = ["rg", "bin/fzf:fzf"] # symlinks: [source] or [source:target]
app = "Firefox.app"        # name for .app bundles
```

---



## Location & Sharding

Templates must be stored in the `registry/` directory using two-letter sharding:

`registry/ri/ripgrep.toml`
`registry/ba/bat.toml`
`registry/1/1password.toml` (numbers use `1/` prefix)
