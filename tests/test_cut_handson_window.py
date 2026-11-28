# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise real pointer picks on the solids used for a post and beam."""
import inspect
import json
import os
from pathlib import Path

import pytest
from built_core import Matrix4
from built_core.group import world_mesh
from built_core.history import History
from built_core.scene import Scene
from formats import igz
from scripts.isolated_launch import run
from tests.test_sketch_rows import apply
from tests.test_live_cut import make_member


def launch_window(folder, model, steps):
    root = Path(__file__).resolve().parents[1]
    binary = os.getenv("BUILT_BINARY", root / "rust/built_app/target/release/built")
    script = folder / "steps.txt"
    script.write_text("\n".join(["wait 400", *steps, "quit"]))
    result = run([binary, "--new-instance", "--root", root, "--venv", root / ".venv",
                  "--size", "1600x1000", "--script", script, "--shots", folder, model],
                 settings=folder/"profile", text=True, capture_output=True, timeout=90)
    assert result.returncode == 0 and "Traceback" not in result.stderr, result.stderr


def drawing_view(app):
    from built_core import Vec3

    def plan():
        camera = app.viewport.camera
        camera.set_view("top")
        camera.fit_to(Vec3(-4, -2, -.1), Vec3(4, 2, 0))
    app.add_menu_action("Plan", plan)


@pytest.fixture(scope="module")
def drawn_stock(tmp_path_factory):
    folder = tmp_path_factory.mktemp("drawn-stock")
    empty = folder / "empty.built"
    igz.save_scene(Scene(), empty)
    plugin = folder / "profile/data/built/plugins/stock.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text(inspect.getsource(drawing_view).replace("def drawing_view(", "def setup("))
    for name, first, opposite, middle, height in [
            ("beam", "541 526", "973 474", "757 500", ".6"),
            ("post", "765 535", "835 465", "800 500", "4")]:
        launch_window(folder, empty, [
            "menu extension:stock:0", "wait 250", "tool rectangle", "click "+first,
            "move "+opposite, "click "+opposite, "tool pushpull", "move "+middle,
            "click "+middle, "move 750 360", "type "+height, "key enter", "tool select",
            "menu select_all", "menu make_group", f"menu save_to:{folder}/{name}-group.built",
            "menu select_all", "menu verb:make_component", "wait 300", "key enter",
            "wait 200", f"menu save_to:{folder}/{name}-component.built"])
    return folder


def post_and_beam(shape, crossing, folder):
    scene = Scene()
    history = History(scene)
    if shape == "sweep":
        make_member(history, [[-4, 0, 2.3], [1 if crossing else -.0225, 0, 2.3]])
        make_member(history, [[0, 0, 0], [0, 0, 4]])
        return scene
    for name in ("beam", "post"):
        doc = Scene()
        form = "group" if shape == "loose" else shape
        igz.load_into(doc, folder / f"{name}-{form}.built")
        group, = doc.groups
        assert group.is_component() == (form == "component")
        assert len(group.mesh.faces) == 6
        doc.groups.remove(group)
        group.name = name.title()
        scene.groups.append(group)
    beam, post = scene.groups
    right = max(v.position.x() for v in world_mesh(beam).vertices)
    left = min(v.position.x() for v in world_mesh(post).vertices)
    shift = Matrix4()
    shift.translate((1 if crossing else left) - right, 0, 2)
    beam.xform = shift if beam.xform is None else shift * beam.xform
    if shape == "loose":
        for group in (beam, post):
            apply(history, "explode_group", placement=group.uid)
    scene.select([])
    return scene


def window_probe(app):
    import json
    from pathlib import Path
    from built_core import Vec3, command_protocol
    from built_core.solids import solid_volume

    base = Path(__file__).parent
    def frame():
        camera = app.viewport.camera
        camera.set_view("iso")
        camera.fit_to(Vec3(-4, -.4, 0), Vec3(1, .4, 4))

    def coordinates():
        display = app.window._host.view
        x, y = getattr(display, "chrome_offset", (0, 0))
        points = []
        from built_core.group import world_mesh
        post = app.scene.groups[-1] if app.scene.groups else None
        side = max(v.position.x() for v in world_mesh(post).vertices) if post else .4
        for point in [(-1, 0, 2.3), (side, 0, 3.3)]:
            pos = display._world_to_pixel(Vec3(*point))
            points.append([round(pos[0]+x), round(pos[1]+y)])
        (base / "pixels.json").write_text(json.dumps(points))

    def sample():
        tool = app.window._tools["cut"]
        preview = tool._machine.preview()
        with (base / "samples.jsonl").open("a") as f:
            print(json.dumps(dict(prompt=tool._state("prompt"), target=tool._state("target"),
                                  sizes=tool._state("cut_sizes"), lines=len(preview["lines"]), faces=len(preview["faces"]),
                                  volumes=[solid_volume(g) for g in app.scene.groups],
                                  rows=command_protocol.live_rows(app.scene))), file=f)
    app.add_menu_action("Frame members", frame)
    app.add_menu_action("Measure pointer positions", coordinates)
    app.add_menu_action("Read cut relation", sample)


@pytest.mark.window
@pytest.mark.skipif(os.getenv("BUILT_WINDOW_TESTS") != "1", reason="requires native window")
@pytest.mark.parametrize("shape", ["group", "component", "sweep", "loose"])
@pytest.mark.parametrize("crossing", [True, False], ids=["crossing", "end"])
def test_cut_member_picks(tmp_path, shape, crossing, drawn_stock):
    profile = tmp_path / "profile"
    plugin = profile / "data/built/plugins/cut_probe.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text(inspect.getsource(window_probe).replace("def window_probe(", "def setup("))
    model = tmp_path / "members.built"
    igz.save_scene(post_and_beam(shape, crossing, drawn_stock), model)
    menu = "menu extension:cut_probe:"
    def launch(steps):
        launch_window(tmp_path, model, [menu+"0", "wait 300", *steps])
    launch([menu+"1"])
    beam, post = [" ".join(map(str, p)) for p in json.loads((plugin.parent/"pixels.json").read_text())]
    steps = []
    presets = ("Penetration", "Cope", "Mitre", "End plate")
    for preset in presets:
        steps += ["tool cut", "key l", "wait 150", "type "+preset, "wait 150", "key enter", "wait 100", "move "+beam, menu+"2", "click "+beam, menu+"2",
                  "move "+post, menu+"2", "click "+post, menu+"2", "tool select"]
        if shape != "loose":
            steps += ["menu undo"]
    launch(steps)
    samples = [json.loads(line) for line in (plugin.parent/"samples.jsonl").read_text().splitlines()]
    assert len(samples) == 16
    for i, kind in enumerate(["body", "body", "bisector", "plane"]):
        beam_hover, target, post_hover, result = samples[i*4:i*4+4]
        cuts = [r for r in result["rows"] if r["kind"] == "cut"]
        if shape == "loose":
            assert not cuts and not target["target"] and "loose geometry" in target["prompt"]
        else:
            assert beam_hover["lines"] >= 12 and beam_hover["faces"] == 0
            if kind == "plane":
                assert post_hover["faces"] == 1
            elif kind == "body" and shape == "sweep" and (crossing or presets[i] == "Cope"):
                # A rectangular cut face can have just four rim edges.
                assert post_hover["lines"] >= 4 and post_hover["faces"] == 0
                assert dict(post_hover["sizes"])["Depth"] > 0
            else:
                assert post_hover["lines"] >= 12 and post_hover["faces"] == 0
            cut, = cuts
            assert cut["name"] == presets[i]
            assert cut["owner"] == target["target"] and kind in cut["cutter"]
            assert cut["clearance"] == (2 if i == 1 else 0)
            if i == 0 and not crossing:
                assert cut["state"] == "cut_misses" and "do not overlap" in result["prompt"]
            elif i == 3 and not crossing:
                # The selected face is the far side of a touching post.
                assert cut["state"] in ("ok", "cut_misses")
            else:
                assert cut["state"] == "ok"
            assert result["volumes"][1] == pytest.approx(target["volumes"][1])
            if crossing and i in (0, 1):
                assert result["volumes"][0] < target["volumes"][0]
