# SPDX-License-Identifier: GPL-3.0-or-later
"""Slab and Footing presets, the Sketch workspace's Release and a refused
drop in the tree, in a real window with real pointer input.

Each case launches the app twice in a private, lived-in profile (a dozen
recent files, three of them gone): once to learn where its points are on
screen, once to work as a first-time user would. Nothing is done through
protocol verbs except reading the model back."""
import inspect
import json
import os
from pathlib import Path

import pytest

from scripts.isolated_launch import run

ROOT = Path(__file__).resolve().parents[1]
pytestmark = [pytest.mark.window,
              pytest.mark.skipif(os.getenv("BUILT_WINDOW_TESTS") != "1", reason="requires native window")]


def probe(app):
    import json
    from pathlib import Path
    from built_core import Vec3, command_protocol as live, units
    from built_core.settings import settings

    folder = Path(__file__).parent
    config = json.loads((folder / "points.json").read_text())

    def setup():
        units.apply_units(app.scene, units.with_profile({}, "millimetric"))
        units.bind_scene(app.scene)
        settings().setValue("home/recent_files", json.dumps([{"path": p, "pinned": i < 2} for i, p in enumerate(config["recents"])]))

    def view(kind):
        def go():
            camera = app.viewport.camera
            camera.set_view(kind)
            camera.perspective = kind == "iso"
            if kind == "iso":
                camera.yaw, camera.pitch = -0.55, 0.42
            lo, hi = config["bounds"]
            camera.fit_to(Vec3(*lo), Vec3(*hi))
        return go

    def pixels():
        v = app.window._host.view
        offset = getattr(v, "chrome_offset", (0, 0))
        coords = []
        for point in config["points"]:
            xy = v._world_to_pixel(Vec3(*point)) or (-10000, -10000)
            coords.append([round(xy[0] + offset[0]), round(xy[1] + offset[1])])
        (folder / "pixels.json").write_text(json.dumps(coords))

    def dump():
        groups = []
        for g in app.scene.groups:
            pts = [(v.x(), v.y(), v.z()) for f in g.mesh.faces for v in f.vertices]
            if pts and g.xform is not None:
                m = g.xform.data()
                pts = [tuple(m[i] * x + m[4 + i] * y + m[8 + i] * z + m[12 + i] for i in range(3)) for x, y, z in pts]
            groups.append(dict(name=g.name, uid=g.uid, cls=(g.ifc or {}).get("class"), faces=len(g.mesh.faces),
                               lo=[min(p[i] for p in pts) for i in range(3)] if pts else None,
                               hi=[max(p[i] for p in pts) for i in range(3)] if pts else None))
        out = dict(groups=groups, rows=live.live_rows(app.scene), undo=len(app.viewport.history.undo_stack))
        with (folder / "dump.jsonl").open("a") as stream:
            print(json.dumps(out), file=stream)

    for name, action in [("Setup", setup), ("Iso", view("iso")), ("Top", view("top")), ("Pixels", pixels), ("Dump", dump)]:
        app.add_menu_action(name, action)


SETUP, ISO, TOP, PIXELS, DUMP = (f"menu extension:probe:{i}" for i in range(5))


class Session:
    """A private profile with the probe plugin; `points` come back as pixels."""

    def __init__(self, folder, points, bounds, view, size):
        self.folder, self.view, self.size = folder, view, size
        self.plugins = folder / "profile/data/built/plugins"
        self.plugins.mkdir(parents=True)
        (self.plugins / "probe.py").write_text(inspect.getsource(probe).replace("def probe(", "def setup("))
        studies = folder / "studies"
        studies.mkdir()
        recents = []
        for i in range(12):
            path = studies / f"Study {i + 1} of the long-named warehouse extension.built"
            if i % 4 != 3:
                path.write_bytes(b"")
            recents.append(str(path))
        (self.plugins / "points.json").write_text(json.dumps(dict(points=points, bounds=bounds, recents=recents)))
        self.launch([SETUP, "wait 200", view, "wait 300", PIXELS])
        self.pixels = [" ".join(map(str, p)) for p in json.loads((self.plugins / "pixels.json").read_text())]

    def launch(self, steps):
        binary = os.getenv("BUILT_BINARY", ROOT / "rust/built_app/target/release/built")
        script = self.folder / "steps.txt"
        script.write_text("\n".join(["wait 500", "menu new", "wait 400", *steps, "quit", ""]))
        result = run([binary, "--new-instance", "--root", ROOT, "--venv", ROOT / ".venv", "--size", self.size,
                      "--script", script, "--shots", self.folder],
                     settings=self.folder / "profile", text=True, capture_output=True, timeout=180)
        assert result.returncode == 0 and "Traceback" not in result.stderr, result.stderr[-3000:]

    def work(self, steps):
        self.launch([SETUP, "wait 200", self.view, "wait 300", *steps])
        return [json.loads(line) for line in (self.plugins / "dump.jsonl").read_text().splitlines()]


def preset(name):
    """Sweep › L › the preset's name › Enter, as the status bar says."""
    return ["tool sweep", "wait 150", "key l", "wait 300", "type " + name, "wait 300", "key enter", "wait 300"]


def named(dump, name):
    return next(g for g in dump["groups"] if g["name"] == name)


def test_slab_and_footings_drawn_in_a_small_window(tmp_path):
    points = [[0, 0, 0], [6, 4, 0], [1, 6, 0], [5, 6, 0], [3, 6.1, 0], [9, 6, 0]]
    s = Session(tmp_path, points, [[-1, -1, -1], [11, 8, 3]], TOP, "1280x800")
    a, c, w0, w1, wall, free = s.pixels
    (dump,) = s.work([
        # A slab by two corners, its thickness typed first.
        *preset("Slab"), "type 250", "key enter", "wait 200", f"move {a}", f"click {a}", f"move {c}", "wait 200", f"click {c}", "wait 400", "key escape",
        *preset("Wall"), f"move {w0}", f"click {w0}", f"move {w1}", "wait 200", f"click {w1}", "wait 400", "key escape", "wait 200",
        # A strip under the wall it is pointed at, a pad at a free point.
        *preset("Strip footing"), f"move {wall}", "wait 400", f"click {wall}", "wait 500",
        *preset("Pad footing"), f"move {free}", "wait 300", f"click {free}", "wait 500", DUMP])
    slab, w, strip, pad = (named(dump, n) for n in ("SL-1", "W-1", "SF-1", "PF-1"))
    assert slab["cls"] == "IfcSlab" and slab["lo"][2] == pytest.approx(-0.25) and slab["hi"][2] == pytest.approx(0)
    assert [slab["hi"][i] - slab["lo"][i] for i in (0, 1)] == pytest.approx([6, 4], abs=0.05)
    # 150 mm past the wall's faces and its ends, 250 deep, its top under the wall.
    assert strip["cls"] == "IfcFooting" and strip["hi"][2] == pytest.approx(0) and strip["lo"][2] == pytest.approx(-0.25)
    assert [strip["lo"][i] - w["lo"][i] for i in (0, 1)] == pytest.approx([-0.15, -0.15], abs=1e-6)
    assert [strip["hi"][i] - w["hi"][i] for i in (0, 1)] == pytest.approx([0.15, 0.15], abs=1e-6)
    rows = {r["owner"]: r for r in dump["rows"] if r["kind"] == "sweep"}
    assert rows[strip["uid"]]["fit"]["host"]["sweep"] == rows[w["uid"]]["uid"]
    assert pad["cls"] == "IfcFooting" and [pad["hi"][i] - pad["lo"][i] for i in range(3)] == pytest.approx([0.6, 0.6, 0.4])
    assert "host" not in (rows[pad["uid"]].get("fit") or {})
    assert all(r["state"] == "ok" for r in dump["rows"])


def test_release_in_the_sketch_workspace_gives_each_member_its_own_drawing(tmp_path):
    points = [[0, 0, 0], [3, 0, 0], [0, 2, 0], [3, 2, 0], [1.5, 0, 0.02]]
    s = Session(tmp_path, points, [[-0.5, -0.5, -0.5], [3.5, 2.5, 1]], ISO, "1600x1000")
    a, b, c, d, on_first = s.pixels
    # Right-click the member › Edit profile: the menu's first row, beside the pointer.
    x, y = map(int, on_first.split())
    edit_profile = f"{x + 50} {y + 23}"
    before, released, undone = s.work([
        # A profile drawn from the Sweep tool (D), two members made with it.
        "tool sweep", "wait 200", "key d", "wait 700", "type r", "click 600 400", "move 700 300", "type 100;50", "key enter", "wait 300",
        "click-command sketch_done", "wait 700",
        f"move {a}", f"click {a}", f"move {b}", f"click {b}", "wait 300", "key escape", "wait 200",
        "tool sweep", f"move {c}", f"click {c}", f"move {d}", f"click {d}", "wait 300", "key escape", "wait 300", DUMP,
        # One member's profile opened, a hole drawn in it, then Release instead of Done.
        "tool select", "wait 200", f"click {on_first}", "wait 400", f"click {on_first} right", "wait 500", f"click {edit_profile}", "wait 800",
        "type c", "click 686 502", "move 760 502", "type 15", "key enter", "wait 300",
        "click-command sketch_release", "wait 900", DUMP,
        "menu undo", "wait 500", DUMP])
    sketches = lambda dump: [r for r in dump["rows"] if r["kind"] == "sketch"]  # noqa: E731
    sweeps = lambda dump: [r for r in dump["rows"] if r["kind"] == "sweep"]  # noqa: E731
    (shared,) = sketches(before)
    assert [r["profile"]["sketch"] for r in sweeps(before)] == [shared["uid"]] * 2
    # One step: both members took the hole, each on a frozen drawing of its own.
    assert released["undo"] == before["undo"] + 1
    copies = sketches(released)
    assert len(copies) == 2 and all(c.get("frozen") is True and c["state"] == "ok" for c in copies)
    assert sorted(r["profile"]["sketch"] for r in sweeps(released)) == sorted(c["uid"] for c in copies)
    members = [g for g in released["groups"] if g["cls"]]
    assert len(members) == 2 and all(g["faces"] > 6 for g in members)
    # One Undo: the shared drawing is back, without the hole.
    assert [r["uid"] for r in sketches(undone)] == [shared["uid"]] and "frozen" not in sketches(undone)[0]
    assert all(g["faces"] == 6 for g in undone["groups"] if g["cls"])


def test_a_cut_dropped_on_a_junction_row_that_refuses_it_stays(tmp_path):
    """A cut made with the Cut tool, dragged in the tree onto the row of a
    junction it has no part in: refused, nothing changes, no History step."""
    points = [[0, 0, 0], [6, 4, 0], [1, 3, 0], [5, 3, 0], [3, 3.1, 0.4], [8, 0, 0], [11, 3, 0], [9.5, 1.5, 0], [9.5, -2, 0]]
    s = Session(tmp_path, points, [[-1, -1, -1], [12, 5, 3]], ISO, "1600x1000")
    a, c, w0, w1, wall, a2, c2, mid2, free = s.pixels
    cut_row, junction_row = "1400 262", "1400 334"
    made, dropped, again, undone = s.work([
        *preset("Slab"), f"move {a}", f"click {a}", f"move {c}", "wait 200", f"click {c}", "wait 400",
        f"move {a2}", f"click {a2}", f"move {c2}", "wait 200", f"click {c2}", "wait 400", "key escape", "wait 200",
        *preset("Wall"), f"move {w0}", f"click {w0}", f"move {w1}", "wait 200", f"click {w1}", "wait 500", "key escape", "wait 300",
        "tool junction", "wait 400", "key l", "wait 400", "type Wall on slab", "wait 400", "key enter", "wait 400",
        f"move {wall}", "wait 700", f"click {wall}", "wait 900", "key escape", "wait 200",
        # An opening drawn on the second slab's face with the Cut tool (D).
        "tool cut", "wait 300", f"move {mid2}", "wait 300", f"click {mid2}", "wait 400", f"move {free}", "wait 300", "key d", "wait 900",
        "type r", "click 600 550", "move 640 520", "type 800;600", "key enter", "wait 400",
        "click-command sketch_done", "wait 900", f"move {mid2}", "wait 500", f"click {mid2}", "wait 800",
        # The tree: Slabs › SL-2 › its cut, Walls › W-1 › its junction.
        "click 1305 190", "wait 300", "click 1321 238", "wait 300", "click 1305 286", "wait 300", "click 1321 310", "wait 300", DUMP,
        f"move {cut_row}", "wait 200", f"press {cut_row}", "wait 200", "move 1400 270", "wait 100", "move 1400 290", "wait 100",
        "move 1400 315", "wait 100", f"move {junction_row}", "wait 300", f"release {junction_row}", "wait 800", DUMP,
        # Again with the Select tool in hand.
        "key escape", "wait 300", f"move {cut_row}", f"press {cut_row}", "wait 200", "move 1400 270", "wait 100", "move 1400 300", "wait 100",
        f"move {junction_row}", "wait 300", f"release {junction_row}", "wait 800", DUMP,
        "menu undo", "wait 500", DUMP])
    cuts = lambda dump: [r for r in dump["rows"] if r["kind"] == "cut"]  # noqa: E731
    (cut,) = cuts(made)
    (junction,) = [r for r in made["rows"] if r["kind"] == "junction"]
    assert junction["state"] == "ok" and named(made, "SL-2")["uid"] == cut["owner"]
    for after in (dropped, again):
        assert after["undo"] == made["undo"], "a refused drop is not a History step"
        assert cuts(after) == [cut] and [r for r in after["rows"] if r["kind"] == "junction"] == [junction]
    # So the next Undo is the cut's own creation, nothing else.
    assert undone["undo"] == made["undo"] - 1 and cuts(undone) == []
