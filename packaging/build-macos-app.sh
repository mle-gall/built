#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
#
# Build the Built macOS artifacts: a double-clickable .app and a
# distributable .dmg wrapping it with a drag-to-Applications shortcut.
#
#     packaging/build-macos-app.sh [outdir]
#
# Uses the same native bundle builder as the installed macOS app.
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
OUT=${1:-$ROOT/dist}
WORK=${BUILT_BUILD_DIR:-${INGETRAZO_BUILD_DIR:-$ROOT/build}}
PYTHON=${PYTHON:-$ROOT/venv/bin/python}
ARCH=$(uname -m)   # arm64 or x86_64 — whichever Mac builds it

cd "$ROOT"
VERSION=$("$PYTHON" -c 'from built_core.version import __version__; print(__version__)')
APP="$OUT/Built.app"
DMG="$OUT/Built-$VERSION-macos-$ARCH.dmg"

echo "==> Built $VERSION -> $DMG"

PYO3_PYTHON="$(command -v "$PYTHON")" packaging/build-macos-native.sh "$OUT"

echo "==> .dmg"
# A staging folder with the .app plus a symlink to /Applications is what
# gives the mounted volume's Finder window its familiar drag-to-install
# layout.
STAGE="$WORK/dmg-stage"
rm -rf "$STAGE"; mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "Built $VERSION" -srcfolder "$STAGE" \
    -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"

printf '\n%s  (%s)\n' "$APP" "$(du -sh "$APP" | cut -f1)"
printf '%s  (%s)\n' "$DMG" "$(du -h "$DMG" | cut -f1)"
