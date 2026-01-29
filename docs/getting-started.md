# Getting Started

## Install

```bash
curl -fsSL https://apl.pub/install | sh
```

This downloads the `apl` binary and adds `~/.apl/bin` to your PATH.

### Build from source

```bash
git clone https://github.com/jpmacdonald/apl.git
cd apl
cargo build --release
cp target/release/apl ~/.local/bin/
```

## Setup

Add `~/.apl/bin` to your PATH if the installer didn't do it automatically.

**Zsh** (`~/.zshrc`):
```bash
export PATH="$HOME/.apl/bin:$PATH"
```

**Bash** (`~/.bashrc`):
```bash
export PATH="$HOME/.apl/bin:$PATH"
```

**Fish** (`~/.config/fish/config.fish`):
```fish
fish_add_path ~/.apl/bin
```

## First use

```bash
apl update           # fetch package index
apl install ripgrep  # install a package
rg --version         # verify it works
```

## Shell completions

**Zsh:**
```bash
apl completions zsh > ~/.zfunc/_apl
```

**Bash:**
```bash
apl completions bash > /etc/bash_completion.d/apl
```

**Fish:**
```bash
apl completions fish > ~/.config/fish/completions/apl.fish
```

## Directory layout

APL stores everything in `~/.apl/`:

```
~/.apl/
├── bin/       symlinks to installed binaries
├── store/     installed packages (versioned)
├── cache/     downloaded archives
├── index      package index (binary format)
└── state.db   SQLite database
```

## Next steps

- [User Guide](user-guide.md) - all commands
- [Package Format](package-format.md) - add packages to the registry
