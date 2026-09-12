#!/usr/bin/env bash
# Fetch the prebuilt `ana` engine for this platform into ~/.anamnesis/bin, where
# the Anamnesis plugin's hooks look for it.
#
#   bash install-ana.sh [version]      # version defaults to "latest"
#
# The download is verified against the release's sha256.sum and FAILS CLOSED: if
# the checksum file is missing, or does not match, nothing is installed. A tool
# whose whole subject is not fooling yourself should not install an unverified
# binary because verification was inconvenient.
set -euo pipefail

REPO="${ANAMNESIS_REPO:-Anbu-00001/Anamnesis}"
DEST="$HOME/.anamnesis/bin"

# The package is `anamnesis`; `ana` is only the binary's name, so naming `ana`
# here fails with "package ID specification `ana` did not match any packages".
BUILD_HINT="cargo install --git https://github.com/$REPO --locked"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)   target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64)  target=aarch64-unknown-linux-gnu ;;
  Darwin-x86_64)  target=x86_64-apple-darwin ;;
  Darwin-arm64)   target=aarch64-apple-darwin ;;
  *) echo "No prebuilt binary for $(uname -s)-$(uname -m)." >&2
     echo "Build instead:  $BUILD_HINT" >&2
     exit 1 ;;
esac

for tool in curl sha256sum; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    if [ "$tool" = "sha256sum" ] && command -v shasum >/dev/null 2>&1; then
      continue   # macOS ships `shasum -a 256` instead
    fi
    echo "missing required tool: $tool" >&2
    echo "Build instead:  $BUILD_HINT" >&2
    exit 1
  fi
done

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

ver="${1:-latest}"
# ANAMNESIS_RELEASE_BASE lets the test suite point this at a local directory;
# unset, it is the real release URL.
if [ -n "${ANAMNESIS_RELEASE_BASE:-}" ]; then
  base="$ANAMNESIS_RELEASE_BASE"
elif [ "$ver" = "latest" ]; then
  base="https://github.com/$REPO/releases/latest/download"
else
  base="https://github.com/$REPO/releases/download/$ver"
fi
asset="ana-$target"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "↓ $base/$asset"
if ! curl -fsSL "$base/$asset" -o "$tmp/ana"; then
  echo "Download failed. Build instead:  $BUILD_HINT" >&2
  exit 1
fi

echo "↓ $base/sha256.sum"
if ! curl -fsSL "$base/sha256.sum" -o "$tmp/sha256.sum"; then
  echo "No sha256.sum in this release — refusing to install an unverified binary." >&2
  echo "Build instead:  $BUILD_HINT" >&2
  exit 1
fi

want="$(grep -E "[ *]$asset\$" "$tmp/sha256.sum" | head -n1 | cut -d' ' -f1 || true)"
if [ -z "$want" ]; then
  echo "sha256.sum does not list $asset — refusing to install." >&2
  exit 1
fi
got="$(sha256_of "$tmp/ana")"
if [ "$want" != "$got" ]; then
  echo "CHECKSUM MISMATCH for $asset" >&2
  echo "  expected $want" >&2
  echo "  got      $got" >&2
  echo "Nothing was installed." >&2
  exit 1
fi
echo "✓ sha256 verified"

mkdir -p "$DEST"
mv "$tmp/ana" "$DEST/ana"
chmod +x "$DEST/ana"
echo "✓ installed $("$DEST/ana" --version) → $DEST/ana"
echo "  add to PATH if you like:  export PATH=\"\$HOME/.anamnesis/bin:\$PATH\""
