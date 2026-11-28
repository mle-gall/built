# SPDX-License-Identifier: GPL-3.0-or-later
"""Fit Library profiles in a new, unsaved millimetre model."""
import inspect
import json
import os
from pathlib import Path

import pytest
from scripts.isolated_launch import run


def profile_probe(app):
    from pathlib import Path
    import json
    from built_core import Vec3, command_protocol
    from built_core.group import world_mesh
    from built_core.solids import solid_volume

    folder = Path(__file__).parent
    config = json.loads((folder / "points.json").read_text())

    def camera(bounds):
        view = app.viewport.camera
        view.set_view("iso")
        view.perspective = True
        view.yaw = -.40
        view.pitch = .32
        view.fit_to(Vec3(*bounds[0]), Vec3(*bounds[1]))

    def placement_view():
        lo = [min(p[i] for p in config[:4])-.2 for i in range(3)]
        hi = [max(p[i] for p in config[:4])+.2 for i in range(3)]
        camera([lo, hi])

    def connection_view():
        camera([[-.14, -.21, -.18], [.37, .18, .2]])

    def clearance_view():
        view = app.viewport.camera
        view.set_view("top")
        view.perspective = False
        view.fit_to(Vec3(-.035, -.065, .21), Vec3(.062, .065, .23))

    def prepare():
        from built_core.guide import Guide
        from built_core import units
        units.apply_units(app.scene, units.with_profile({}, "millimetric"))
        units.bind_scene(app.scene)
        app.scene.guides.extend([Guide(Vec3(*p)) for p in config[:4]])

    def pixels():
        view = app.window._host.view
        offset = getattr(view, "chrome_offset", (0, 0))
        coords = []
        for point in config:
            x, y = view._world_to_pixel(Vec3(*point)) or (-10000, -10000)
            coords.append([round(x + offset[0]), round(y + offset[1])])
        with (folder / "pixels.jsonl").open("a") as stream:
            print(json.dumps(coords), file=stream)

    def sample():
        tool = app.window._tools["cut"]
        preview = tool._machine.preview()
        rows = command_protocol.live_rows(app.scene)
        groups = {g.uid: g for g in app.scene.groups}
        members = []
        for row in rows:
            if row["kind"] == "sweep":
                group = groups[row["owner"]]
                members.append(dict(profile=row["profile"]["library"], owner=group.uid,
                                    volume=solid_volume(group), path=row["path"]["points"],
                                    vertices=[v.position.toTuple() for v in world_mesh(group).vertices]))
        members.sort(key=lambda m: abs(m["path"][0][0]-m["path"][-1][0])+abs(m["path"][0][1]-m["path"][-1][1]))
        with (folder / "samples.jsonl").open("a") as stream:
            print(json.dumps(dict(members=members, cuts=[r for r in rows if r["kind"] == "cut"],
                                  prompt=tool._state("prompt"), target=tool._state("target"),
                                  sizes=tool._state("cut_sizes"), lines=len(preview["lines"]), faces=len(preview["faces"]))), file=stream)

    def recovery():
        app.window._autosave_timer.start(200)

    def recovery_result():
        import shutil
        from built_core import autosave
        doc = app.window.active_document()
        slot = autosave.pending(doc.path, doc.untitled_no)
        worker = app.window._background_autosave
        (folder / "recovery.json").write_text(json.dumps(dict(path=doc.path, error=worker.last_error,
                units=app.scene.units["length"], saved=slot is not None)))
        if slot is not None:
            shutil.copyfile(slot, folder / "Recovered.built")
        app.window._autosave_timer.stop()

    for name, callback in [("Placement view", placement_view), ("Connection view", connection_view),
                           ("Measure pixels", pixels), ("Measure cut", sample), ("Start recovery timer", recovery), ("Set out model", prepare), ("Read recovery", recovery_result), ("Inspect clearance", clearance_view)]:
        app.add_menu_action(name, callback)


def launch_window(folder, steps, size="1600x1000"):
    root = Path(__file__).resolve().parents[1]
    script = folder/"steps.txt"
    script.write_text("\n".join(["wait 350", "menu new", "wait 300", *steps, "quit"]))
    result = run([os.getenv("BUILT_BINARY", root/"rust/built_app/target/release/built"), "--new-instance",
                  "--root", root, "--venv", os.getenv("BUILT_TEST_VENV", root/".venv"), "--size", size,
                  "--script", script, "--shots", folder], settings=folder/"profile",
                 text=True, capture_output=True, timeout=100)
    (folder/"window.log").write_text(result.stdout+result.stderr)
    assert result.returncode == 0, result.stderr
    assert "Traceback" not in result.stderr and "unknown key" not in result.stderr, result.stderr


def library_placement(profile, start, finish, start_view, end_view, size):
    tab, search, row = ("515 166", "1200 166", "540 287") if size.startswith("1600") else ("414 128", "1056 128", "416 250")
    return ["click-command library", "wait 200", "click "+tab, "wait 150", "click "+search,
            "type " + profile, "wait 180", "click "+row, "wait 180",
            start_view, "wait 200", "move " + start, "click " + start, end_view, "wait 220", "move " + finish, "click " + finish,
            "key escape", "tool select", "wait 260"]


def exercise_connection(folder, layout, preset, *, screenshots=False, autosave=False, size="1600x1000", shortcuts=False, preselected=False):
    column = [[0, 0, -.22], [0, 0, .3]]
    side = .09 if layout == "beside" else 0
    ends = {"touch": .00325, "gap": .03825, "overrun": -.035, "beside": -.42, "corner": 0,
            "flange": -.095, "cope_end": .00325}
    beam = [[3.86, side, 0], [ends[layout], side, 0]]
    if layout == "corner":
        column = [[0, -.45, 0], [0, 0, 0]]
    elif layout == "cope_end":
        column = [[0, -.75, .14], [0, .30, .14]]
    elif layout == "flange":
        beam = [[0, -3.85, 0], [0, -.095, 0]]
    picks = [[.24, side, .1], [.00325, 0, .165]]
    if layout == "corner":
        picks[1] = [0, -.14, .1]
    elif layout == "cope_end":
        picks[1] = [0, -.14, .235]
    elif layout == "flange":
        picks = [[0, -.23, .1], [.04, -.095, .16]]
    points = column + beam + picks
    plugin = folder / "profile/data/built/plugins/profile_probe.py"
    plugin.parent.mkdir(parents=True)
    plugin.write_text(inspect.getsource(profile_probe).replace("def profile_probe(", "def setup("))
    (plugin.parent / "points.json").write_text(json.dumps(points))
    action = "menu extension:profile_probe:"
    launch_window(folder, [action+"0", "wait 220", action+"2", action+"1", "wait 220", action+"2"], size)
    positions = [json.loads(line) for line in (plugin.parent/"pixels.jsonl").read_text().splitlines()]
    pixels = [" ".join(map(str, v)) for v in positions[1][:2] + positions[0][2:3] + positions[1][3:]]
    steps = [action+"5", action+"0", "wait 230"]
    placements = [("IPE 200" if layout == "corner" else "HE 200 A", *pixels[:2]),
                       ("IPE 200", *pixels[2:4])]
    if layout in ("beside", "overrun"):
        placements.reverse()
    for profile, a, b in placements:
        steps += library_placement(profile, a, b, action+("1" if a == pixels[0] else "0"), action+"1", size)
    steps += [action+"1", "wait 260"]
    if screenshots:
        steps += ["capture stock"]
        if layout == "touch" and preset in (None, "Cope"):
            steps += [action+"7", "wait 220", "capture gap-before", action+"1", "wait 220"]
    if preselected:
        steps += ["move "+pixels[4], "click "+pixels[4], "wait 450"]
    steps += ["tool cut"]
    if preset and preset != "Flush" and shortcuts:
        steps += ["key "+{"Cope":"c", "End plate":"p", "Penetration":"b", "Mitre":"m"}[preset]]
    elif preset and preset != "Flush":
        steps += ["key l", "wait 180", "type "+preset, "wait 180", "key enter"]
    steps += ["move "+pixels[4], action+"3"]
    if not preselected:
        steps += ["click "+pixels[4]]
    steps += [action+"3", "move "+pixels[5], "wait 150", action+"3", "state"]
    if preset == "Flush":
        steps += ["type 0", "key enter", "wait 160"]
    if screenshots:
        steps += ["capture preview"]
    steps += ["click "+pixels[5], "wait 180", action+"3", "state"]
    if screenshots:
        steps += ["capture result"]
        if layout == "touch" and preset in (None, "Cope"):
            steps += ["tool select", action+"7", "wait 220", "capture gap-after", action+"1", "wait 220"]
    if autosave:
        steps += ["tool select", action+"4", "wait 1400", action+"6", "state", "capture recovery",
                  f"menu save_to:{folder}/Manual.built", "state"]
    if screenshots and preset == "Cope":
        steps += ["tool select", "wait 600", "move "+pixels[4], "click "+pixels[4], "wait 250", "capture selected"]
    if screenshots:
        steps += ["tool select", "wait 600", "move "+pixels[5],
                  "click "+pixels[5], "wait 120", "menu hide", "wait 180", "capture beam"]
    launch_window(folder, steps, size)
    return [json.loads(line) for line in (plugin.parent/"samples.jsonl").read_text().splitlines()]


@pytest.mark.window
@pytest.mark.skipif(os.getenv("BUILT_WINDOW_TESTS") != "1", reason="requires native window")
@pytest.mark.parametrize(("layout","preset"), [
    ("flange", None), ("touch", "Penetration"), ("touch", "Flush"), ("touch", None), ("gap", None), ("overrun", None), ("beside", None),
    ("touch", "Cope"), ("beside", "Cope"), ("cope_end", "Cope"), ("gap", "End plate"),
    ("beside", "Penetration"), ("corner", "Mitre"),
])
def test_library_connection(tmp_path, layout, preset):
    recover = (layout == "touch" and preset is None) or layout == "corner"
    hover, selected, preview, result = exercise_connection(tmp_path, layout, preset,
                                                          autosave=recover, shortcuts=True)
    stock, beam = hover["members"]
    assert [stock["profile"], beam["profile"]] == [
        "en/ipe200" if layout == "corner" else "en/he200a", "en/ipe200"]
    assert selected["target"] == beam["owner"]
    if layout in ("touch", "gap", "overrun"):
        assert beam["volume"] == pytest.approx(.011, abs=.00013)
    assert hover["lines"] > 12 and hover["faces"] == 0
    assert not preview["cuts"]
    if preset == "Cope" and layout in ("touch", "cope_end"):
        expected = (.002, .2) if layout == "touch" else (.09875, .057)
        assert dict(preview["sizes"]) == pytest.approx(dict(zip(("Length", "Depth"), expected)), abs=1e-6)
    assert preview["faces"] == (1 if preset == "End plate" else 0)
    cut = next(r for r in result["cuts"] if r["owner"] == beam["owner"])
    new_stock, new_beam = result["members"]
    if layout != "corner":
        assert stock["vertices"] == new_stock["vertices"]
    delta = new_beam["volume"] - beam["volume"]
    if preset == "Penetration" and layout == "touch":
        assert cut["state"] == "cut_misses"
        assert abs(delta) < 1e-10
    elif preset == "Cope" or layout == "beside":
        assert cut["state"] == "ok" and cut["cutter"] == {"body": stock["owner"]}
        assert cut["clearance"] == (2 if preset == "Cope" else 0)
        assert delta < -1e-6
    elif layout == "corner":
        assert "both" in preview["prompt"]
        assert len(result["cuts"]) == 2
        assert all(r["state"] == "ok" and "bisector" in r["cutter"] for r in result["cuts"])
        # Both mating caps lie on the same 45-degree plane through the corner.
        for member in (new_stock, new_beam):
            tip = [p for p in member["vertices"] if abs(p[0]) < .12 and abs(p[1]) < .12]
            assert len(tip) >= 12
            assert max(abs(x+y) for x,y,_ in tip) < 2e-5
    else:
        assert cut["state"] == "ok" and "plane" in cut["cutter"]
        assert cut["anchor"]["ref"]["path"][-1] == stock["owner"]
        if layout == "gap":
            assert "extend" in preview["prompt"] and delta > .00009
        elif layout == "overrun":
            assert "trim" in preview["prompt"] and delta < -.00009
        else:
            assert "type 0 for flush" in preview["prompt"]
            if preset == "Flush":
                assert abs(delta) < 1e-10
            else:
                assert delta < -5e-6
        coord = 1 if layout == "flange" else 0
        tip = (max if coord else min)(v[coord] for v in new_beam["vertices"])
        allowance = 0.002 if layout in ("touch", "flange") and preset is None else 0
        expected_tip = -.095 - allowance if coord else .00325 + allowance
        assert tip == pytest.approx(expected_tip, abs=1e-6)
        if preset is None:
            assert preview["lines"] > selected["lines"]
    if recover:
        from built_core import command_protocol
        from built_core.scene import Scene
        from formats import igz
        folder = tmp_path/"profile/data/built/plugins"
        recovery = json.loads((folder/"recovery.json").read_text())
        assert recovery["path"] is None and recovery["saved"] and recovery["units"] == "mm", recovery
        for path in (tmp_path/"Manual.built", folder/"Recovered.built"):
            reopened = Scene()
            igz.load_into(reopened, path)
            assert [r for r in command_protocol.live_rows(reopened) if r["kind"] == "cut"] == result["cuts"]
        if layout == "corner":
            from built_core.group import world_mesh
            from built_core.history import History
            from tests.test_sketch_rows import apply
            history = History(reopened)
            apply(history, "move_placements", placements=[stock["owner"]], delta=[.02, 0, 0])
            for group in reopened.groups:
                if group.uid in (stock["owner"], beam["owner"]):
                    ends = [v.position for v in world_mesh(group).vertices
                            if abs(v.position.x())<.14 and abs(v.position.y())<.12]
                    assert max(abs(v.x()+v.y()-.02) for v in ends) < 2e-5
            apply(history, "undo")
            assert all(r["state"] == "ok" for r in command_protocol.live_rows(reopened))


@pytest.mark.window
@pytest.mark.skipif(os.getenv("BUILT_WINDOW_TESTS") != "1", reason="requires native window")
def test_preselected_library_beam_fits_with_one_click(tmp_path):
    hover, _, preview, result = exercise_connection(tmp_path, "overrun", None, preselected=True)
    assert hover["target"] == hover["members"][1]["owner"]
    assert "trim" in preview["prompt"]
    assert result["cuts"][0]["state"] == "ok"
    assert result["members"][1]["volume"] < hover["members"][1]["volume"] - .00009
