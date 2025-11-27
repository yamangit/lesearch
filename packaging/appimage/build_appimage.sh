#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../.. && pwd)"
cd "$ROOT_DIR"

if ! command -v appimagetool >/dev/null 2>&1; then
    echo "appimagetool is required to build the AppImage (https://github.com/AppImage/AppImageKit)" >&2
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
APPDIR="$ROOT_DIR/target/package/appimage/Lesearch.AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"

install -m755 target/release/les "$APPDIR/usr/bin/les"
install -m755 target/release/lesd "$APPDIR/usr/bin/lesd"
install -m644 packaging/appimage/utilities-terminal.png "$APPDIR/utilities-terminal.png"

cat >"$APPDIR/AppRun" <<'EOF'
#!/bin/sh
set -e
APPDIR="${APPDIR:-$(dirname "$(readlink -f "$0")")}"
if [ "$1" = "lesd" ]; then
    shift
    exec "$APPDIR/usr/bin/lesd" "$@"
fi
exec "$APPDIR/usr/bin/les" "$@"
EOF
chmod +x "$APPDIR/AppRun"

cat >"$APPDIR/lesearch.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=lesearch
Comment=Linux Everything-style search client
Exec=les %F
Icon=utilities-terminal
Terminal=true
Categories=Utility;System;
EOF

APPIMAGE_OUT="$ROOT_DIR/target/package/appimage/Lesearch-${VERSION}-$(uname -m).AppImage"
appimagetool "$APPDIR" "$APPIMAGE_OUT"
echo "Created $APPIMAGE_OUT"
