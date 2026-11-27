# SPDX-License-Identifier: GPL-3.0-or-later
"""The Sketch workspace in the native ``built`` shell, in a real window:
enter a section sketch on a member's end face, draw with typed values, undo
and redo inside the sketch, Done as one document step, undo it from the
model, leave a second sketch with Esc Esc and Discard. Runs only with
``BUILT_WINDOW_TESTS=1`` and a built binary."""
import json
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "rust" / "built_app" / "target" / "release" / "built"

pytestmark = [
    pytest.mark.window,
    pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1" or not BIN.exists(),
                       reason="opens a window: BUILT_WINDOW_TESTS=1 and the built binary"),
]


def _model(path):
    from built_core import Matrix4, Vec3
    from built_core.group import Group
    from built_core.mesh import Mesh
    from built_core.scene import Scene
    from formats import igz
    from tests.test_persistent_ids import face_id

    m = Mesh()
    (x0, y0, z0), (x1, y1, z1) = (0, 0, 0), (0.15, 3.0, 0.15)
    v = Vec3
    for ring in ([v(x0, y0, z0), v(x0, y1, z0), v(x1, y1, z0), v(x1, y0, z0)],
                 [v(x0, y0, z1), v(x1, y0, z1), v(x1, y1, z1), v(x0, y1, z1)],
                 [v(x0, y0, z0), v(x1, y0, z0), v(x1, y0, z1), v(x0, y0, z1)],
                 [v(x0, y1, z0), v(x0, y1, z1), v(x1, y1, z1), v(x1, y1, z0)],
                 [v(x0, y0, z0), v(x0, y0, z1), v(x0, y1, z1), v(x0, y1, z0)],
                 [v(x1, y0, z0), v(x1, y1, z0), v(x1, y1, z1), v(x1, y0, z1)]):
        m.add_face(ring)
    s = Scene()
    g = Group(m, "P-1")
    g.xform = Matrix4()
    s.groups.append(g)
    igz.save_scene(s, path)
    back = Scene()
    igz.load_into(back, path)
    p = back.groups[0]
    end = next(f for f in p.mesh.faces if f.normal().y() < -0.9)
    return p.uid, face_id(p.mesh, end)


def _states(log):
    return [json.loads(line.split(" ", 2)[2]) for line in log.splitlines() if line.startswith("[built] state ")]


def test_sketch_on_a_face_done_undo_and_discard(tmp_path):
    model = tmp_path / "purlin.built"
    uid, face = _model(model)
    shots = tmp_path / "shots"
    shots.mkdir()
    script = tmp_path / "script.txt"
    script.write_text(f"""wait 1500
menu sketch:section:{uid}|{face}
wait 600
capture 01-entered
state
type r
click 600 400
move 700 300
type 60;20
key enter
wait 200
capture 02-rectangle
state
key z cmd
wait 100
state
menu redo
wait 100
state
key enter
wait 600
capture 03-done
state
menu undo
wait 300
state
menu sketch:new
wait 300
key escape
wait 200
capture 04-leave-prompt
key escape
wait 100
state
key escape
wait 100
menu sketch:cancel
wait 300
menu redo
wait 300
state
quit
""")
    sys.path.insert(0, str(ROOT / "scripts"))
    import isolated_launch
    result = isolated_launch.run(
        [str(BIN), "--script", str(script), "--shots", str(shots), "--size", "1280x800", "--new-instance",
         "--settings", str(tmp_path / "settings"), str(model)],
        env=dict(os.environ, BUILT_ANSWERS="escape"), settings=tmp_path / "settings",
        capture_output=True, text=True, timeout=300)
    log = result.stdout + result.stderr
    # One state per `state` step (the shell adds one more on quit).
    states = _states(log)
    assert len(states) >= 8, log[-4000:]
    entered, drawn, undone, redone, done, doc_undo, prompt, doc_redo = states[:8]
    summary = [(s["sketch"]["shown"], len((s["sketch"]["row"] or {}).get("curves", [])),
                [len(r["curves"]) for r in s["sketch_rows"]]) for s in states]
    assert entered["sketch"]["shown"] and entered["sketch"]["row"]["use"] == "section"
    assert entered["sketch"]["row"]["plane"]["ref"] == {"path": [uid], "id": face}
    assert len(drawn["sketch"]["row"]["curves"]) == 4, summary
    assert len(undone["sketch"]["row"]["curves"]) == 0
    assert len(redone["sketch"]["row"]["curves"]) == 4
    # Done: one sketch row, out of the workspace; the rectangle is 60 x 20.
    assert not done["sketch"]["shown"]
    (row,) = done["sketch_rows"]
    assert row["state"] == "ok" and len(row["curves"]) == 4
    xs = sorted({round(c["a"][0], 6) for c in row["curves"]})
    ys = sorted({round(c["a"][1], 6) for c in row["curves"]})
    assert round(xs[-1] - xs[0], 6) == 60 and round(ys[-1] - ys[0], 6) == 20
    # The model's undo takes the whole sketch away in one step.
    assert doc_undo["sketch_rows"] == []
    # Esc with nothing to drop asks to leave; Esc again stays.
    assert prompt["sketch"]["shown"]
    assert len(doc_redo["sketch_rows"]) == 1
    for name in ("01-entered", "02-rectangle", "03-done", "04-leave-prompt"):
        assert (shots / f"{name}.png").stat().st_size > 10_000, name
    assert not re.search(r"\[built\] error in (?!.*script)", log), log[-4000:]
