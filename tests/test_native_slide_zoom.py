"""A trackpad slide zooms by the finger's travel, however slowly it moves.

Cocoa hands a slow slide and the inertia after it over in fractions of a
pixel. The viewport reads whole pixels, and rounding each event alone
dropped the deltas under a quarter pixel and zoomed the ones between a
quarter and a half pixel backwards: the zoom crawled, stalled and bounced
at the end of the inertia (the "zoom is laggy and slow" report). The shell
carries the fraction (`gestures::carry`), so the same travel zooms the same
amount at any speed, and a sub-pixel event costs no Python call.
"""
import json
import os
from pathlib import Path
import subprocess

import pytest


def _states(stdout, tag):
    return [json.loads(line.removeprefix(f"[built] {tag} ")) for line in stdout.splitlines()
            if line.startswith(f"[built] {tag} ")]


def test_slow_slide_and_inertia_zoom_like_a_fast_slide(tmp_path):
    root = Path(__file__).resolve().parents[1]
    binary = Path(os.environ.get("BUILT_BINARY", root / "rust/built_app/target/debug/built"))
    if not binary.is_file():
        pytest.skip("build the native built_app binary for the windowed smoke test")
    model = root / "tests/fixtures/shell/slab-and-cube.igz"
    x, y = 930, 430   # off the centre: the zoom heads for the point under the cursor
    # The same 40 px of travel three ways: fast, slow, and as an inertia tail.
    fast = [f"wheel {x} {y} -4 move"] * 10
    slow = [f"wheel {x} {y} -0.4 move"] * 100
    tail = [f"wheel {x} {y} -0.3 momentum"] * 133 + [f"wheel {x} {y} -0.1 momentum"]
    lines = ["wait 600", "menu view:zoom_extents", "wait 100", "state"]
    for travel in (fast, slow, tail):
        start = [f"wheel {x} {y} 0 start"]
        end = [] if travel is tail else [f"wheel {x} {y} 0 end"]
        lines += start + travel + end + ["state"]
    lines += ["menu view:zoom_extents", "wait 100", "state", "quit"]
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
    states = _states(result.stdout, "state")
    perf = _states(result.stdout, "perf")
    assert all(s["errors"] == 0 for s in states), result.stderr
    home_view, after_fast, after_slow, after_tail, extents = states[:5]
    d0, d1, d2, d3 = (s["camera_distance"] for s in (home_view, after_fast, after_slow, after_tail))
    # Each travel zooms IN (Natural Scrolling: fingers up), by the same factor.
    assert d1 < d0 and d2 < d1 and d3 < d2
    assert d2 / d1 == pytest.approx(d1 / d0, rel=0.01)
    assert d3 / d2 == pytest.approx(d1 / d0, rel=0.01)
    # A zoom does not turn the view, and off the centre the eye heads for
    # the point under the cursor, not along the view to the target.
    assert after_fast["camera_forward"] == pytest.approx(home_view["camera_forward"])
    moved = [a - b for a, b in zip(after_fast["camera_eye"], home_view["camera_eye"])]
    along = sum(m * f for m, f in zip(moved, home_view["camera_forward"]))
    assert along > 0 and along < 0.999 * sum(m * m for m in moved) ** 0.5
    # Zoom Extents frames the model as it did before the zooms.
    assert extents["camera_distance"] == pytest.approx(d0, rel=1e-6)
    assert extents["camera_eye"] == pytest.approx(home_view["camera_eye"], rel=1e-6, abs=1e-6)
    # Operation count: only whole pixels reach Python. The slow slide's 100
    # events and the tail's 134 hand over one call per pixel of travel, plus
    # the opening and closing phases.
    calls = perf[2]["wheel_ms"]["n"] + perf[3]["wheel_ms"]["n"]
    assert calls <= 2 * 40 + 4, perf
