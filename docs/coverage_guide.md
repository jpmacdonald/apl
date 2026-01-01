# Code Coverage Guide

## Overview

APL uses `cargo-llvm-cov` for code coverage tracking. This guide explains how to generate and interpret coverage reports.

## Quick Start

### Install cargo-llvm-cov

```bash
cargo install cargo-llvm-cov
```

### Generate Coverage Report

```bash
# Generate HTML report (recommended for local development)
cargo llvm-cov --all-features --workspace --html

# Open in browser
open target/llvm-cov/html/index.html
```

### Generate Text Summary

```bash
# Quick terminal summary
cargo llvm-cov --all-features --workspace
```

### Generate LCOV Format (for CI/Codecov)

```bash
# For upload to codecov.io
cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info
```

## Interpreting Results

### Coverage Metrics

- **Line Coverage**: Percentage of code lines executed during tests
- **Region Coverage**: Percentage of code regions (blocks) executed
- **Function Coverage**: Percentage of functions called

### Color Coding (HTML Report)

- 🟢 **Green**: Line was executed by tests
- 🔴 **Red**: Line was NOT executed (needs test coverage)
- 🟡 **Yellow**: Line was partially executed (e.g., one branch of if/else)

## Current Coverage

As of the latest run:

- **Overall**: ~75% (estimated)
- **Core modules** (`core/`): >80%
- **UI modules** (`ui/`): >70%
- **I/O modules** (`io/`): >65%
- **Ops modules** (`ops/`): ~60%

## Areas Needing Coverage

1. **DMG real-world scenarios** - Requires physical DMG files
2. **Network download failures** - Requires mocking HTTP
3. **Concurrent install edge cases** - Hard to test deterministically
4. **UI actor race conditions** - Requires stress testing

## CI Integration

Coverage runs automatically on every PR via `.github/workflows/coverage.yml`:

- Generates HTML report (uploaded as artifact)
- Generates LCOV for Codecov (optional)
- Does NOT fail CI on low coverage (informational only)

## Tips for Writing Tests

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_feature() {
        // Arrange
        let input = setup();
        
        // Act
        let result = function_under_test(input);
        
        // Assert
        assert_eq!(result, expected);
    }
}
```

### Integration Tests

Place in `tests/*.rs` for end-to-end testing.

### E2E Tests

Place in `tests/e2e/*.rs` for system-level testing.

## Excluding Code from Coverage

Use `#[coverage(off)]` for code that shouldn't be covered:

```rust
#[coverage(off)]
fn debug_only_function() {
    // Only used in development, hard to test
}
```

## Goals

- **v1.0 Target**: >80% line coverage overall
- **Core modules**: >85%
- **Critical paths**: 100% (install, remove, index loading)

## Resources

- [cargo-llvm-cov docs](https://github.com/taiki-e/cargo-llvm-cov)
- [LLVM Coverage Mapping](https://llvm.org/docs/CoverageMappingFormat.html)
