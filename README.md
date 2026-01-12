# APL: Advanced Package Layer

APL is a next-generation package manager for macOS, focusing on hermetic builds, strict versioning, and unified architecture.

## Unified Workspace

This repository is a Cargo Workspace containing the entire ecosystem:

### 1. The Client (`crates/apl`)
The consumer CLI (`apl`) installed on user machines.
- **Role**: Discovers, verifies, and installs packages.
- **Feeds**: 
    - **Feed A (GitHub)**: Upstream releases via GitHub API.
    - **Feed B (Ports)**: Hermetic artifacts from our R2 Registry.

### 2. The Engine (`crates/apl-ports`)
The producer binary that powers Feed B.
- **Role**: Scrapes vendor sites (AWS, Python, Ruby, etc.), validates artifacts (SHA256), and indexes them.
- **Output**: `index.json` files uploaded to R2 (`ports/<name>/index.json`).
- **Pipeline**: Runs daily via `.github/workflows/update-ports.yml`.

### 3. Shared Types (`crates/apl-types`)
Type definitions shared between Engine and Client to guarantee contract validity.
- **Artifact**: Strict schema with validation (`validate()`).
- **PortConfig**: Declarative port definitions.

## Ports Registry (`ports/`)
Declarative definitions for ports managed by the Engine.
- `ports/terraform/port.toml`
- `ports/node/port.toml`
- ...

## Development

```bash
# Run the Client
cargo run -p apl -- install node

# Run the Engine (Dry Run)
cargo run -p apl-ports -- --dry-run
```
