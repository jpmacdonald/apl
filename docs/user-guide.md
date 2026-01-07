# User Guide

Reference for APL CLI commands.

## Package Management

### Install

`apl install <package> [@version]`

```bash
# Install packages
apl install ripgrep fd bat

# Install specific version
apl install jq@1.7.1

# Dry run
apl install --dry-run neovim
```

### Remove

`apl remove <package>`

```bash
# Remove packages
apl remove ripgrep fd

# Remove all
apl remove --all

# Skip confirmation
apl remove --yes ripgrep
```

### List Installed Packages

```bash
apl list
```

Shows all installed packages with versions and sizes.

### Search for Packages

```bash
# Search by name or description
apl search editor

# Search for a specific tool
apl search json
```

### View Package Info

```bash
apl info neovim
```

Shows package details: version, description, size, and dependencies.

---

## Updates and Upgrades

### Update Package Index

```bash
apl update
```

Fetches the latest package definitions from the registry.

### Check for Updates

```bash
apl status
```

Shows which installed packages have newer versions available.

### Upgrade Packages

```bash
# Upgrade all packages
apl upgrade

# Upgrade specific packages
apl upgrade ripgrep neovim

# Skip confirmation
apl upgrade --yes
```

---

## Version Management

### Switch Versions

If you have multiple versions of a package installed, switch between them:

```bash
# Switch to a specific version
apl use jq@1.6

# The symlink in ~/.apl/bin now points to jq 1.6
```

### View History

```bash
apl history neovim
```

Shows the installation and upgrade history for a package.

### Rollback

```bash
apl rollback neovim
```

Reverts to the previously installed version.

---

## Advanced Features

### Run Without Installing

Run a package without installing it globally:

```bash
# Run a one-off command
apl run jq -- '.name' package.json

# Arguments after -- are passed to the package
apl run ripgrep -- --help
```

### Project Environments

Create isolated environments for projects using an `apl.toml` file:

```toml
# apl.toml
[dependencies]
ripgrep = "14.1"
fd = "10.2"
jq = "*"
```

Then enter the environment:

```bash
# Enter shell with project dependencies
apl shell

# Or run a command in the environment
apl shell -- rg "pattern" .

# CI mode: fail if lockfile is missing
apl shell --frozen
```

---

## Maintenance

### Clean Cache

Remove unused cached downloads and temporary files:

```bash
apl clean
```

### Self Update

Update APL itself to the latest version:

```bash
apl self-update
```

---

## Global Options

These options work with any command:

| Option | Description |
|--------|-------------|
| `--dry-run` | Show what would happen without making changes |
| `-q, --quiet` | Suppress non-essential output |
| `-h, --help` | Show help for a command |
| `-V, --version` | Show APL version |

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `APL_HOME` | `~/.apl` | Base directory for APL data |
| `APL_INDEX_URL` | `https://apl.pub/index` | Custom index URL |
| `GITHUB_TOKEN` | (none) | For higher API rate limits |

---

## Examples

### Set Up a Development Environment

```bash
# Install essential dev tools
apl install ripgrep fd bat eza delta lazygit

# Check what's installed
apl list
```

### Keep Everything Updated

```bash
# Update index and check for updates
apl update
apl status

# Upgrade everything
apl upgrade --yes
```

### Pin a Specific Version

```bash
# Install a specific version
apl install node@18.19.0

# Later, if you need a different version
apl install node@20.10.0
apl use node@18.19.0  # Switch back
```
