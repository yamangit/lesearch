#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../.. && pwd)"
cd "$ROOT_DIR"

if ! command -v dpkg-deb >/dev/null 2>&1; then
    echo "dpkg-deb is required to build the .deb package" >&2
    exit 1
fi

cargo build --release

VERSION="$(python3 - <<'PY'
import json, subprocess
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--no-deps"]
))
root_pkg = next(p for p in meta["packages"] if p["name"] == "les")
print(root_pkg["version"])
PY
)"

ARCH="$(uname -m)"
case "$ARCH" in
    x86_64) ARCH=amd64 ;;
    aarch64) ARCH=arm64 ;;
    *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

PKG_DIR="$ROOT_DIR/target/package/deb/lesearch_${VERSION}_${ARCH}"
rm -rf "$PKG_DIR"
mkdir -p "$PKG_DIR/DEBIAN" \
         "$PKG_DIR/usr/bin" \
         "$PKG_DIR/usr/lib/systemd/system"

install -m755 target/release/les "$PKG_DIR/usr/bin/les"
install -m755 target/release/lesd "$PKG_DIR/usr/bin/lesd"
install -m644 packaging/systemd/lesd.service \
        "$PKG_DIR/usr/lib/systemd/system/lesd.service"

cat >"$PKG_DIR/DEBIAN/control" <<EOF
Package: lesearch
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Depends: libc6 (>= 2.31)
Maintainer: Lesearch Developers <you@example.com>
Description: Linux Everything-style search daemon and CLI
 Lesearch provides an Everything-style search daemon (lesd) and client (les)
 for instant file lookups on Linux.
EOF

DEB_OUT="$ROOT_DIR/target/package/lesearch_${VERSION}_${ARCH}.deb"
mkdir -p "$(dirname "$DEB_OUT")"
dpkg-deb --build "$PKG_DIR" "$DEB_OUT"
echo "Created $DEB_OUT"
