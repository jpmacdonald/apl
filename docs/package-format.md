# Package Format

Packages are TOML files that tell APL how to discover and download releases.

## Example

```toml
[package]
name = "ripgrep"
description = "Fast grep"
homepage = "https://github.com/BurntSushi/ripgrep"
license = "MIT"

[discovery]
source = "github"
owner = "BurntSushi"
repo = "ripgrep"
tag_pattern = "{{version}}"

[assets.select]
arm64-macos = { contains = "aarch64-apple-darwin" }
x86_64-macos = { contains = "x86_64-apple-darwin" }

[checksums]
url_template = "https://github.com/BurntSushi/ripgrep/releases/download/{{version}}/ripgrep-{{version}}-{{target}}.tar.gz.sha256"

[install]
bin = ["rg"]
```

## Sections

### `[package]`

| Field | Description |
|-------|-------------|
| `name` | Package identifier (lowercase, no spaces) |
| `description` | Short summary |
| `homepage` | Project URL |
| `license` | SPDX license identifier |
| `tags` | Optional list of tags for search |

### `[discovery]`

Tells APL where to find versions.

```toml
[discovery]
source = "github"
owner = "BurntSushi"
repo = "ripgrep"
tag_pattern = "{{version}}"     # matches tags like 14.1.0
```

For ports (prebuilt by APL):
```toml
[discovery]
ports = "python"
```

### `[assets]`

Maps architectures to release assets.

```toml
[assets.select]
arm64-macos = { contains = "aarch64-apple-darwin" }
x86_64-macos = { contains = "x86_64-apple-darwin" }
```

Selectors:
- `contains = "string"` - asset filename contains string
- `suffix = ".tar.gz"` - asset filename ends with string
- `regex = "pattern"` - asset filename matches regex

### `[checksums]`

Where to find SHA-256 checksums (avoids downloading full binaries during indexing).

```toml
[checksums]
url_template = "https://example.com/releases/{{version}}/SHA256SUMS"
```

If omitted, APL downloads the binary and computes the hash.

Set `skip_checksums = true` in `[assets]` to skip verification entirely (not recommended).

### `[install]`

What to symlink after extraction.

```toml
[install]
bin = ["rg"]                    # symlink rg to ~/.apl/bin/rg
bin = ["bin/rg:rg"]             # symlink bin/rg to ~/.apl/bin/rg
```

For GUI apps:
```toml
[install]
strategy = "app"
app = "Firefox.app"
```

### `[dependencies]`

Runtime and build dependencies.

```toml
[dependencies]
runtime = ["openssl"]
build = ["cmake", "ninja"]
```

## File location

Packages are stored in `packages/` with two-letter sharding:

```
packages/ri/ripgrep.toml
packages/fd/fd.toml
packages/jq/jq.toml
```

## Validation

Check your package before submitting:

```bash
cargo run -p apl-pkg -- check
```
