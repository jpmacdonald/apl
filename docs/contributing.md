# Contributing to APL

Guide for contributing to APL development.

## Development Setup

### Prerequisites

- Rust 2024 edition (install via [rustup](https://rustup.rs))
- macOS 14.0 or later
- Git

### Clone and Build

```bash
git clone https://github.com/jpmacdonald/apl.git
cd apl
cargo build
```

### Run Tests

```bash
# Unit tests
cargo test

# Doc tests
cargo test --doc

# All tests
cargo test --all
```

---

## Code Style

### Formatting

Use `cargo fmt` before committing:

```bash
cargo fmt
```

### Linting

Use `cargo clippy` for lints:

```bash
cargo clippy -- -D warnings
```

### Conventions

- **Error handling**: Use `thiserror` for error types, `anyhow` for propagation
- **Async**: Use `tokio` runtime, avoid blocking in async contexts
- **Documentation**: Add doc comments to public APIs

---

## Code Coverage

APL uses `cargo-llvm-cov` for coverage tracking.

### Install

```bash
cargo install cargo-llvm-cov
```

### Generate Report

```bash
# HTML report (recommended)
cargo llvm-cov --all-features --workspace --html
open target/llvm-cov/html/index.html

# Terminal summary
cargo llvm-cov --all-features --workspace


# LCOV format (for CI)
cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info
```

### Coverage Goals

- **Overall target**: >80%
- **Core modules**: >85%
- **Critical paths** (install, remove): 100%

---

## Registry Maintenance

APL uses an **Algorithmic Registry**. Instead of manually updating version strings, you contribute **Templates** that tell APL how to discover new versions.

### Prerequisites

Set a GitHub token for higher API rate limits (required for index generation):

```bash
export GITHUB_TOKEN=your_token_here
```

### Contributing a New Package

1. Create a template in `registry/{prefix}/{name}.toml`.
   - Example: `registry/ri/ripgrep.toml`
2. Follow the [Package Format](package-format.md) guide to define:
   - `[discovery]`: GitHub repo and tag pattern.
   - `[assets]`: URL templates with `{{version}}` and `{{target}}`.
   - `[checksums]`: Vendor checksum templates.
3. Validate your template:
   ```bash
   cargo run --release --bin apl-pkg -- check
   ```
4. Generate a local index to test:
   ```bash
   cargo run --release --bin apl-pkg -- index
   ```

### Updating the Index

The index is automatically updated every 6 hours by GitHub Actions. You don't need to manually update versions unless the template logic changes.

---

## Project Structure

```
apl/
├── src/
│   ├── main.rs          # CLI entry point
│   ├── lib.rs           # Library root
│   ├── cmd/             # Command implementations
│   ├── core/            # Core logic (index, resolver, etc.)
│   ├── io/              # I/O operations (download, extract)
│   ├── ops/             # High-level operations
│   ├── store/           # Database and storage
│   ├── ui/              # Terminal UI
│   ├── registry/        # GitHub API client
│   └── bin/             # Additional binaries (apl-pkg)
├── packages/            # Package definitions
├── tests/               # Integration tests
├── docs/                # Documentation
└── .github/workflows/   # CI/CD
```

---

## Pull Request Process

1. **Fork** the repository
2. **Create a branch** for your feature: `git checkout -b feature/my-feature`
3. **Make changes** with tests
4. **Run checks**:
   ```bash
   cargo fmt
   cargo clippy -- -D warnings
   cargo test
   ```
5. **Commit** with a descriptive message
6. **Push** and open a PR

### PR Guidelines

- Keep changes focused and atomic
- Add tests for new functionality
- Update documentation if needed
- Ensure CI passes

---

## Adding New Commands

1. Create `src/cmd/<command>.rs`
2. Add the command enum variant to `src/main.rs`
3. Wire up the handler in the `match` statement
4. Add tests
5. Update the user guide

---

## Debugging

### Enable Tracing

```bash
RUST_LOG=debug cargo run -- install ripgrep
```

Levels: `error`, `warn`, `info`, `debug`, `trace`

### Build Logs

Source build logs are saved to `~/.apl/logs/`:

```bash
cat ~/.apl/logs/<package>-<version>.log
```

---

## Release Process

1. Update version in `Cargo.toml`
2. Update CHANGELOG
3. Create a git tag: `git tag v0.x.x`
4. Push: `git push --tags`
5. GitHub Actions builds and publishes releases

---

## Getting Help

- **Issues**: [GitHub Issues](https://github.com/jpmacdonald/apl/issues)
- **Discussions**: [GitHub Discussions](https://github.com/jpmacdonald/apl/discussions)
