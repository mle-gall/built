#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
#
# Build the IngeTrazo macOS artifacts: a double-clickable .app and a
# distributable .dmg wrapping it with a drag-to-Applications shortcut.
#
#     packaging/build-macos-app.sh [outdir]
#
# Needs: the project venv with the runtime deps (PyInstaller is installed
# on first run if missing). No Xcode project — PyInstaller freezes main.py
# straight from ingetrazo.spec, whose BUNDLE() step (macOS-only, see the
# bottom of the spec) produces the .app; `hdiutil`/`iconutil`, part of every
# macOS install, make the icon and the .dmg. (Sibling of
# packaging/build-appimage.sh; same shape, different OS.)
#
# Ad-hoc signed only (no Developer ID / notarization here): Gatekeeper will
# still show the "unidentified developer" prompt on a machine that isn't
# this one — right-click > Open the first time, same as the README already
# tells a source-build user.
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
OUT=${1:-$ROOT/dist}
WORK=${INGETRAZO_BUILD_DIR:-$ROOT/build}
PYTHON=${PYTHON:-$ROOT/venv/bin/python}
ARCH=$(uname -m)   # arm64 or x86_64 — whichever Mac builds it

cd "$ROOT"
VERSION=$("$PYTHON" -c 'from core.version import __version__; print(__version__)')
BUILD_APP="$WORK/pyi/Built.app"
APP="$OUT/Built.app"
DMG="$OUT/Built-$VERSION-macos-$ARCH.dmg"

echo "==> Built $VERSION -> $DMG"

echo "==> icon (.icns from the committed PNGs, via iconutil)"
# The Windows .ico is built by scripts/gen_app_icon.py + ImageMagick/Inkscape;
# .icns only needs macOS's own iconutil, so it is generated here rather than
# committed — one less binary in git, and every Mac already has the tool.
bash packaging/macos-icon.sh "$ROOT" "$WORK"

echo "==> PyInstaller"
"$PYTHON" -m PyInstaller --version >/dev/null 2>&1 \
    || "$PYTHON" -m pip install --quiet pyinstaller
rm -rf "$BUILD_APP" "$WORK/pyi"
"$PYTHON" -m PyInstaller --noconfirm \
    --distpath "$WORK/pyi" --workpath "$WORK/pyi-work" \
    ingetrazo.spec

echo "==> the bundle finds its own data"
# Before wrapping it in a .dmg, ask the bundle itself. A missing shader or
# texture starts fine and only fails when the user needs it.
"$PYTHON" scripts/isolated_launch.py "$BUILD_APP/Contents/MacOS/ingetrazo" --check

echo "==> .app"
mkdir -p "$OUT"
rm -rf "$APP"
cp -R "$BUILD_APP" "$APP"

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
