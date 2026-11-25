"""A zoom gesture does not hover once per event.

macOS sends a pointer move with every scroll event, and a hover picks and
snaps what is under the cursor: 24 ms a move on a five-million-face model,
after every event of a trackpad zoom (owner, 2026-11-25, ride file: "zoom
is smooth right after a document loads, then becomes very laggy"). The
shell hands the window the newest pointer position once the wheel, slide
or pinch is over, and at once to anything that reads the hover.
"""
import json
import os
from pathlib import Path
import subprocess

import pytest


def _lines(stdout, tag):
    return [json.loads(line.removeprefix(f"[built] {tag} ")) for line in stdout.splitlines()
            if line.startswith(f"[built] {tag} ")]


def _moves(block):
    return block.get("move_ms", {"n": 0})["n"]


def test_pointer_moves_wait_for_the_end_of_a_zoom(tmp_path):
    root = Path(__file__).resolve().parents[1]
    binary = Path(os.environ.get("BUILT_BINARY", root / "rust/built_app/target/debug/built"))
    if not binary.is_file():
        pytest.skip("build the native built_app binary for the windowed smoke test")
    model = root / "tests/fixtures/shell/slab-and-cube.igz"

    def zoom(event):
        out = []
        for i in range(30):
            x, y = 900 + i % 3, 430 + i % 2
            out += [f"move {x} {y}", event.format(x=x, y=y, n=-1 if i < 15 else 1)]
        return out

    lines = ["wait 600", "menu view:zoom_extents", "tool line", "move 900 430", "wait 300", "state"]
    lines += zoom("notch {x} {y} {n}") + ["state", "wait 300"]                        # read at once
    lines += zoom("pinch {x} {y} {n}") + ["wait 400", "state"]                        # read after the pause
    lines += zoom("wheel {x} {y} {n} move") + ["click 902 431", "state", "wait 300"]   # a press reads the hover
    lines += ["move 910 440", "move 920 450", "move 930 460", "state", "quit"]
    steps = tmp_path / "steps"
    steps.write_text("\n".join(lines) + "\n")
    home, settings = tmp_path / "home", tmp_path / "settings"
    home.mkdir()
    settings.mkdir()
    result = subprocess.run([binary, "--root", root, "--venv", root / ".venv", "--new-instance",
                             "--script", steps, "--size", "1600x1000", model],
                            env=dict(os.environ, HOME=str(home), CFFIXED_USER_HOME=str(home),
                                     BUILT_SETTINGS_DIR=str(settings), BUILT_PERF="1"),
                            capture_output=True, text=True, timeout=90, check=True)
    states = _lines(result.stdout, "state")
    perf = _lines(result.stdout, "perf")
    assert all(s["errors"] == 0 for s in states), result.stderr
    still, notches, pinches, slide, plain = perf[:5]
    assert _moves(still) == 1
    # Thirty moves between thirty zoom events: the move before the first
    # event hovers, and the last position once the zoom is read or over.
    assert notches["wheel_ms"]["n"] == 30 and _moves(notches) == 2
    assert pinches["gesture_ms"]["n"] >= 30 and _moves(pinches) == 2
    # The click moves the pointer to its own position before it presses.
    assert _moves(slide) == 3
    # Away from a zoom every scripted move is its own hover.
    assert _moves(plain) == 3
    # The zooms changed the view and came back: fifteen steps out, fifteen in.
    assert states[1]["camera_distance"] == pytest.approx(states[0]["camera_distance"], rel=1e-6)
