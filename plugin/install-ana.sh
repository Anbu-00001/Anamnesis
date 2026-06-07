#!/usr/bin/env bash
# Fetch the prebuilt `ana` engine for this platform into ~/.anamnesis/bin, where
# the Anamnesis plugin's hooks look for it. Falls back to a build hint if there
# is no prebuilt binary for your platform.
#
#   bash install-ana.sh [version]      # version defaults to "latest"
set -euo pipefail

REPO="${ANAMNESIS_REPO:-Anbu-00001/Anamnesis}"
DEST="$HOME/.anamnesis/bin"
mkdir -p "$DEST"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)   target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64)  target=aarch64-unknown-linux-gnu ;;
  Darwin-x86_64)  target=x86_64-apple-darwin ;;
  Darwin-arm64)   target=aarch64-apple-darwin ;;
  *) echo "No prebuilt binary for $(uname -s)-$(uname -m)." >&2
     echo "Build instead:  cargo install --git https://github.com/$REPO ana" >&2
     exit 1 ;;
esac

ver="${1:-latest}"
if [ "$ver" = "latest" ]; then
  url="https://github.com/$REPO/releases/latest/download/ana-$target"
else
  url="https://github.com/$REPO/releases/download/$ver/ana-$target"
fi

echo "↓ $url"
curl -fsSL "$url" -o "$DEST/ana"
chmod +x "$DEST/ana"
echo "✓ installed $("$DEST/ana" --version) → $DEST/ana"
echo "  add to PATH if you like:  export PATH=\"\$HOME/.anamnesis/bin:\$PATH\""
