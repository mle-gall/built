# SPDX-License-Identifier: GPL-3.0-or-later
"""Measure from dispatch through the first rendered frame in a release launcher."""
import inspect
import json
import os
from pathlib import Path
import zipfile

import pytest

from scripts.isolated_launch import run

ROOT = Path(__file__).resolve().parents[1]
BUDGET_MS = {"open": 200, "components": 100, "materials": 100, "close": 50,
             "first_press": 50, "line_commit": 150, "erase_commit": 150}


def large_document(folder):
    """3,000 distinct, painted 26x26 grids in four-deep assemblies, plus two links."""
    from built_core import Matrix4, Vec3 as V, links
    from built_core.group import Group
    from built_core.scene import Scene
    from built_core.materials import Material
    from formats import igz
    from tests.raster_samples import solid_raster

    folder.mkdir(exist_ok=True)
    result = folder / "assembly.built"
    seed = Scene()
    seed.mesh.add_face([V(-6,-6,0), V(-4,-6,0), V(-4,-4,0), V(-6,-4,0)])
    texture = folder / "grid.png"
    solid_raster(4, 4, (45, 55, 65)).save(str(texture))
    for i in range(3000):
        name = f"Finish {i:04}"
        mat = Material(name, color=(.3, .4, .5))
        if i % 150 == 0:
            mat.texture = {"path": str(texture), "sw": 1., "sh": 1.}
        seed.materials[name] = mat
    grid = Group(name="Grid")
    grid.xform = Matrix4()
    for y in range(26):
        for x in range(26):
            f = grid.mesh.add_face([V(x,y,0), V(x+1,y,0), V(x+1,y+1,0), V(x,y+1,0)])
            f.attrs["mat"] = "Finish 0000"
    seed.groups.append(grid)
    for i in range(2):
        linked = Scene()
        linked.groups.append(Group(name=f"Linked part {i}"))
        name = f"Linked finish {i}"
        linked.materials[name] = Material(name, color=(.7, .3, .2))
        linked.groups[0].mesh.add_face([V(0,0,0), V(1,0,0), V(0,1,0)]).attrs["mat"] = name
        source = folder / f"source-{i}.built"
        igz.save_scene(linked, source)
        seed.groups.append(links.make_link(seed, source, host_path=result))
    path = folder / "seed.built"
    igz.save_scene(seed, path)
    with zipfile.ZipFile(path) as src, zipfile.ZipFile(result, "w", zipfile.ZIP_DEFLATED, compresslevel=1) as dst:
        doc = json.loads(src.read("document.json"))
        scene = doc["scene"]
        base = scene["groups"].pop(0)
        template = scene["protos"][base["proto"]]
        block = src.read(template["geom"]["member"])
        for member in src.namelist():
            if member != "document.json":
                dst.writestr(member, src.read(member))
        # Reuse the encoded array layout. Each definition loads its own mesh.
        for i in range(3000):
            proto = json.loads(json.dumps(template).replace("Finish 0000", f"Finish {i:04}"))
            member = f"geom/grid-{i}.bin"
            proto["geom"]["member"] = member
            dst.writestr(member, block)
            placement = dict(base, proto=len(scene["protos"]), uid=f"{i+1:016x}", name=f"Part {i:04}")
            scene["protos"].append(proto)
            if i % 4:
                parent["children"] = [placement]
            else:
                scene["groups"].append(placement)
                placement["xform"] = list(placement["xform"])
                placement["xform"][12:15] = [40 + 28*(i//4%30), 40 + 28*(i//120), 0]
            parent = placement
        dst.writestr("document.json", json.dumps(doc))
    return result


def setup(app):
    """Temporary test plugin, executed by the release binary's script runner."""
    import json
    from pathlib import Path
    from time import perf_counter
    from built_render import chrome_sheet as sheet, wgpu_library as library

    from built_core import Vec3 as V
    win = app.window
    values = {}

    def frame():
        win._host.frame()

    def timed(key, action):
        start = perf_counter()
        action()
        frame()
        values[key] = (perf_counter() - start) * 1000

    def tab(name):
        sheet.action(win, sheet.SEP.join(("library", "tab", name)))
        assert library._state(win._host.view)["type"] == name

    def click(x):
        win._host.pointer("press", x, 480, 1, 1)
        win._host.pointer("release", x, 480, 1)

    def start():
        assert win._host.state()["instances"] >= 3000 and len(app.scene.materials) >= 3000
        app.viewport.camera.set_view("top")
        app.viewport.camera.fit_to(V(-8,-8,0), V(8,8,0))
        frame()

    def open_sheet():
        timed("open", lambda: library.open_library(win, "materials"))
        assert sheet.shown(win) and len(library._state(win._host.view)["items"]) >= 3000

    def components():
        timed("components", lambda: tab("components"))
        assert sum(k.startswith("cm:") for k in library._state(win._host.view)["items"]) >= 3000

    def close_sheet():
        timed("close", lambda: sheet.close(win))
        assert not sheet.shown(win)

    def finish():
        assert len(app.scene.mesh.edges) == 4 and win._host.state()["errors"] == 0, values
        Path(__file__).with_suffix(".json").write_text(json.dumps(values))

    def line():
        timed("line_commit", lambda: click(560))
        assert len(app.scene.mesh.edges) == 5, values

    for name, action in [
        ("Prepare view", start), ("Open", open_sheet),
        ("Components", components),
        ("Materials", lambda: timed("materials", lambda: tab("materials"))),
        ("Close", close_sheet),
        ("First press", lambda: timed("first_press", lambda: win._host.pointer("press", 400, 480, 1, 1))),
        ("Commit line", line), ("Erase", lambda: timed("erase_commit", lambda: click(480))),
        ("Record", finish),
    ]:
        app.add_menu_action(name, action)


def measurement_script():
    steps = ["wait 700"]
    for i in range(9):
        if i == 5:
            steps.extend(["tool line", "move 400 480", "wait 150"])
        elif i == 6:
            steps.extend(["release 400 480", "move 560 480", "wait 150"])
        elif i == 7:
            steps.extend(["key escape", "tool eraser", "move 480 480", "wait 150"])
        steps.extend([f"menu extension:library_budget:{i}", "wait 100"])
    return "\n".join([*steps, "state", "quit", ""])


@pytest.mark.perf
@pytest.mark.window
@pytest.mark.skipif(os.environ.get("BUILT_WINDOW_TESTS") != "1", reason="requires a native window")
def test_large_document_interaction_budgets(tmp_path):
    binary = Path(os.environ.get("BUILT_BINARY", ROOT / "rust/built_app/target/release/built"))
    source = large_document(tmp_path)
    script = tmp_path / "measure.txt"
    script.write_text(measurement_script())
    trials = []
    for attempt in range(3):
        profile = tmp_path / f"profile-{attempt}"
        plugin = profile / "data/built/plugins/library_budget.py"
        plugin.parent.mkdir(parents=True)
        plugin.write_text(inspect.getsource(setup))
        result = run([binary, "--new-instance", "--root", ROOT, "--venv", ROOT / ".venv",
                      "--size", "1280x800", "--script", script, source],
                     settings=profile, capture_output=True, text=True, timeout=180)
        report = plugin.with_suffix(".json")
        assert result.returncode == 0 and report.exists(), result.stderr
        trials.append(json.loads(report.read_text()))
        print(trials[-1])
        best = {key: min(trial[key] for trial in trials) for key in BUDGET_MS}
        overruns = [f"{key}: best {best[key]:.1f} ms (budget {limit} ms)"
                    for key, limit in BUDGET_MS.items() if best[key] >= limit]
        if not overruns:
            return
    pytest.fail("\n".join(overruns))
