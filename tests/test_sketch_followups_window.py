# SPDX-License-Identifier: GPL-3.0-or-later
"""Sketch follow-ups in the native ``built`` shell, in a real window: Sweep's
D draws a profile and Done comes back to Sweep with it in the chip; a
member's catalogue size opens as a drawing and Done re-points the member in
one step (undone in one); a drawn member opens its own sketch; Draw › New
Section and the DWG import (with and without LibreDWG). Runs only with
``BUILT_WINDOW_TESTS=1`` and a built binary."""
import json
import os
import re
import shutil
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "rust" / "built_app" / "target" / "release" / "built"
DWG = ROOT / "tests" / "fixtures" / "sketch" / "plate-mm.dwg"

pytestmark = [
    pytest.mark.window,
    pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                       reason="opens a window: BUILT_WINDOW_TESTS=1 and the built binary"),
]


def _model(path):
    from built_core.history import History
    from built_core.protocol import commands
    from built_core.scene import Scene
    from formats import igz

    s = Scene()
    h = History(s)
    assert commands.run(h, "create_sweeps", sweeps=[
        dict(profile=dict(library="en/ipe240"), path=dict(points=[[0, 0, 0], [0, 3, 0]]))])["ok"]
    square = [[0, 0], [80, 0], [80, 80], [0, 80]]
    curves = [dict(id=i + 1, kind="line", a=square[i], b=square[(i + 1) % 4]) for i in range(4)]
    assert commands.run(h, "sketch_commit", row=dict(uid="5ce7c40000000201", use="profile", name="Post",
                                                      curves=curves))["ok"]
    assert commands.run(h, "create_sweeps", sweeps=[
        dict(profile=dict(sketch="5ce7c40000000201"), path=dict(points=[[2, 0, 0], [2, 3, 0]]))])["ok"]
    igz.save_scene(s, path)
    return [g.uid for g in s.groups]


def _states(log):
    return [json.loads(line.split(" ", 2)[2]) for line in log.splitlines() if line.startswith("[built] state ")]


def _launch(tmp_path, model, script, path_env=None):
    shots = tmp_path / "shots"
    shots.mkdir(exist_ok=True)
    steps = tmp_path / "script.txt"
    steps.write_text(script)
    sys.path.insert(0, str(ROOT / "scripts"))
    import isolated_launch
    env = dict(os.environ, BUILT_ANSWERS="escape")
    if path_env is not None:
        env["PATH"] = path_env
    result = isolated_launch.run(
        [str(BIN), "--script", str(steps), "--shots", str(shots), "--size", "1280x800", "--new-instance",
         "--settings", str(tmp_path / "settings"), str(model)],
        env=env, settings=tmp_path / "settings", capture_output=True, text=True, timeout=300)
    log = result.stdout + result.stderr
    assert not re.search(r"\[built\] error in (?!.*script)", log), log[-4000:]
    return _states(log), shots, log


def test_sweep_d_and_a_members_profile_round_trip(tmp_path):
    model = tmp_path / "members.built"
    ipe, post = _model(model)
    states, shots, log = _launch(tmp_path, model, f"""wait 1500
tool sweep
wait 200
key d
wait 500
state
type r
click 600 400
move 700 300
type 60;20
wait 150
capture 01-rectangle-typing
key enter
wait 200
key enter
wait 700
state
menu sketch:member:{ipe}
wait 600
capture 02-member-catalogue
state
key enter
wait 700
state
menu undo
wait 300
state
menu sketch:member:{post}
wait 600
state
menu sketch:cancel
wait 300
menu sketch:new:section
wait 500
state
menu sketch:cancel
wait 300
quit
""")
    assert len(states) >= 7, log[-4000:]
    drawing, back, catalogue, adopted, undone, drawn, section = states[:7]
    # Sweep › D: the workspace, then Done back in Sweep with the profile.
    assert drawing["sketch"]["shown"] and drawing["sketch"]["row"]["use"] == "profile"
    assert not back["sketch"]["shown"]
    new = [r for r in back["sketch_rows"] if r["uid"] != "5ce7c40000000201"]
    assert len(new) == 1 and len(new[0]["curves"]) == 4
    assert back["sweep_chip"] and "Drawn profile" in back["sweep_chip"], back["sweep_chip"]
    # The IPE member's size as an exact drawing; Done re-points the member.
    assert catalogue["sketch"]["shown"]
    kinds = sorted(c["kind"] for c in catalogue["sketch"]["row"]["curves"])
    assert kinds.count("arc") == 4 and kinds.count("line") == 12
    member = next(r for r in adopted["sweep_rows"] if r["owner"] == ipe)
    assert "sketch" in member["profile"] and member["state"] == "ok"
    assert member["profile"]["sketch"] in {r["uid"] for r in adopted["sketch_rows"]}
    # One undo puts the catalogue size back and takes the drawing away.
    member = next(r for r in undone["sweep_rows"] if r["owner"] == ipe)
    assert member["profile"].get("library") == "en/ipe240"
    assert len(undone["sketch_rows"]) == len(adopted["sketch_rows"]) - 1
    # A drawn member opens its own sketch row.
    assert drawn["sketch"]["shown"] and drawn["sketch"]["row"]["name"] == "Post"
    assert section["sketch"]["shown"] and section["sketch"]["row"]["use"] == "section"
    assert (shots / "01-rectangle-typing.png").stat().st_size > 10_000


def _converter_dir():
    found = shutil.which("dwg2dxf") or os.environ.get("BUILT_TEST_DWG2DXF")
    return str(Path(found).parent) if found else None


def test_dwg_import_with_and_without_libredwg(tmp_path):
    model = tmp_path / "members.built"
    _model(model)
    script = f"""wait 1500
menu sketch:import:{DWG}
wait 800
state
menu sketch:cancel
wait 300
quit
"""
    # Without the converter: a clear message, the workspace stays shut.
    bare = tmp_path / "bare"
    bare.mkdir()
    (state, *_), _, log = _launch(bare, model, script, path_env="/usr/bin:/bin")
    assert not state["sketch"]["shown"]
    folder = _converter_dir()
    if folder is None:
        pytest.skip("LibreDWG's dwg2dxf is not installed")
    both = script.replace("quit\n", f"""menu sketch:import:{DWG.with_suffix('.dxf')}
wait 800
state
menu sketch:cancel
wait 300
quit
""")
    (dwg, dxf, *_), _, log = _launch(tmp_path, model, both, path_env=f"{folder}:/usr/bin:/bin")
    assert dwg["sketch"]["shown"], log[-3000:]
    # The DWG reads as the DXF it was written from: the same curves.
    def rounded(v):
        if isinstance(v, float):
            return round(v, 6)
        return [rounded(x) for x in v] if isinstance(v, list) else v

    def shapes(state):
        return sorted(json.dumps({k: rounded(v) for k, v in c.items() if k != "id"}, sort_keys=True)
                      for c in state["sketch"]["row"]["curves"])
    assert {c["kind"] for c in dwg["sketch"]["row"]["curves"]} >= {"line", "arc"}
    assert shapes(dwg) == shapes(dxf)


def test_double_click_on_a_section_face_opens_its_profile(tmp_path):
    """Seen from the front, both members show their section faces: a
    double-click on the drawn Post opens its sketch row; one on the IPE's
    flange opens the catalogue size as a drawing (lines and four arcs)."""
    model = tmp_path / "members.built"
    _model(model)
    states, shots, log = _launch(tmp_path, model, """wait 1500
menu view_front
wait 400
menu zoom_extents
wait 800
double 893 402
wait 800
capture 01-post
state
menu sketch:cancel
wait 300
double 183 361
wait 800
capture 02-ipe
state
menu sketch:cancel
wait 300
quit
""")
    assert len(states) >= 2, log[-4000:]
    post, ipe = states[:2]
    assert post["sketch"]["shown"] and post["sketch"]["row"]["name"] == "Post", log[-3000:]
    assert ipe["sketch"]["shown"], log[-3000:]
    kinds = sorted(c["kind"] for c in ipe["sketch"]["row"]["curves"])
    assert kinds.count("arc") == 4 and kinds.count("line") == 12
