# SPDX-License-Identifier: GPL-3.0-or-later
"""The demo-recording kit: beats compile to the app's own script steps with
repeatable human timing and play through the real input path; the
keystroke overlay shows what is played; V looks straight at a face."""
from __future__ import annotations

import json
import math
import os
from pathlib import Path

import pytest

from scripts.play_beat import BeatError, compile_beat, play

ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = ROOT / "docs/beats/box-and-cylinder.beat"
SCRIPT_VERBS = {"open", "wait", "move", "press", "release", "click", "key", "type", "menu",
                "notch", "capture", "state", "quit"}


def test_the_example_beat_compiles_to_script_steps():
    beat = compile_beat(EXAMPLE.read_text())
    assert beat.size == (1600, 1000)
    verbs = {s.split()[0] for s in beat.steps}
    assert verbs <= SCRIPT_VERBS, verbs - SCRIPT_VERBS
    # A blank model, then the overlay on, before the first key.
    assert beat.steps.index("key enter") < beat.steps.index("menu show_keystrokes") < beat.steps.index("key r")
    # Typing is one character at a time with a pause between.
    first = beat.steps.index("type 4")
    assert beat.steps[first + 1].startswith("wait ") and beat.steps[first + 2] == "type ;"
    # The pointer travels: many small moves between the clicks, not a jump.
    moves = [s for s in beat.steps if s.startswith("move ")]
    assert len(moves) > 100
    # The orbit is a middle-button drag.
    assert any(s.startswith("press ") and s.endswith(" middle") for s in beat.steps)


def test_a_beat_plays_the_same_every_time_and_a_seed_gives_another_take():
    text = EXAMPLE.read_text()
    assert compile_beat(text).steps == compile_beat(text).steps
    assert compile_beat(text, seed=3).steps != compile_beat(text).steps


def test_a_beat_with_a_mistake_says_where():
    with pytest.raises(BeatError, match="line 2"):
        compile_beat("pause 10\njump 1 2\n")
    with pytest.raises(BeatError):
        compile_beat("type 4 3\n")
    beat = compile_beat("model examples/a.built\nkeystrokes off\nkey r\n")
    assert "key enter" not in beat.steps and "menu show_keystrokes" not in beat.steps


def _states(stdout: str):
    return [json.loads(line.split(" ", 2)[2]) for line in stdout.splitlines()
            if line.startswith("[built] state ")]


def _binary():
    binary = Path(os.environ.get("BUILT_BINARY", ROOT / "rust/built_app/target/release/built"))
    if os.environ.get("BUILT_WINDOW_TESTS") != "1" or not binary.is_file():
        pytest.skip("requires the native window")
    return binary


@pytest.mark.window_size(1600, 1000)
def test_the_example_beat_draws_through_the_window_and_the_overlay_shows_it(tmp_path):
    binary = _binary()
    text = EXAMPLE.read_text().replace("key r                       # Rectangle",
                                       "key r\nstep state", 1)
    beat = compile_beat(text + "\nstep state\n")
    beat.steps = [s for s in beat.steps if not s.startswith("capture ")]
    result = play(beat, background=True, shots=None, quit_at_end=True, binary=binary, timeout=240)
    assert result.returncode == 0, result.stderr[-2000:]
    states = _states(result.stdout)
    armed, final = states[0], states[-1]
    assert armed["tool"] == "RectangleTool"
    assert ["R", "Rectangle", False] in armed["keystrokes"]
    # A box and a cylinder: the box's 6 faces less the covered top plus
    # the cylinder's sides and cap, all drawn by the tools.
    assert final["faces"] > 20 and final["errors"] == 0, final
    assert ["⇧Z", "", False] in final["keystrokes"] or final["keystrokes"] == []


@pytest.mark.window_size(1600, 1000)
def test_v_looks_straight_at_the_face_under_the_cursor(tmp_path):
    binary = _binary()
    beat = compile_beat("""
key r
click 640 520
point 820 600
type 4;3
key enter
key p
click 760 540
point 760 420
type 2.5
key enter
key space
point 700 500
step state
key v
pause 900
step state
""")
    result = play(beat, background=True, shots=None, quit_at_end=True, binary=binary, timeout=240)
    assert result.returncode == 0, result.stderr[-2000:]
    before, after = _states(result.stdout)[:2]     # (quit prints one more)
    assert before["camera_perspective"] and not after["camera_perspective"]
    fx, fy, fz = after["camera_forward"]
    # Square to a wall: level, along one of the box's horizontal axes.
    assert abs(fz) < 1e-6 and max(abs(fx), abs(fy)) == pytest.approx(1.0, abs=1e-6)
    # The world's up stays up on screen.
    assert after["camera_up_vector"][2] == pytest.approx(1.0, abs=1e-6)
    # The point under the cursor comes to the centre at the scale it had
    # (the distance becomes its depth).
    assert after["camera_distance"] < before["camera_distance"]
    assert ["V", "", False] in after["keystrokes"]
    assert not math.isnan(after["camera_distance"])
