# APL Maintainer's Guide

This guide provides instructions for adding and maintaining packages in the APL registry using the `apl-pkg` utility.

## Prerequisites

To maintain the registry efficiently, set the `GITHUB_TOKEN` environment variable. Authenticated requests benefit from a significantly higher rate limit (5,000 requests per hour compared to 60 for unauthenticated requests).

```bash
export GITHUB_TOKEN=your_token_here
```

## Adding a New Package

Use the `add` command with the GitHub repository path in `owner/repo` format.

```bash
cargo run --release --bin apl-pkg -- add owner/repo
```

The `add` command performs the following steps:
1. Fetches the latest release metadata from the GitHub API.
2. Identifies the most compatible macOS ARM64 asset (supporting 15+ naming conventions and raw binaries).
3. Downloads the asset and calculates its BLAKE3 hash.
4. Generates a standard package definition in `packages/<repo>.toml`.

## Updating the Registry

To check for newer versions of all tracked packages and regenerate the index in a single operation:

```bash
cargo run --release --bin apl-pkg -- update
```

To update a specific package:

```bash
cargo run --release --bin apl-pkg -- update --package <name>
```

## Validating the Registry

Use the `check` command to lint the registry for integrity. This ensures that all package definitions have valid versions and required fields.

```bash
cargo run --release --bin apl-pkg -- check
```

## Regenerating the Index

If package definitions are modified manually, the binary index must be regenerated:

```bash
cargo run --release --bin apl-pkg -- index
```
