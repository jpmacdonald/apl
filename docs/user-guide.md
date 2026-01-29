# User Guide

## Install packages

```bash
apl install ripgrep           # latest version
apl install ripgrep fd bat    # multiple packages
apl install jq@1.7.1          # specific version
apl install --dry-run neovim  # preview without installing
```

## Remove packages

```bash
apl remove ripgrep            # remove one package
apl remove ripgrep fd         # remove multiple
apl remove --yes ripgrep      # skip confirmation
apl remove --all              # remove everything
```

## List installed packages

```bash
apl list
```

Shows package name, version, and size.

## Search packages

```bash
apl search json               # search by name or description
apl search editor
```

## Package info

```bash
apl info neovim
```

Shows version, description, size, dependencies, and install status.

## Update index

```bash
apl update
```

Fetches the latest package index from the registry.

## Check for updates

```bash
apl status
```

Shows which installed packages have newer versions.

## Upgrade packages

```bash
apl upgrade                   # upgrade all
apl upgrade ripgrep neovim    # upgrade specific packages
apl upgrade --yes             # skip confirmation
```

## Version management

```bash
apl install jq@1.6            # install old version
apl install jq@1.7            # install new version (both coexist)
apl use jq@1.6                # switch active version
```

## History and rollback

```bash
apl history neovim            # view install/upgrade history
apl rollback neovim           # revert to previous version
```

## Run without installing

```bash
apl run jq -- '.name' package.json
apl run ripgrep -- --help
```

Arguments after `--` are passed to the command.

## Project environments

Create an `apl.toml` in your project:

```toml
[dependencies]
ripgrep = "14.1"
fd = "10.2"
jq = "*"
```

Then:

```bash
apl shell                     # enter environment with project deps
apl shell -- rg "pattern" .   # run command in environment
apl shell --frozen            # fail if lockfile missing (CI mode)
```

## Maintenance

```bash
apl clean                     # remove cached downloads
apl self-update               # update APL itself
```

## Options

| Option | Description |
|--------|-------------|
| `--dry-run` | preview without changes |
| `-q, --quiet` | suppress output |
| `-h, --help` | show help |
| `-V, --version` | show version |

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `APL_HOME` | `~/.apl` | base directory |
| `APL_INDEX_URL` | `https://apl.pub/index` | index URL |
| `GITHUB_TOKEN` | - | for higher API rate limits |
