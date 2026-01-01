#!/bin/bash
# Comprehensive package audit for naming and binary mismatches

echo "=== Package Naming and Binary Audit ==="
echo ""

errors=0

echo "## 1. Checking for filename vs binary name mismatches"
echo ""

for toml in packages/*.toml; do
  filename=$(basename "$toml" .toml)
  pkg_name=$(grep "^name = " "$toml" | sed 's/name = "\(.*\)"/\1/')
  bin_names=$(grep "^bin = " "$toml" | sed 's/bin = \[\(.*\)\]/\1/' | tr ',' '\n' | sed 's/[" ]//g')
  
  # Check if filename matches package name
  if [ "$filename" != "$pkg_name" ]; then
    echo "⚠️  TOML NAME MISMATCH: $toml"
    echo "   Filename: $filename"
    echo "   Package name: $pkg_name"
    echo "   Fix: Rename to ${pkg_name}.toml"
    ((errors++))
    echo ""
  fi
  
  # Check if package name matches any binary name
  match_found=false
  for bin in $bin_names; do
    if [ "$bin" = "$pkg_name" ]; then
      match_found=true
      break
    fi
  done
  
  if [ "$match_found" = false ] && [ -n "$bin_names" ]; then
    echo "⚠️  BINARY NAME MISMATCH: $toml"
    echo "   Package: $pkg_name"
    echo "   Binaries: $(echo $bin_names | tr '\n' ', ')"
    echo "   Issue: Package name '$pkg_name' not in bin list"
    ((errors++))
    echo ""
  fi
done

echo "## 2. Checking for duplicate package names"
echo ""

duplicates=$(grep -h "^name = " packages/*.toml | sort | uniq -d)
if [ -n "$duplicates" ]; then
  echo "⚠️  DUPLICATE NAMES FOUND:"
  echo "$duplicates"
  echo ""
  ((errors++))
fi

echo "## 3. Known Issues to Fix"
echo ""

# Check problematic packages
if [ -f "packages/cli.toml" ]; then
  repo=$(grep "homepage" "packages/cli.toml" | grep -o 'cli/cli')
  if [ -n "$repo" ]; then
    echo "❌ cli.toml: Should be gh.toml (repo: cli/cli, binary: gh)"
    ((errors++))
  fi
fi

if [ -f "packages/opentofu.toml" ]; then
  bin=$(grep "^bin = " "packages/opentofu.toml" | grep -o 'tofu')
  if [ -z "$bin" ]; then
    echo "❌ opentofu.toml: bin should be ['tofu'] not ['opentofu']"
    ((errors++))
  fi
fi

echo ""
echo "=== Summary ==="
echo "Total issues found: $errors"

if [ $errors -eq 0 ]; then
  echo "✅ All packages pass audit!"
  exit 0
else
  echo "⚠️  Please review and fix issues above"
  exit 1
fi
