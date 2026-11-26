#!/bin/bash
# Build with a prepared venv containing PyInstaller, the runtime wheels,
# ifcopenshell and the three extensions installed by scripts/build_native.sh.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${1:-"$ROOT/dist"}
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
cd "$ROOT"
WORK="$ROOT/build/native"
PYTHON=${PYO3_PYTHON:-"$ROOT/.venv/bin/python"}
export PYO3_PYTHON="$PYTHON"
mkdir -p "$WORK"
STAGE=$(mktemp -d "$OUT/.native-XXXXXXXX")
trap 'rm -rf "$STAGE"' EXIT
PROFILE="$STAGE/profile"
mkdir -p "$PROFILE/settings"
isolated() {
    HOME="$PROFILE" CFFIXED_USER_HOME="$PROFILE" XDG_CONFIG_HOME="$PROFILE/settings" \
        BUILT_SETTINGS_DIR="$PROFILE/settings" BUILT_BACKGROUND=1 "$@"
}

isolated "$PYTHON" -c 'import PyInstaller, ifcopenshell, built_core, built_render'
if [[ -d "$ROOT/../rvt-rs" ]]; then
    isolated "$PYTHON" -c 'import built_rfa' || {
        echo "The sibling rvt-rs checkout is present. Run scripts/build_native.sh to install its reader." >&2
        exit 1
    }
    export BUILT_PACKAGE_RFA=1
else
    echo "Warning: no sibling rvt-rs checkout. This package will not support .rfa import." >&2
    export BUILT_PACKAGE_RFA=0
fi
bash packaging/macos-icon.sh "$ROOT" "$WORK"
# BUILT_CARGO_TARGET_DIR: build on a target directory that already holds the
# launcher's dependencies (CI builds the launcher once for its tests).
export CARGO_TARGET_DIR="${BUILT_CARGO_TARGET_DIR:-$WORK/cargo}"
BUILT_MACOS_BUNDLE=1 cargo build --locked --release --manifest-path rust/built_app/Cargo.toml
PYTHONPATH="$ROOT" isolated "$PYTHON" -m PyInstaller --clean --noconfirm \
    --workpath "$WORK/work" --distpath "$WORK/collect" packaging/native.spec

APP="$STAGE/Built.app"
cp -R "$WORK/collect/Built.app" "$APP"
EXE="$APP/Contents/MacOS/built"
cp "$CARGO_TARGET_DIR/release/built" "$EXE"
LIB=$(otool -L "$EXE" | awk '/Python.framework.*\/Python \(/{print $1}')
test -n "$LIB" && test -f "$APP/Contents/Frameworks/Python" || {
    echo "Missing Python.framework dependency or collected library" >&2; exit 1;
}
install_name_tool -change "$LIB" '@executable_path/../Frameworks/Python' "$EXE"
/usr/libexec/PlistBuddy -c 'Set :CFBundleExecutable built' "$APP/Contents/Info.plist"
# The collector supplies the embedded Python libraries. The signed product
# has a single executable entry point.
codesign -f -s - "$APP"
codesign --verify --deep --strict "$APP"
BUILT_BACKGROUND=1 "$PYTHON" scripts/isolated_launch.py "$EXE" --check
codesign --verify --deep --strict "$APP"
if [[ -e "$OUT/Built.app" ]]; then mv "$OUT/Built.app" "$STAGE/previous.app"; fi
mv "$APP" "$OUT/Built.app" || {
    [[ ! -e "$STAGE/previous.app" ]] || mv "$STAGE/previous.app" "$OUT/Built.app"
    exit 1
}
du -sh "$OUT/Built.app"
