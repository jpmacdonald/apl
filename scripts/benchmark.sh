#!/bin/bash
# Benchmark script for APL installation

# Verified Packages (mix of sizes and types)
# Removed dust (missing arm64), yq (missing checksum)
# Renamed bottom -> btm
PACKAGES="bat jq gh starship zoxide fzf exa lsd delta btm duf procs sd hyperfine gping xh gitui glow k9s fd"

echo "Starting benchmark..."
echo "Packages: $PACKAGES"
echo "Count: $(echo $PACKAGES | wc -w)"

# Ensure clean slate
echo "Cleaning up..."
apl remove -y -a 2>/dev/null || true

# Run benchmark
echo "Installing packages..."
start_time=$(date +%s)
apl install $PACKAGES
end_time=$(date +%s)

duration=$((end_time - start_time))
echo ""
echo "----------------------------------------"
echo "Benchmark Complete"
echo "Total Duration: ${duration} seconds"
echo "Average per package: $(echo "scale=2; $duration / 20" | bc) seconds"
echo "----------------------------------------"
