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

echo "🚀 Installing apl..."

mkdir -p "$BIN_DIR" "$APL_HOME/cache" "$APL_HOME/tmp"

# Binary Resolution
if [ -f "./target/release/apl" ]; then
    echo "  → Using local release build"
    cp "./target/release/apl" "$BIN_DIR/apl"
elif [ -f "./target/debug/apl" ]; then
    echo "  → Using local debug build"
    cp "./target/debug/apl" "$BIN_DIR/apl"
else
    # Remote Production Download
    echo "  → Fetching latest release info..."
    
    # Map architecture to manifest key
    case "$ARCH" in
        arm64|aarch64) TARGET="darwin-arm64" ;;
        x86_64)        TARGET="darwin-x64" ;;
        *)             echo "✗ Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    
    # Fetch release manifest (JSON)
    MANIFEST_URL="https://apl.pub/manifest.json"
    MANIFEST=$(curl -sL "$MANIFEST_URL")

    if [ -z "$MANIFEST" ]; then
        error "Failed to fetch release manifest from $MANIFEST_URL"
        exit 1
    fi

# Parse JSON to find URL for target using python for reliability
if command -v python3 >/dev/null 2>&1; then
    DOWNLOAD_URL=$(echo "$MANIFEST" | python3 -c "import sys, json; print(json.load(sys.stdin).get('$TARGET', ''))")
elif command -v python >/dev/null 2>&1; then
    DOWNLOAD_URL=$(echo "$MANIFEST" | python -c "import sys, json; print(json.load(sys.stdin).get('$TARGET', ''))")
else
    error "Python is required to parse the manifest, but it was not found."
    exit 1
fi

if [ -z "$DOWNLOAD_URL" ]; then
    error "No binary found for platform: $TARGET"
    echo "   Manifest content:"
    echo "$MANIFEST"
    exit 1
fi
    
    echo "  → Downloading apl for $ARCH..."
    TMP_FILE="$APL_HOME/tmp/apl_install.tar.gz"
    curl -fL "$DOWNLOAD_URL" -o "$TMP_FILE"
    
    tar -xzf "$TMP_FILE" -C "$APL_HOME/tmp"
    mv "$APL_HOME/tmp/apl" "$BIN_DIR/apl"
    rm "$TMP_FILE"
fi

chmod +x "$BIN_DIR/apl"

echo "✓ apl installed to $BIN_DIR/apl"
echo ""

# PATH Automation
PATH_EXPORT='export PATH="$HOME/.apl/bin:$PATH"'

DETECTED_PROFILE=""
case "$SHELL" in
    *zsh)  DETECTED_PROFILE="$HOME/.zshrc" ;;
    *bash) DETECTED_PROFILE="$HOME/.bashrc" ;;
    *fish) DETECTED_PROFILE="$HOME/.config/fish/config.fish" ;;
esac

if [ -n "$DETECTED_PROFILE" ]; then
    if grep -q ".apl/bin" "$DETECTED_PROFILE" 2>/dev/null; then
        echo "✓ ~/.apl/bin is already in your PATH"
    else
        echo "" >> "$DETECTED_PROFILE"
        echo "# apl" >> "$DETECTED_PROFILE"
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
