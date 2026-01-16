#!/bin/bash
set -e

echo "=== 🚀 Simulating Pipeline Verification ==="

# 1. Build Utilities
echo "Step 1: Building apl-pkg..."
cargo build --release -p apl-pkg
PKG_BIN="./target/release/apl-pkg"

echo "Step 2: Building apl-builder..."
cargo build --release -p apl-builder
BUILDER_BIN="./target/release/apl-builder"

# 2. Simulate apl-pkg (Registry)
echo "Step 3: Verifying apl-pkg behavior..."
# Create a dummy registry
mkdir -p tmp/packages
echo 'name = "test-pkg"
description = "A test package"
[install]
bin = ["test"]
[discovery]
github = "owner/repo"
tag_pattern = "v{{version}}"' > tmp/packages/test.toml

# Run index (dry runish - output to file)
$PKG_BIN --registry tmp/packages index --output tmp/index --verbose
if [ -f "tmp/index" ]; then
    echo "✅ apl-pkg index generation successful"
else
    echo "❌ apl-pkg index failed"
    exit 1
fi

# 3. Simulate apl-builder (Ports)
echo "Step 4: Verifying apl-builder behavior..."
# Run local ports scan (dry-run if possible or minimal)
# We'll just check if it runs help successfully as a smoke test
if $BUILDER_BIN --help > /dev/null; then
    echo "✅ apl-builder execution successful"
else
    echo "❌ apl-builder failed to run"
    exit 1
fi

echo "=== 🎉 Simulation Complete: All binaries checks passed ==="
rm -rf tmp
