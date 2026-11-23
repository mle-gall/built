#!/bin/sh
# Read back only the Rust render target, once through each host.
set -eu
cd "$(dirname "$0")/.."
root=$PWD
run=$(mktemp -d)
plist="$HOME/Library/Preferences/com.ingetrazo.IngeTrazo.plist"
stamp() { stat -f %m "$plist" 2>/dev/null || echo absent; }
before=$(stamp)
cleanup() {
    rm -rf "$run"
    [ "$(stamp)" = "$before" ] || { echo "Preferences mtime changed" >&2; exit 1; }
}
trap cleanup EXIT
out="${DS_OUT:-build/design-system}"
mkdir -p "$run/home" "$run/settings" "$out"
export HOME="$run/home" CFFIXED_USER_HOME="$run/home" XDG_CONFIG_HOME="$run/settings"
export BUILT_SETTINGS_DIR="$run/settings" BUILT_PASTEBOARD="gallery-$$" PYTHONDONTWRITEBYTECODE=1
.venv/bin/python scripts/design_system_capture.py gallery --dest "$out"
cat > "$run/steps" <<'EOF'
wait 1500
capture gallery-built
quit
EOF
"${BUILT_BIN:-$root/rust/built_app/target/release/built}" --gallery --new-instance \
    --settings "$run/settings" --root "$root" --venv "$root/.venv" --size 1280x800 \
    --script "$run/steps" --shots "$run"
mv "$run/gallery-built.png" "$out/${DS_PREFIX:-}gallery-built.png"
[ -z "${DS_PREFIX:-}" ] || mv "$out/gallery-qt.png" "$out/${DS_PREFIX}gallery-qt.png"
