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

INSTALL_DIR="$(printf '%s' "$INSTALL_DIR" | sed 's:/*$::')"
mkdir -p "$INSTALL_DIR"
URL="https://github.com/$REPO/releases/latest/download/$ASSET"
echo "webrain: downloading $URL"
# Download to a temp file, sanity-check it, then atomically rename into place —
# writing straight to the final path would destroy a working install if the
# download is interrupted or fails partway.
TMP="$INSTALL_DIR/.webrain.tmp.$$"
if ! curl -fsSL "$URL" -o "$TMP"; then
  rm -f "$TMP"
  echo "webrain: download failed. Check your network or the release at https://github.com/$REPO/releases" >&2
  exit 1
fi
if [ ! -s "$TMP" ]; then
  rm -f "$TMP"
  echo "webrain: downloaded file is empty — aborting (truncated/HTML error page?)." >&2
  exit 1
fi
# Verify against the release's published checksums.txt (the release workflow
# publishes it): a tampered release/mirror/MITM fails the check instead of
# silently installing arbitrary code that runs on the next launch.
SUM_URL="https://github.com/$REPO/releases/latest/download/checksums.txt"
SUM_FILE="$INSTALL_DIR/.webrain.sum.$$"
if curl -fsSL "$SUM_URL" -o "$SUM_FILE" 2>/dev/null; then
  EXPECTED=$(awk -v a="$ASSET" '$2 == a { print $1 }' "$SUM_FILE" | head -n1)
  rm -f "$SUM_FILE"
  if [ -n "$EXPECTED" ]; then
    ACTUAL=$(sha256sum "$TMP" 2>/dev/null | awk '{print $1}' || shasum -a 256 "$TMP" | awk '{print $1}')
    if [ "$ACTUAL" != "$EXPECTED" ]; then
      rm -f "$TMP"
      echo "webrain: SHA-256 mismatch for $ASSET — aborting (tampered download?)." >&2
      exit 1
    fi
    echo "webrain: SHA-256 verified."
  else
    echo "webrain: checksums.txt has no entry for $ASSET — skipping verification." >&2
  fi
else
  rm -f "$SUM_FILE"
  echo "webrain: could not fetch checksums.txt — skipping verification." >&2
fi
chmod +x "$TMP"
mv -f "$TMP" "$INSTALL_DIR/webrain"

case ":$PATH:" in
  *":$INSTALL_DIR:"*|*":$INSTALL_DIR/:"*) : ;;
  *) echo "webrain: installed to $INSTALL_DIR/webrain — add $INSTALL_DIR to your PATH." >&2 ;;
esac

echo "webrain: installed. Next steps:"
echo "  webrain install          # download Chrome for Testing (first run)"
echo "  webrain doctor           # verify the install"
echo "  webrain mcp --http 9223  # start the MCP server"
