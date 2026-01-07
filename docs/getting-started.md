# Getting Started

Fast, minimal package manager for macOS CLI tools.

## Installation

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

## Configuration

Add the binary path `~/.apl/bin` to your shell profile.

### Zsh
`~/.zshrc`:
```bash
export PATH="$HOME/.apl/bin:$PATH"
```

### Bash
`~/.bashrc`:
```bash
export PATH="$HOME/.apl/bin:$PATH"
```

### Fish
`~/.config/fish/config.fish`:
```fish
fish_add_path ~/.apl/bin
```

## Usage

Update the index and install a package:

```bash
apl update
apl install rg
rg --version
```

## Shell Completions

Generate and source completions for your shell.

### Zsh
```bash
apl completions zsh > ~/.zfunc/_apl
```

### Bash
```bash
apl completions bash > /etc/bash_completion.d/apl
```

### Fish
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
