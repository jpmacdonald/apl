# apl - A Package Layer

> **A fast binary package manager for macOS.**  
> Like brew, but faster.

## Installation

```bash
curl -sL https://raw.githubusercontent.com/jpmacdonald/distill/main/install.sh | sh
```

## Quick Start

```bash
apl update              # Fetch latest package index
apl install bat fd jq   # Install packages
apl list                # Show installed packages
```

## Commands

### Core
```bash
apl install <pkg>           # Install latest
apl install <pkg>@<version> # Install specific version
apl remove <pkg>            # Uninstall
apl list                    # Show installed
apl search <query>          # Find packages
apl update                  # Sync index
apl upgrade                 # Upgrade all
apl clean                   # Remove cached files
```

### Power User
```bash
apl switch <pkg>@<version>  # Switch active version
apl rollback <pkg>          # Undo last change
apl history <pkg>           # View changes
apl run <pkg>               # Run without installing
apl info <pkg>              # Package details
apl lock                    # Generate apl.lock
```

### Developer
```bash
apl package new <name>      # Create package template
apl package check <file>    # Validate package
apl generate-index          # Build index.bin
apl hash <file>             # Compute BLAKE3
apl completions <shell>     # Shell completions
```

## Options

- `--dry-run` - Show what would happen
- `-q, --quiet` - Suppress output

## Architecture

```
~/.apl/
├── bin/        # Symlinks to binaries
├── cache/      # Downloaded files
├── index.bin   # Package index
└── state.db    # SQLite state
```

---

MIT License
