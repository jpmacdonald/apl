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

# In a real scenario, we would download the pre-built binary
# For this demo, we assume the user has cloned the repo or we point to a CDN
# curl -sL https://github.com/jimmy/distill/releases/latest/download/dl-macos-$ARCH -o "$BIN_DIR/dl"

# For local verification/bootstrap:
if [ -f "./target/release/dl" ]; then
    cp "./target/release/dl" "$BIN_DIR/dl"
elif [ -f "./target/debug/dl" ]; then
    cp "./target/debug/dl" "$BIN_DIR/dl"
fi

chmod +x "$BIN_DIR/dl"

echo "✓ Distill installed to $BIN_DIR/dl"
echo ""

# PATH Automation
DETECTED_PROFILE=""
case "$SHELL" in
    *zsh)  DETECTED_PROFILE="$HOME/.zshrc" ;;
    *bash) DETECTED_PROFILE="$HOME/.bashrc" ;;
    *fish) DETECTED_PROFILE="$HOME/.config/fish/config.fish" ;;
esac

if [ -n "$DETECTED_PROFILE" ]; then
    if ! grep -q ".dl/bin" "$DETECTED_PROFILE" 2>/dev/null; then
        echo "💡 Would you like to add ~/.dl/bin to your PATH in $DETECTED_PROFILE? (y/n)"
        # Note: In a real non-interactive script, we might just do it or use a flag --no-modify-path
        # For now, we'll just print the instructions to keep it safe.
        echo "   Run this to add it: echo 'export PATH=\"\$HOME/.dl/bin:\$PATH\"' >> $DETECTED_PROFILE"
    else
        echo "✓ ~/.dl/bin is already in your PATH ($DETECTED_PROFILE)"
    fi
else
    echo "💡 Add this to your shell profile:"
    echo "   export PATH=\"\$HOME/.dl/bin:\$PATH\""
fi

echo ""
echo "Run 'dl update' to get started!"
