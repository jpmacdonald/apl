# Getting Started with APL

APL is a fast, minimal package manager for macOS CLI tools.

## Installation

### Quick Install (Recommended)

```bash
curl -fsSL https://apl.dev/install.sh | sh
```

### Build from Source

```bash
git clone https://github.com/jpmacdonald/apl.git
cd apl
cargo build --release
cp target/release/apl ~/.local/bin/
```

## Setup

### Add to PATH

APL installs binaries to `~/.apl/bin`. Add it to your shell profile:

**Zsh** (`~/.zshrc`):
```bash
export PATH="$HOME/.apl/bin:$PATH"
```

**Bash** (`~/.bashrc` or `~/.bash_profile`):
```bash
export PATH="$HOME/.apl/bin:$PATH"
```

**Fish** (`~/.config/fish/config.fish`):
```fish
fish_add_path ~/.apl/bin
```

Then reload your shell:
```bash
source ~/.zshrc  # or restart your terminal
```

## Your First Install

Update the package index and install a package:

```bash
# Fetch the latest package index
apl update

# Install ripgrep (a fast search tool)
apl install rg

# Verify it works
rg --version
```

## Shell Completions

Enable tab completion for your shell:

**Zsh**:
```bash
apl completions zsh > ~/.zfunc/_apl
# Add to ~/.zshrc: fpath+=~/.zfunc && autoload -Uz compinit && compinit
```

**Bash**:
```bash
apl completions bash > /etc/bash_completion.d/apl
```

**Fish**:
```bash
apl completions fish > ~/.config/fish/completions/apl.fish
```

## What's Next?

- [User Guide](user-guide.md) - Complete command reference
- [Package Format](package-format.md) - Create your own packages
- [Contributing](contributing.md) - Help improve APL

## Directory Structure

APL stores all data in `~/.apl/`:

```
~/.apl/
├── bin/          # Symlinks to installed binaries
├── store/        # Installed packages (versioned)
├── cache/        # Downloaded archives
├── index.bin     # Package index
└── state.db      # Installation database
```
