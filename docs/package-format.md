# Package Format

APL uses **Algorithmic Templates** to define packages. Instead of listing every version manually, you define a template that tells APL how to discover versions from GitHub and how to construct download URLs.

## Quick Example: `ripgrep.toml`

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
# Use {{version}} and {{target}} placeholders
url_template = "https://github.com/BurntSushi/ripgrep/releases/download/{{version}}/ripgrep-{{version}}-{{target}}.tar.gz"

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
Currently, only **GitHub Releases** discovery is fully implemented.

---

## `[assets]` Section

Defines how to construct download URLs for different architectures.

| Field | Description |
|-------|-------------|
| `url_template` | URL with `{{version}}` and `{{target}}` placeholders |
| `universal` | If true, ignore `targets` and use same URL for all |

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

## `[install]` Section

Instructions for linking the package into your system.

```toml
[install]
strategy = "link"          # link (cli), app (applications)
bin = ["rg", "bin/fzf:fzf"] # symlinks: [source] or [source:target]
app = "Firefox.app"        # name for .app bundles
```

---

## Placeholders

Templates use the following placeholders:

| Placeholder | Replaced with... |
|-------------|------------------|
| `{{version}}` | The discovered version string (e.g. `1.2.3`) |
| `{{target}}` | The target-specific string from `assets.targets` |

---

## Location & Sharding

Templates must be stored in the `registry/` directory using two-letter sharding:

`registry/ri/ripgrep.toml`
`registry/ba/bat.toml`
`registry/1/1password.toml` (numbers use `1/` prefix)
