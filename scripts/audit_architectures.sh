#!/bin/bash
# Audit all packages for architecture support

echo "=== Package Architecture Audit ==="
echo ""

echo "Packages with both ARM64 and x86_64:"
for f in registry/*/*.toml; do
  [ -e "$f" ] || continue
  if grep -q 'arm64 =' "$f" && grep -q 'x86_64 =' "$f"; then
    echo "✅ $(basename $f .toml)"
  fi
done | wc -l

echo ""
echo "Packages with ARM64 only (missing x86_64):"
for f in registry/*/*.toml; do
  [ -e "$f" ] || continue
  if grep -q 'arm64 =' "$f" && ! grep -q 'x86_64 =' "$f"; then
    echo "⚠️  $(basename $f .toml)"
  fi
done

echo ""
echo "Packages with x86_64 only (missing ARM64):"
for f in registry/*/*.toml; do
  [ -e "$f" ] || continue
  if grep -q 'x86_64 =' "$f" && ! grep -q 'arm64 =' "$f"; then
    echo "⚠️  $(basename $f .toml)"
  fi
done

echo ""
echo "=== Summary ==="
total=$(find registry -name "*.toml" | wc -l)
both=$(for f in registry/*/*.toml; do [ -e "$f" ] || continue; if grep -q 'arm64 =' "$f" && grep -q 'x86_64 =' "$f"; then echo "$f"; fi; done | wc -l)
arm_only=$(for f in registry/*/*.toml; do [ -e "$f" ] || continue; if grep -q 'arm64 =' "$f" && ! grep -q 'x86_64 =' "$f"; then echo "$f"; fi; done | wc -l)  
x86_only=$(for f in registry/*/*.toml; do [ -e "$f" ] || continue; if grep -q 'x86_64 =' "$f" && ! grep -q 'arm64 =' "$f"; then echo "$f"; fi; done | wc -l)

echo "Total packages: $total"
echo "Both architectures: $both"
echo "ARM64 only: $arm_only"
echo "x86_64 only: $x86_only"
echo "Coverage: $(echo "scale=1; $both * 100 / $total" | bc)%"
