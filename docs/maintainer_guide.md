# APL Maintainer's Guide

This guide explains how to add and maintain packages in the APL registry using the `apl-pkg` tool.

## Prerequisites

To maintain the registry efficiently, you must have a `GITHUB_TOKEN` set in your environment to avoid rate limits (increases limit from 60 to 5000 requests per hour).

```bash
export GITHUB_TOKEN=your_token_here
```

## Adding a New Package

Use the `add` command with the GitHub repository path.

```bash
cargo run --release --bin apl-pkg -- add owner/repo
```

`apl-pkg` will:
1. Fetch the latest release from GitHub.
2. Search for the best macOS ARM64 asset (supporting 15+ naming patterns and raw binaries).
3. Download and calculate the BLAKE3 hash.
4. Scaffold a `packages/<repo>.toml` file.

## Updating the Registry

To check for updates for all packages and regenerate the index:

```bash
cargo run --release --bin apl-pkg -- update
```

To update a specific package only:

```bash
cargo run --release --bin apl-pkg -- update --package <name>
```

## Validating the Registry

Always run the linter before pushing changes to ensure no "broken" packages (e.g., version `0.0.0`) enter the index.

```bash
cargo run --release --bin apl-pkg -- check
```

## Regenerating the Index

If you manually edit TOML files, regenerate the binary index:

```bash
cargo run --release --bin apl-pkg -- index
```
