#!/bin/bash
set -e

# Distill (dl) Bootstrap Installer

DL_HOME="$HOME/.dl"
BIN_DIR="$DL_HOME/bin"
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

if [ "$OS" != "darwin" ]; then
    echo "✗ Distill currently only supports macOS."
    exit 1
fi

echo "🚀 Installing Distill (dl)..."

mkdir -p "$BIN_DIR"
mkdir -p "$DL_HOME/cache"

# For local development: copy from build directory
if [ -f "./target/release/dl" ]; then
    cp "./target/release/dl" "$BIN_DIR/dl"
elif [ -f "./target/debug/dl" ]; then
    cp "./target/debug/dl" "$BIN_DIR/dl"
fi

chmod +x "$BIN_DIR/dl"

echo "✓ Distill installed to $BIN_DIR/dl"
echo ""

# PATH Automation - ensure ~/.dl/bin comes FIRST in PATH
PATH_EXPORT='export PATH="$HOME/.dl/bin:$PATH"'

DETECTED_PROFILE=""
case "$SHELL" in
    *zsh)  DETECTED_PROFILE="$HOME/.zshrc" ;;
    *bash) DETECTED_PROFILE="$HOME/.bashrc" ;;
    *fish) DETECTED_PROFILE="$HOME/.config/fish/config.fish" ;;
esac

if [ -n "$DETECTED_PROFILE" ]; then
    if grep -q ".dl/bin" "$DETECTED_PROFILE" 2>/dev/null; then
        echo "✓ ~/.dl/bin is already in your PATH ($DETECTED_PROFILE)"
    else
        # Add to profile (at the end so it takes priority)
        echo "" >> "$DETECTED_PROFILE"
        echo "# Distill package manager" >> "$DETECTED_PROFILE"
        echo "$PATH_EXPORT" >> "$DETECTED_PROFILE"
        echo "✓ Added ~/.dl/bin to PATH in $DETECTED_PROFILE"
        echo "  Run 'source $DETECTED_PROFILE' or restart your terminal"
    fi
else
    echo "💡 Add this to your shell profile:"
    echo "   $PATH_EXPORT"
fi

echo ""
echo "Run 'dl update' to get started!"
