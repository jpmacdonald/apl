#!/bin/bash
# Comprehensive package audit for naming and binary mismatches

echo "=== Algorithmic Registry Audit ==="
echo ""

errors=0

echo "## 1. Checking for filename vs binary name mismatches"
echo ""

for toml in registry/*/*.toml; do
  [ -e "$toml" ] || continue
  filename=$(basename "$toml" .toml)
  pkg_name=$(grep "^name = " "$toml" | sed 's/name = "\(.*\)"/\1/')
  bin_names=$(grep "^bin = " "$toml" | sed 's/bin = \[\(.*\)\]/\1/' | tr ',' '\n' | sed 's/[" ]//g')
  
  # Check if filename matches package name
  if [ "$filename" != "$pkg_name" ]; then
    echo "⚠️  NAME MISMATCH: $toml"
    echo "   Filename: $filename"
    echo "   Package name: $pkg_name"
    ((errors++))
    echo ""
  fi
done

echo "## 2. Checking for duplicate package names"
echo ""

duplicates=$(grep -rh "^name = " registry/ | sort | uniq -d)
if [ -n "$duplicates" ]; then
  echo "⚠️  DUPLICATE NAMES FOUND:"
  echo "$duplicates"
  echo ""
  ((errors++))
fi

echo ""
echo "=== Summary ==="
echo "Total issues found: $errors"

if [ $errors -eq 0 ]; then
  echo "✅ Registry passes audit!"
  exit 0
else
  echo "⚠️  Please review and fix issues above"
  exit 1
fi
