# Contributing

## Setup

Requirements:
- Rust (install via [rustup](https://rustup.rs))
- macOS 13.0+
- Git

```bash
git clone https://github.com/jpmacdonald/apl.git
cd apl
cargo build --workspace

# Setup git hooks
git config core.hooksPath .githooks
```

## Development

```bash
cargo build --workspace              # build
cargo test --workspace               # test
cargo clippy --workspace -- -D warnings  # lint
cargo fmt                            # format
```

Debug logging:
```bash
RUST_LOG=debug cargo run -p apl-cli -- install ripgrep
```

## Project structure

```
crates/
├── apl-schema/    types, index format
├── apl-core/      resolver, downloader, builder
├── apl-cli/       CLI (the `apl` binary)
└── apl-pkg/       index generator
```

## Adding a package

1. Create `packages/<first-two-letters>/<name>.toml`
2. Follow the format in [Package Format](package-format.md)
3. Validate: `cargo run -p apl-pkg -- check`
4. Submit PR

## Adding a CLI command

1. Create `crates/apl-cli/src/cmd/<command>.rs`
2. Add variant to the clap enum in `main.rs`
3. Wire up the handler
4. Add tests
5. Update docs/user-guide.md

## Code style

- `thiserror` for library errors, `anyhow` for binaries
- `tokio` for async
- Doc comments on public APIs
- No fluff in comments

## Pull requests

1. Fork and create a branch
2. Make changes with tests
3. Run `cargo fmt && cargo clippy -- -D warnings && cargo test`
4. Push and open PR

Keep PRs focused. CI must pass.

## Release process

1. Update version in `Cargo.toml`
2. Tag: `git tag v0.x.x`
3. Push: `git push --tags`
4. GitHub Actions builds and publishes
