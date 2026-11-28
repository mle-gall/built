# SPDX-License-Identifier: GPL-3.0-or-later
"""Slab and Footing presets on the general primitives: an extrusion is a
sweep whose path is a point, a direction and a depth; its edge profile is a
stack of rings; a footing is sized from its host through `fit`."""
import pytest

from built_core import Matrix4, Vec3
from built_core import command_protocol as live
from built_core.group import Group
from built_core.history import History
from built_core.protocol import commands
from built_core.scene import Scene
from built_core.solids import solid_volume
from formats import igz
from tests import addon_cases
from tests.test_live_recipes import apply
from tests.test_persistent_ids import all_ids


def preset(name, scene=None):
    return next(p for p, _ in live.preset_catalog(scene) if p["name"] == name)


def steel(key):
    """A catalogue size from the steel add-on pack (never bundled): the row keeps its own dimensions."""
    return addon_cases.profile(key)


def steel_column():
    return next(r for r in addon_cases.rows("steel-profiles-en") if r["name"] == "Steel column")


def bounds(group):
    """World bounds: a member keeps its own frame (X along it, origin at its start)."""
    pts = [(v.x(), v.y(), v.z()) for f in group.mesh.faces for v in f.vertices]
    if group.xform is not None:
        m = group.xform.data()
        pts = [tuple(m[i] * x + m[4 + i] * y + m[8 + i] * z + m[12 + i] for i in range(3)) for x, y, z in pts]
    return [min(p[i] for p in pts) for i in range(3)], [max(p[i] for p in pts) for i in range(3)]


def slab(h, name="Slab", width=4000, height=3000, **more):
    p = preset(name)
    spec = dict(preset_uid=p["uid"], profile=dict(library="basic/flat", params=dict(width=width, height=height)),
                attachment="bottom_left", path=dict(points=[[0, 0, 0]], dir=[0, 0, -1]), **more)
    apply(h, "create_sweeps", sweeps=[spec])
    return h.scene.groups[-1]


def test_slab_is_an_extrusion_below_its_outline_filed_as_a_slab(tmp_path):
    scene = Scene()
    h = History(scene)
    g = slab(h)
    lo, hi = bounds(g)
    assert lo == pytest.approx([0, -3, -0.2]) and hi == pytest.approx([4, 0, 0])
    assert solid_volume(g) == pytest.approx(4 * 3 * 0.2)
    assert g.ifc["class"] == "IfcSlab" and g.ifc["predefined_type"] == "FLOOR" and g.name == "SL-1"
    row = live.live_sweep(scene, g.uid)
    assert row["path"]["depth"] == 200 and row["preset"] == "Slab"
    ids = all_ids(g.mesh)
    # The thickness is a free field: the slab stays on its preset.
    row["path"]["depth"] = 300
    apply(h, "live_set", row=row)
    assert bounds(g)[0][2] == pytest.approx(-0.3) and solid_volume(g) == pytest.approx(3.6)
    assert live.live_sweep(scene, g.uid)["preset_uid"] == preset("Slab")["uid"]
    # The same faces: a dimension on the top face keeps its face.
    assert all_ids(g.mesh)["face"] == ids["face"]
    apply(h, "undo")
    assert solid_volume(g) == pytest.approx(2.4)
    path = tmp_path / "slab.built"
    igz.save_scene(scene, path)
    opened = Scene()
    igz.load_into(opened, path)
    again = live.live_sweep(opened, opened.groups[0].uid)
    assert again["path"] == {"points": [[0, 0, 0]], "closed": False, "depth": 200, "dir": [0, 0, -1]}
    assert solid_volume(opened.groups[0]) == pytest.approx(2.4)


def test_a_path_with_a_depth_needs_its_direction():
    h = History(Scene())
    p = preset("Slab")
    with pytest.raises(ValueError, match="invalid_path"):
        commands.run(h, "create_sweeps", sweeps=[dict(preset_uid=p["uid"], path=dict(points=[[0, 0, 0]]))])
    # Two points give it: the depth then replaces their distance.
    apply(h, "create_sweeps", sweeps=[dict(preset_uid=p["uid"], path=dict(points=[[0, 0, 0], [0, 0, 5]]))])
    assert bounds(h.scene.groups[0])[1][2] == pytest.approx(0.2)


@pytest.mark.parametrize("edge, faces, volume, low", [
    (dict(shape="chamfer", size=50, side="start"), 10, 2.4 - 14 * 0.05 * 0.05 / 2 + 4 * 0.05 ** 3 / 3, -0.2),
    (dict(shape="chamfer", size=50, side="both"), 14, 2.4 - 2 * (14 * 0.05 * 0.05 / 2 - 4 * 0.05 ** 3 / 3), -0.2),
    (dict(shape="step", size=100, depth=50, side="start"), 11, 2.4 - (12 - 3.8 * 2.8) * 0.05, -0.2),
    (dict(shape="downstand", size=400, depth=350, side="end"), 11, 2.4 + (12 - 3.2 * 2.2) * 0.35, -0.55),
])
def test_edge_profiles_are_rings_between_the_outline_and_an_end_face(edge, faces, volume, low):
    h = History(Scene())
    g = slab(h, edge=edge)
    assert len(g.mesh.faces) == faces
    assert solid_volume(g) == pytest.approx(volume)
    lo, hi = bounds(g)
    assert lo == pytest.approx([0, -3, low]) and hi == pytest.approx([4, 0, 0])


def test_a_rounded_edge_is_smooth_and_an_edge_too_large_keeps_the_last_slab():
    scene = Scene()
    h = History(scene)
    g = slab(h, edge=dict(shape="round", size=40, side="start"))
    soft = [e for e in g.mesh.edges if e.soft]
    assert soft and solid_volume(g) < 2.4
    good = solid_volume(g)
    row = live.live_sweep(scene, g.uid)
    row["edge"] = dict(shape="chamfer", size=2000, side="start")
    apply(h, "live_set", row=row)
    assert live.live_sweep(scene, g.uid)["state"] == "edge_too_large"
    assert solid_volume(g) == pytest.approx(good)


def test_raft_slab_preset_carries_its_edge_beam_and_edits_follow_the_preset():
    scene = Scene()
    h = History(scene)
    g = slab(h, "Raft slab")
    row = live.live_sweep(scene, g.uid)
    assert row["edge"] == {"shape": "downstand", "size": 400, "depth": 350, "side": "end"}
    assert g.ifc["predefined_type"] == "BASESLAB" and bounds(g)[0][2] == pytest.approx(-0.6)
    # A project copy of the preset with a deeper beam updates the slab in one step.
    p = dict(preset("Raft slab"))
    p["values"] = dict(p["values"], edge=dict(shape="downstand", size=400, depth=500, side="end"))
    p["locks"] = dict(p["locks"], edge="fixed")
    apply(h, "preset_update", rows=[p])
    assert bounds(g)[0][2] == pytest.approx(-0.75)
    apply(h, "undo")
    assert bounds(g)[0][2] == pytest.approx(-0.6)


def test_openings_are_live_cuts_that_follow_the_slab():
    from tests.test_geometry_arrays import CUBE
    scene = Scene()
    shaft = Group(name="Shaft")
    shaft.xform = Matrix4()
    for face in CUBE:
        shaft.mesh.add_face([Vec3(1 + x * 0.5, -1.5 + y * 0.5, z - 0.5) for x, y, z in face])
    scene.groups.append(shaft)
    h = History(scene)
    g = slab(h)
    opening = preset("Penetration")
    apply(h, "live_set", row=dict(kind="cut", owner=g.uid, uid="c0700000000000a1", cutter={"body": shaft.uid},
                                  operation="subtract", preset=opening["name"]))
    assert solid_volume(g) == pytest.approx(2.4 - 0.5 * 0.5 * 0.2)
    row = live.live_sweep(scene, g.uid)
    row["path"]["depth"] = 300
    apply(h, "live_set", row=row)
    assert solid_volume(g) == pytest.approx(3.6 - 0.5 * 0.5 * 0.3)
    apply(h, "move_placements", placements=[shaft.uid], delta=[10, 0, 0])
    assert solid_volume(g) == pytest.approx(3.6)


def test_wall_on_slab_meets_a_preset_slab_on_its_top():
    scene = Scene()
    h = History(scene)
    g = slab(h)
    apply(h, "create_sweeps", sweeps=[dict(profile=dict(library="basic/wall"), attachment="bottom_left",
                                           path=dict(points=[[0.5, -1, 0], [3, -1, 0]]))])
    wall = scene.groups[1]
    meetings = [m for m in live.meetings(scene) if m["kind"] == "base"]
    assert len(meetings) == 1 and set(meetings[0]["members"]) == {g.uid, wall.uid}
    detail = preset("Wall on slab", scene)
    apply(h, "junction_apply", detail=detail["uid"], bindings={"main": {"placement": g.uid}, "joining": {"placement": wall.uid}})
    j = next(r for r in live.live_rows(scene) if r["kind"] == "junction")
    assert j["state"] == "ok" and j["frames"]["meet"][14] == pytest.approx(0)


def test_a_cut_a_junction_refuses_stays_as_it_was_made():
    """A cut dropped on a junction's row that cannot take it (it cuts a member
    outside the meeting): refused with its reason, no History step, so the
    next Undo is the cut's own creation and Redo brings it back."""
    from tests.test_geometry_arrays import CUBE
    scene = Scene()
    shaft = Group(name="Shaft")
    shaft.xform = Matrix4()
    for face in CUBE:
        shaft.mesh.add_face([Vec3(11 + x * 0.5, -1.5 + y * 0.5, z - 0.5) for x, y, z in face])
    scene.groups.append(shaft)
    h = History(scene)
    g = slab(h)
    apply(h, "create_sweeps", sweeps=[dict(profile=dict(library="basic/wall"), attachment="bottom_left",
                                           path=dict(points=[[0.5, -1, 0], [3, -1, 0]]))])
    wall = scene.groups[-1]
    detail = preset("Wall on slab", scene)
    apply(h, "junction_apply", detail=detail["uid"], bindings={"main": {"placement": g.uid}, "joining": {"placement": wall.uid}})
    j = next(r for r in live.live_rows(scene) if r["kind"] == "junction")
    p = preset("Slab")
    apply(h, "create_sweeps", sweeps=[dict(preset_uid=p["uid"], attachment="bottom_left", path=dict(points=[[10, 0, 0]], dir=[0, 0, -1]))])
    other = scene.groups[-1]
    apply(h, "live_set", row=dict(kind="cut", owner=other.uid, uid="c0700000000000b2", cutter={"body": shaft.uid},
                                  operation="subtract", preset=preset("Penetration")["name"]))
    cut = lambda: [r for r in live.live_rows(scene) if r["kind"] == "cut"]  # noqa: E731
    volume = solid_volume(other)
    assert volume == pytest.approx(2.4 - 0.5 * 0.5 * 0.2) and len(cut()) == 1
    steps = len(h.undo_stack)
    reply = commands.run(h, "junction_adopt", junction=j["uid"], rows=["c0700000000000b2"])
    assert not reply["ok"] and reply["rejected"] == "Cannot adopt it: it cuts something outside this meeting"
    assert len(h.undo_stack) == steps and len(h.redo_stack) == 0
    assert len(cut()) == 1 and "junction" not in cut()[0] and solid_volume(other) == pytest.approx(volume)
    apply(h, "undo")
    assert cut() == [] and solid_volume(other) == pytest.approx(2.4)
    apply(h, "redo")
    assert len(cut()) == 1 and solid_volume(other) == pytest.approx(volume)


def column(h, at=(1, 1), key="en/he200a", **more):
    p = steel_column()
    apply(h, "create_sweeps", sweeps=[dict(preset_uid=p["uid"], preset_definition=p, profile=steel(key),
                                           path=dict(points=[[at[0], at[1], 0], [at[0], at[1], 3]]), **more)])
    return h.scene.groups[-1]


def pad(h, host, **more):
    p = preset("Pad footing")
    row = live.live_sweep(h.scene, host.uid)["uid"]
    anchor = {"to": {"sweep": row, "at": "start"}}
    apply(h, "create_sweeps", sweeps=[dict(preset_uid=p["uid"], attachment="centre",
                                           path=dict(ends=[anchor, anchor], dir=[0, 0, -1]),
                                           fit=dict(host={"sweep": row, "at": "start"}), **more)])
    return h.scene.groups[-1]


def test_pad_footing_is_sized_from_its_column_and_follows_it(tmp_path):
    scene = Scene()
    h = History(scene)
    col = column(h)
    foot = pad(h, col)
    row = live.live_sweep(scene, foot.uid)
    # HE 200 A is 200 wide and 190 deep: the pad is 400 larger each way.
    assert row["profile"]["params"] == {"width": 600, "height": 590}
    assert row["fit"]["values"] == {"profile.params.height": "host.height + 400", "profile.params.width": "host.width + 400"}
    lo, hi = bounds(foot)
    assert lo == pytest.approx([0.7, 0.705, -0.4]) and hi == pytest.approx([1.3, 1.295, 0])
    assert foot.ifc["class"] == "IfcFooting" and foot.ifc["predefined_type"] == "PAD_FOOTING" and foot.name == "PF-1"
    # A larger column: the pad follows, in the same step.
    crow = live.live_sweep(scene, col.uid)
    apply(h, "live_set", row=dict(crow, profile=steel("en/ipe240")))
    other = live.live_sweep(scene, foot.uid)["profile"]["params"]
    assert other["width"] == pytest.approx(520) and other["height"] == pytest.approx(640)
    lo, hi = bounds(foot)
    assert lo == pytest.approx([0.74, 0.68, -0.4]) and hi == pytest.approx([1.26, 1.32, 0])
    apply(h, "undo")
    assert live.live_sweep(scene, foot.uid)["profile"]["params"] == {"width": 600, "height": 590}
    # The column moves: so does its pad.
    apply(h, "move_placements", placements=[col.uid], delta=[2, 0, 0])
    assert bounds(foot)[0][0] == pytest.approx(2.7)
    path = tmp_path / "pad.built"
    igz.save_scene(scene, path)
    opened = Scene()
    igz.load_into(opened, path)
    assert live.live_sweep(opened, opened.groups[1].uid)["fit"]["host"]["sweep"] == crow["uid"]
    assert bounds(opened.groups[1])[0][0] == pytest.approx(2.7)


def test_a_fitted_size_edited_by_hand_stops_following_the_host():
    scene = Scene()
    h = History(scene)
    col = column(h)
    foot = pad(h, col)
    row = live.live_sweep(scene, foot.uid)
    row["profile"]["params"]["width"] = 900
    apply(h, "live_set", row=row)
    row = live.live_sweep(scene, foot.uid)
    assert row["profile"]["params"]["width"] == 900
    assert row["fit"]["values"] == {"profile.params.height": "host.height + 400"}
    crow = live.live_sweep(scene, col.uid)
    apply(h, "live_set", row=dict(crow, profile=steel("en/ipe240")))
    after = live.live_sweep(scene, foot.uid)["profile"]["params"]
    assert after["width"] == 900 and after["height"] == pytest.approx(640)
    # A host whose section no longer resolves leaves the footing as it was.
    apply(h, "live_set", row=dict(crow, profile=dict(library="en/no-such-size")))
    assert live.live_sweep(scene, foot.uid)["profile"]["params"]["height"] == pytest.approx(640)
    assert solid_volume(foot) == pytest.approx(0.9 * 0.64 * 0.4)
    bad = dict(row, fit=dict(row["fit"], values={"segments": "host.width"}))
    reply = commands.run(h, "live_set", row=bad)
    assert not reply["ok"] and "segments" in reply["rejected"]


def test_strip_footing_runs_under_its_wall_and_follows_its_length_and_thickness():
    scene = Scene()
    h = History(scene)
    wall_preset = preset("Wall")
    apply(h, "create_sweeps", sweeps=[dict(preset_uid=wall_preset["uid"], path=dict(points=[[0, 0, 0], [4, 0, 0]]))])
    wall = scene.groups[0]
    wrow = live.live_sweep(scene, wall.uid)
    assert wrow["attachment"] == "bottom_left"
    wlo, whi = bounds(wall)
    p = preset("Strip footing")
    ends = [{"to": {"sweep": wrow["uid"], "at": end}, "offset": -150, "extend": True} for end in ("start", "end")]
    values = dict(p["values"]["fit"]["values"], **{"shift.x": "host.width * 0.5"})
    apply(h, "create_sweeps", sweeps=[dict(preset_uid=p["uid"], path=dict(ends=ends), attachment="top",
                                           fit=dict(host={"sweep": wrow["uid"], "at": "start"}, values=values))])
    foot = scene.groups[1]
    row = live.live_sweep(scene, foot.uid)
    assert row["profile"]["params"]["width"] == 500 and row["shift"] == [100, 0]
    lo, hi = bounds(foot)
    # 150 past each end and each face of the 200 wall, 250 deep, under it.
    assert lo == pytest.approx([-0.15, wlo[1] - 0.15, -0.25]) and hi == pytest.approx([4.15, whi[1] + 0.15, 0])
    assert foot.ifc["predefined_type"] == "STRIP_FOOTING"
    # The wall grows longer and thicker.
    wrow["path"]["points"][1] = [6, 0, 0]
    wrow["profile"] = dict(library="basic/wall", params=dict(width=300))
    apply(h, "live_set", row=wrow)
    lo, hi = bounds(foot)
    wlo, whi = bounds(wall)
    assert whi[1] - wlo[1] == pytest.approx(0.3)
    assert lo == pytest.approx([-0.15, wlo[1] - 0.15, -0.25]) and hi == pytest.approx([6.15, whi[1] + 0.15, 0])


def test_presets_keep_their_extrusion_edge_and_fit_through_a_pack_roundtrip():
    scene = Scene()
    h = History(scene)
    mine = dict(preset("Raft slab"), uid="5" * 16, name="Garage raft", pack="My presets")
    mine["values"] = dict(mine["values"], depth=180)
    apply(h, "preset_update", rows=[mine])
    stored = preset("Garage raft", scene)
    assert stored["values"]["depth"] == 180 and stored["values"]["edge"]["shape"] == "downstand"
    apply(h, "create_sweeps", sweeps=[dict(preset_uid=mine["uid"], path=dict(points=[[0, 0, 0]], dir=[0, 0, -1]))])
    g = scene.groups[0]
    assert live.live_sweep(scene, g.uid)["path"]["depth"] == 180
    assert bounds(g)[0][2] == pytest.approx(-0.53)
    bad = dict(mine, values=dict(mine["values"], thickness=3))
    assert not commands.run(h, "preset_update", rows=[bad])["ok"]


@pytest.mark.perf
def test_footing_budgets_on_a_grid_of_columns():
    """200 columns on their pads: a column's change re-evaluates its own pad
    and no other, within the member budget (general-base §3.4), and a slab
    drawn into that model is named and made within it too."""
    import statistics
    import time

    scene = Scene()
    h = History(scene)
    column_preset, pad_preset = steel_column(), preset("Pad footing")
    grid = [(x * 5.0, y * 5.0) for x in range(20) for y in range(10)]
    apply(h, "create_sweeps", sweeps=[dict(preset_uid=column_preset["uid"], preset_definition=column_preset, profile=steel("en/he200a"), path=dict(points=[[x, y, 0], [x, y, 3]]))
                                      for x, y in grid])
    columns = list(scene.groups)
    rows = [live.live_sweep(scene, g.uid) for g in columns]
    pads = []
    for row in rows:
        anchor = {"to": {"sweep": row["uid"], "at": "start"}}
        pads.append(dict(preset_uid=pad_preset["uid"], attachment="centre", path=dict(ends=[anchor, anchor], dir=[0, 0, -1]),
                         fit=dict(host={"sweep": row["uid"], "at": "start"})))
    apply(h, "create_sweeps", sweeps=pads)
    footings = list(scene.groups)[len(columns):]
    assert len(footings) == 200 and footings[-1].name == "PF-200"
    assert all(r["state"] == "ok" for r in live.live_rows(scene))
    meshes = [g.mesh for g in footings]
    samples = []
    for i in range(15):
        size = steel("en/ipe240" if i % 2 == 0 else "en/he200a")
        start = time.perf_counter()
        apply(h, "live_set", row=dict(rows[7], profile=size))
        samples.append((time.perf_counter() - start) * 1000)
    changed = [i for i, (g, m) in enumerate(zip(footings, meshes)) if g.mesh is not m]
    assert changed == [7]
    assert live.live_sweep(scene, footings[7].uid)["profile"]["params"]["width"] == pytest.approx(520)
    edit = statistics.median(samples)
    # A step that touches no host leaves every footing alone.
    meshes = [g.mesh for g in footings]
    samples = []
    for i in range(9):
        start = time.perf_counter()
        slab(h, width=3000 + i)
        samples.append((time.perf_counter() - start) * 1000)
    assert all(g.mesh is m for g, m in zip(footings, meshes))
    assert scene.groups[-1].name == "SL-9"
    draw = statistics.median(samples)
    print(f"200 columns on pads: one column edit with its pad {edit:.2f} ms, a slab drawn into the model {draw:.2f} ms")
    assert edit < 16 and draw < 16, (edit, draw)
