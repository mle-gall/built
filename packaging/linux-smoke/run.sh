#!/usr/bin/env bash
set -euo pipefail
export HOME="$(mktemp -d)" BUILT_SETTINGS_DIR="$(mktemp -d)" BUILT_BACKGROUND=1
export CFFIXED_USER_HOME="$HOME" XDG_RUNTIME_DIR="$(mktemp -d)" WGPU_BACKEND=vulkan
icds=(/usr/share/vulkan/icd.d/lvp_icd*.json)
export VK_ICD_FILENAMES="${icds[0]}"
fixture=/work/tests/fixtures/shell/slab-and-cube.igz
smoke() {
    local name=$1
    local dest; dest=$(mktemp -d)
    vulkaninfo --summary > "$dest/vulkan.log"
    built --check "$fixture" > "$dest/check.log"
    printf '%s\n' wait-open 'wait 300' 'click-command app-menu:Tools' 'wait 100' 'click-command app-item:tool:line' 'wait 150' 'move 400 300' 'capture frame' state quit > "$dest/steps"
    timeout 90 built --script "$dest/steps" --shots "$dest" "$fixture" > "$dest/session.log"
    /work/.venv/bin/python - "$dest" <<'PY'
import json, sys
from pathlib import Path
from PIL import Image
root = Path(sys.argv[1])
for filename, prefix in [('check.log', '[built] check ok '), ('session.log', '[built] state ')]:
    state = json.loads(next(line[len(prefix):] for line in (root / filename).read_text().splitlines() if line.startswith(prefix)))
    if filename == 'session.log':
        assert state['tool'] == 'LineTool', state
    assert Path(state['path']).name == 'slab-and-cube.igz' and not state['opening'], state
    assert (state['faces'], state['edges']) == (6, 16), state
    assert state['instances'] == 1 and state['errors'] == 0 and not state['qt_modules'], state
with Image.open(root / 'frame.png') as frame:
    assert frame.size == (1280, 800) and len(frame.convert('RGB').resize((64, 40)).getcolors(2560)) > 16
PY
    mkdir -p "/result/$name"
    cp "$dest"/{frame.png,check.log,session.log,vulkan.log} "/result/$name/"
}
if [[ $1 == x11-session ]]; then
    smoke x11
    exit
fi
if [[ $1 != wayland ]]; then
    unset WAYLAND_DISPLAY
    xvfb-run -a -s '-screen 0 1440x1000x24' bash "$0" x11-session
fi
if [[ $1 != x11 ]]; then
    unset DISPLAY
    export WAYLAND_DISPLAY=built-smoke
    weston --backend=headless-backend.so --socket="$WAYLAND_DISPLAY" --width=1440 --height=1000 --idle-time=0 --log=/result/weston.log &
    compositor=$!
    trap 'kill "$compositor" 2>/dev/null || true; wait "$compositor" || true' EXIT
    for _ in {1..100}; do [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] && break; sleep 0.1; done
    test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
    smoke wayland
fi
