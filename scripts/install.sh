#!/usr/bin/env bash
# webrain installer — downloads the latest release binary for Linux/macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/prokopis3/webrain/main/scripts/install.sh | bash
#
# (once you own a domain like webrain.sh, mirror this file at /install and
#  use `curl -fsSL https://webrain.sh/install | bash`)
set -euo pipefail

REPO="prokopis3/webrain"
INSTALL_DIR="${WEBRAIN_INSTALL_DIR:-$HOME/.local/bin}"
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)  ASSET="webrain-linux" ;;
  Darwin) ASSET="webrain-macos" ;;
  *)
    echo "webrain installer: unsupported OS '$OS'." >&2
    echo "On Windows use:  scoop bucket add webrain https://github.com/prokopis3/scoop-webrain && scoop install webrain" >&2
    exit 1
    ;;
esac

case "$ARCH" in
  x86_64|amd64) : ;;
  arm64|aarch64) echo "webrain installer: $ARCH builds are not published yet (x86_64 only)." >&2; exit 1 ;;
  *) echo "webrain installer: unsupported architecture '$ARCH'." >&2; exit 1 ;;
esac

mkdir -p "$INSTALL_DIR"
URL="https://github.com/$REPO/releases/latest/download/$ASSET"
echo "webrain: downloading $URL"
if ! curl -fsSL "$URL" -o "$INSTALL_DIR/webrain"; then
  echo "webrain: download failed. Check your network or the release at https://github.com/$REPO/releases" >&2
  exit 1
fi
chmod +x "$INSTALL_DIR/webrain"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) : ;;
  *) echo "webrain: installed to $INSTALL_DIR/webrain — add $INSTALL_DIR to your PATH." >&2 ;;
esac

echo "webrain: installed. Next steps:"
echo "  webrain install          # download Chrome for Testing (first run)"
echo "  webrain doctor           # verify the install"
echo "  webrain mcp --http 9223  # start the MCP server"
