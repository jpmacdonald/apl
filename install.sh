#!/bin/bash
set -e

# apl - A Package Layer Bootstrap Installer

APL_HOME="$HOME/.apl"
BIN_DIR="$APL_HOME/bin"
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

if [ "$OS" != "darwin" ]; then
    echo "✗ apl currently only supports macOS."
    exit 1
fi

echo "🚀 Installing apl (A Package Layer)..."

mkdir -p "$BIN_DIR"
mkdir -p "$APL_HOME/cache"
mkdir -p "$APL_HOME/tmp"

# Binary Resolution
if [ -f "./target/release/apl" ]; then
    echo "  → Using local release build"
    cp "./target/release/apl" "$BIN_DIR/apl"
elif [ -f "./target/debug/apl" ]; then
    echo "  → Using local debug build"
    cp "./target/debug/apl" "$BIN_DIR/apl"
else
    # Remote Production Download
    echo "  → Downloading latest release from apl.pub..."
    
    # Simple arch mapping
    if [ "$ARCH" = "arm64" ]; then REMOTE_ARCH="arm64"; else REMOTE_ARCH="x86_64"; fi
    
    # The Cloudflare Worker handles the resolution via /active-binary/:arch
    DOWNLOAD_URL="https://apl.pub/active-binary/${REMOTE_ARCH}"
    
    TMP_FILE="$APL_HOME/tmp/apl_install.tar.gz"
    curl -fL "$DOWNLOAD_URL" -o "$TMP_FILE"
    
    tar -xzf "$TMP_FILE" -C "$APL_HOME/tmp" 2>/dev/null || {
        echo "✗ Failed to extract binary. Worker might still be updating."
        exit 1
    }
    mv "$APL_HOME/tmp/apl" "$BIN_DIR/apl"
    rm "$TMP_FILE"
fi

chmod +x "$BIN_DIR/apl"

echo "✓ apl installed to $BIN_DIR/apl"
echo ""

# PATH Automation - ensure ~/.apl/bin comes FIRST in PATH
PATH_EXPORT='export PATH="$HOME/.apl/bin:$PATH"'

DETECTED_PROFILE=""
case "$SHELL" in
    *zsh)  DETECTED_PROFILE="$HOME/.zshrc" ;;
    *bash) DETECTED_PROFILE="$HOME/.bashrc" ;;
    *fish) DETECTED_PROFILE="$HOME/.config/fish/config.fish" ;;
esac

if [ -n "$DETECTED_PROFILE" ]; then
    if grep -q ".apl/bin" "$DETECTED_PROFILE" 2>/dev/null; then
        echo "✓ ~/.apl/bin is already in your PATH ($DETECTED_PROFILE)"
    else
        echo "" >> "$DETECTED_PROFILE"
        echo "# apl - A Package Layer" >> "$DETECTED_PROFILE"
        echo "$PATH_EXPORT" >> "$DETECTED_PROFILE"
        echo "✓ Added ~/.apl/bin to PATH in $DETECTED_PROFILE"
        echo "  Run 'source $DETECTED_PROFILE' or restart your terminal"
    fi
else
    echo "💡 Add this to your shell profile:"
    echo "   $PATH_EXPORT"
fi

echo ""
echo "Run 'apl update' to get started!"
