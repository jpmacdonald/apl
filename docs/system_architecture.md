# APL System Architecture

## Overview
APL (Advanced Package Loader) is a high-performance macOS package manager designed to replace Homebrew with native binaries, hermetic builds, and sub-second install times.

---

## Components Inventory

### Core Modules (`src/core/`)
| Module | Status | Description |
|--------|--------|-------------|
| `index.rs` | ✅ Working | MMAP-based binary index (postcard), O(1) lookups, version 4 |
| `package.rs` | ✅ Working | TOML package definitions, BuildSpec support |
| `version.rs` | ✅ Working | SemVer parsing, PackageSpec (name@version) |
| `resolver.rs` | ✅ Working | Dependency resolution (topological sort) |
| `lockfile.rs` | ⚠️ Partial | Lockfile generation/reading (unused in main flow) |
| `sysroot.rs` | ✅ Working | APFS clonefile COW directories for builds |
| `builder.rs` | ✅ Working | Source build orchestration |
| `relinker.rs` | ✅ Working | install_name_tool wrapper for @rpath |

### I/O Modules (`src/io/`)
| Module | Status | Description |
|--------|--------|-------------|
| `download.rs` | ✅ Working | Pipelined download+extract, gzip/zstd/zip |
| `extract.rs` | ✅ Working | Sync extraction fallback |
| `output.rs` | ✅ Working | Rich terminal UI (TableOutput) |
| `dmg.rs` | ⚠️ Partial | DMG mounting for apps |
| `ui_actor.rs` | ⚠️ Unused | Message-passing UI (not integrated) |

### Commands (`src/cmd/`)
| Command | Status | User Story |
|---------|--------|------------|
| `install` | ✅ Core | Install packages from index or local .toml |
| `remove` | ✅ Core | Uninstall packages |
| `list` | ✅ Core | Show installed packages |
| `update` | ⚠️ Partial | Refresh index (no auto-upgrade) |
| `upgrade` | ⚠️ Partial | Upgrade all packages |
| `use` | ⚠️ Partial | Switch active version |
| `search` | ⚠️ Minimal | Search index |
| `info` | ⚠️ Minimal | Show package details |
| `status` | ⚠️ Partial | Show system status |
| `history` | ⚠️ Minimal | Show install history |
| `rollback` | ⚠️ Minimal | Rollback to previous state |
| `clean` | ⚠️ Minimal | Clean cache |
| `run` | ⚠️ Minimal | Run package in ephemeral env |
| `lock` | ⚠️ Unused | Lock dependencies |
| `hash` | ✅ Util | Compute blake3 |
| `generate-index` | ✅ Util | Generate index from packages/ |
| `self-update` | ⚠️ Partial | Update apl itself |
| `completions` | ⚠️ Minimal | Shell completions |

### Store (`src/store/`)
| Module | Status | Description |
|--------|--------|-------------|
| `db.rs` | ✅ Working | SQLite state database |

---

## Data Flow

```
1. INDEX LOOKUP
   ~/.apl/index.bin (MMAP) → find("ripgrep") → VersionInfo

2. DOWNLOAD + EXTRACT (Pipelined)
   HTTP Stream → [Hasher + Cache] + [Decompressor → Tar Unpack]
   
3. INSTALL
   temp/extracted/ → ~/.apl/store/ripgrep/14.1.1/
   
4. LINK
   ~/.apl/store/.../bin/rg → ~/.apl/bin/rg (symlink)
   
5. DATABASE
   StateDb.install_package(name, version, blake3, files)
```

---

## MVP Definition

### Must Work (P0)
- [ ] `apl install <package>` - Binary install from index
- [ ] `apl remove <package>` - Full cleanup
- [ ] `apl list` - Show installed packages
- [ ] `apl search <query>` - Find packages
- [ ] `apl info <package>` - Show package details
- [ ] PATH setup works correctly

### Should Work (P1)
- [ ] `apl install <local.toml>` - Install from local definition
- [ ] `apl update` - Refresh index
- [ ] `apl upgrade` - Upgrade outdated packages
- [ ] `apl use <package@version>` - Version switching
- [ ] App bundle installs (DMG/zip → /Applications)

### Nice to Have (P2)
- [ ] Source builds with dependencies
- [ ] Shell completions
- [ ] History/rollback
- [ ] `apl run` ephemeral environments

---

## End-to-End Test Plan

### Test 1: Binary CLI Tool
```bash
apl install ripgrep
rg --version          # Verify works
apl list              # Shows ripgrep
apl remove ripgrep
which rg              # Should fail
```

### Test 2: App Bundle
```bash
apl install ghostty
ls /Applications/Ghostty.app  # Verify installed
open -a Ghostty               # Verify launches
apl remove ghostty
```

### Test 3: Version Management
```bash
apl install neovim@0.9.0
nvim --version                # 0.9.0
apl install neovim@0.10.4
nvim --version                # 0.10.4
apl use neovim@0.9.0
nvim --version                # 0.9.0
```

### Test 4: Source Build (Deferred)
Create minimal C program with no dependencies:
```bash
apl install packages/simple-source.toml
simple-tool --version
```

---

## Known Issues

1. **ui_actor.rs unused** - Message-passing UI implemented but not integrated
2. **lockfile.rs unused** - Lockfile module exists but not in main flow
3. **update/upgrade partial** - Update refreshes index but upgrade logic incomplete
4. **DMG handling fragile** - Works for some apps, not all

---

## Next Actions

1. **Execute P0 Test Suite** - Validate core user stories
2. **Fix Identified Issues** - Address failures from testing
3. **Create Simple Source Package** - Minimal C tool for source build testing
4. **Documentation** - User-facing README and usage guide
