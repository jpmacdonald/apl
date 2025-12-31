# APL System Architecture

## Overview
APL (Advanced Package Loader) is a high-performance macOS package manager designed to deliver native binaries, hermetic builds, and sub-second installation times.

---

## Component Inventory

### Core Modules (`src/core/`)
| Module | Status | Description |
|--------|--------|-------------|
| `index.rs` | Functional | MMAP-based binary index (postcard), O(1) lookups, version 4 |
| `package.rs` | Functional | TOML package definitions and BuildSpec support |
| `version.rs` | Functional | SemVer parsing and PackageSpec (name@version) utility |
| `resolver.rs` | Functional | Dependency resolution using topological sort |
| `sysroot.rs` | Functional | APFS clonefile COW directories for isolated builds |
| `builder.rs` | Functional | Source build orchestration |
| `relinker.rs` | Functional | install_name_tool wrapper for @rpath adjustment |

### User Interface (`src/ui/`)
| Module | Status | Description |
|--------|--------|-------------|
| `output.rs` | Functional | Rich terminal UI implementation (TableOutput) |
| `actor.rs` | Implementation | Message-passing architecture for concurrent UI updates |
| `theme.rs` | Functional | UI theming and layout configuration |

### Registry Management (`src/registry/`)
| Module | Status | Description |
|--------|--------|-------------|
| `github.rs` | Functional | GitHub release fetching and intelligent asset detection |
| `mod.rs` | Functional | Shared registry logic and client orchestration |

### Commands (`src/cmd/` and `src/bin/`)
| Command | Location | Description |
|---------|----------|-------------|
| `install` | `src/cmd/` | Binary and source installation logic |
| `remove` | `src/cmd/` | Package uninstallation and cleanup |
| `list` | `src/cmd/` | Display installed packages |
| `update` | `src/cmd/` | Refresh the local package index |
| `apl-pkg` | `src/bin/` | Primary registry maintenance tool (add, update, check, index) |

---

## Data Flow

1. **Index Lookup**
   `~/.apl/index.bin` (MMAP) → `find("ripgrep")` → `VersionInfo`

2. **Download and Extraction**
   HTTP Stream → [Hasher + Cache] + [Decompressor → Tar/Zip Unpack]

3. **Installation**
   Temporary extraction → `~/.apl/store/ripgrep/14.1.1/`

4. **Linking**
   `~/.apl/store/.../bin/rg` → `~/.apl/bin/rg` (symlink)

5. **Database Update**
   `StateDb.install_package(name, version, blake3, files)`

---

## Project Status

### Core Functionality (P0)
- [x] Binary installation from index (`apl install`)
- [x] Full package cleanup (`apl remove`)
- [x] List installed packages (`apl list`)
- [x] Search registry (`apl search`)
- [x] Display package details (`apl info`)
- [x] PATH integration (`~/.apl/bin`)

### Registry Maintenance (P1)
- [x] Automated package addition (`apl-pkg add`)
- [x] Version refresh and auto-update (`apl-pkg update`)
- [x] Index generation (`apl-pkg index`)
- [x] Registry integrity validation (`apl-pkg check`)

### Advanced Features (P2)
- [ ] Source builds with full dependency resolution
- [ ] Comprehensive shell completions
- [ ] Transaction history and rollback
- [ ] Ephemeral environments (`apl run`)

---

## Known Limitations

1. **UI Actor Integration**: The message-passing UI architecture is implemented but not yet fully integrated into the main installation flow.
2. **DMG Handling**: DMG mounting support is functional for some packages but requires further stabilization for complex bundles.

---

## Future Roadmap

1. **Complete P0 Validation**: Finalize the core test suite to ensure stability across macOS versions.
2. **Enhance Source Builds**: Stabilize the isolated build environment for complex multi-dependency projects.
3. **Expand Registry**: Utilize `apl-pkg` to grow the official APL package index.
