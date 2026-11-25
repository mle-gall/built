#!/bin/sh
# The Sheets workspace in the native `built` shell, in a real window: open a
# document with a sheet, switch to Sheets from the top bar's workspace ▾,
# select and drag on the page, zoom, export the sheet as PDF, switch back
# and forth, back to the model. Frame captures in build/sheets-shots/shell/.
#
#   scripts/sheets_shell_smoke.sh
#
# Throwaway HOME and settings; the user's preferences plist must keep its
# mtime (checked at the end). The window closes by itself.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${BUILT_BIN:-$ROOT/rust/built_app/target/release/built}
SHOTS=${SHOTS:-$ROOT/build/sheets-shots/shell}
RUN=$(mktemp -d "${TMPDIR:-/tmp}/built-sheets.XXXXXX")
PLIST="$HOME/Library/Preferences/com.built.Built.plist"
BEFORE=$(stat -f '%m' "$PLIST" 2>/dev/null || echo none)
mkdir -p "$RUN/home" "$RUN/settings" "$SHOTS"
cp "$ROOT/examples/arco-yanque.igz" "$RUN/"
cat > "$RUN/script.txt" <<STEPS
wait 600
capture 01-model
menu sheets
wait 1500
capture 02-sheets
click 1200 120
wait 300
capture 03-dock-row
click 520 560
wait 300
capture 04-selected
drag 520 560 560 560 left 6
wait 300
key z cmd
wait 300
notch 640 400 1
wait 200
pinch 640 400 0.3
wait 300
capture 05-zoomed
key escape
menu sheet_export_to:pdf:$RUN/sheet.pdf
wait 500
capture 06-exported
menu model
wait 500
menu sheets
wait 500
menu model
wait 500
menu sheets
wait 800
capture 07-sheets-again
key tab ctrl
wait 800
capture 08-model
state
quit
STEPS
# BUILT_ANSWERS: AppKit also offers the --script file to the app as a
# document to open (an "open failed" sheet comes up first); Esc answers it.
BUILT_PASTEBOARD=built-sheets-smoke BUILT_ANSWERS=escape BUILT_SHOTS=$SHOTS \
    "${PYTHON:-$ROOT/.venv/bin/python}" "$ROOT/scripts/isolated_launch.py" "$BIN" --settings "$RUN/settings" --shots "$SHOTS" --size 1280x800 --new-instance \
    --script "$RUN/script.txt" "$RUN/arco-yanque.igz" > "$RUN/log.txt" 2>&1
grep '^\[built\] \(state\|capture\|error\|opened\)' "$RUN/log.txt" || true
echo "log: $RUN/log.txt"
test -s "$RUN/sheet.pdf" && echo "pdf: $(wc -c < "$RUN/sheet.pdf") bytes" || echo "pdf: MISSING"
AFTER=$(stat -f '%m' "$PLIST" 2>/dev/null || echo none)
[ "$BEFORE" = "$AFTER" ] && echo "plist untouched" || { echo "PLIST CHANGED ($BEFORE -> $AFTER)" >&2; exit 1; }
