# SPDX-License-Identifier: GPL-3.0-or-later
import pytest

from built_core.history import History
from built_core.scene import Scene
from built_core.solids import solid_volume
from formats import igz
from tests.test_persistent_ids import all_ids
from tests.test_sketch_rows import apply, extent, member, rows_of


def make_member(history, points):
    apply(history, "create_sweeps", sweeps=[member({"library": "basic/timber"}, points)])


def test_oblique_cut_edit_disable_undo_save_open_and_release(tmp_path):
    scene = Scene()
    h = History(scene)
    make_member(h, [[0, 0, 0], [2, 0, 0]])
    target = scene.groups[0]
    base = target.mesh
    row = dict(kind="cut", owner=target.uid, uid="c070000000000001",
               operation="trim", cutter={"plane": {"origin": [1, 0, 0], "normal": [1, 1, 0]}})
    apply(h, "live_set", row=row)
    cut_ids = all_ids(target.mesh)
    assert extent(target)[0] == pytest.approx(1.0225)
    assert extent(target)[1:] == [0.045, 0.145]
    apply(h, "undo")
    assert target.mesh is base and not rows_of(scene, "cut")
    apply(h, "redo")
    assert all_ids(target.mesh) == cut_ids
    apply(h, "live_set", row=dict(row, on=False))
    assert extent(target)[0] == 2
    apply(h, "undo")
    path = tmp_path / "end.built"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert rows_of(back, "cut")[0]["state"] == "ok"
    h = History(back)
    row["cutter"]["plane"]["origin"] = [1.25, 0, 0]
    apply(h, "live_set", row=row)
    assert all_ids(back.groups[0].mesh) == cut_ids
    assert extent(back.groups[0])[0] == pytest.approx(1.2725)
    apply(h, "live_release", uid=row["uid"])
    assert not rows_of(back, "cut")
    assert extent(back.groups[0])[0] == pytest.approx(1.2725)


@pytest.mark.parametrize("operation", ["union", "subtract", "trim", "intersect"])
def test_body_operations_preserve_both_placements(operation):
    scene = Scene()
    h = History(scene)
    make_member(h, [[0, 0, 0], [2, 0, 0]])
    make_member(h, [[1, -1, 0], [1, 1, 0]])
    target, body = scene.groups
    original = solid_volume(target)
    tool = solid_volume(body)
    overlap = 0.045 ** 2 * 0.145
    apply(h, "live_set", row=dict(kind="cut", owner=target.uid, uid="c070000000000002",
                                  operation=operation, cutter={"body": body.uid}))
    assert rows_of(scene, "sweep")[0]["state"] == "ok"
    expected = {"union": original + tool - overlap, "intersect": overlap}.get(operation, original - overlap)
    assert solid_volume(target) == pytest.approx(expected)
    assert solid_volume(body) == pytest.approx(tool)
    apply(h, "undo")
    assert solid_volume(target) == pytest.approx(original)


def test_end_plate_follows_a_separate_host(tmp_path):
    scene = Scene()
    h = History(scene)
    make_member(h, [[0, 0, 0], [2, 0, 0]])
    make_member(h, [[0, 1, 0], [0, 2, 0]])
    target, host = scene.groups
    apply(h, "move_placements", placements=[host.uid], delta=[1, 0, 0])
    row = dict(kind="cut", owner=target.uid, uid="c070000000000003", operation="trim",
               cutter={"plane": {"normal": [1, 0, 0]}},
               anchor={"ref": {"placement": host.uid}, "offset": 20})
    apply(h, "live_set", row=row)
    assert extent(target)[0] == pytest.approx(1.02)
    ids = all_ids(target.mesh)
    apply(h, "move_placements", placements=[host.uid], delta=[0.3, 0, 0])
    assert extent(target)[0] == pytest.approx(1.32)
    assert all_ids(target.mesh) == ids
    apply(h, "undo")
    assert extent(target)[0] == pytest.approx(1.02)
    apply(h, "delete_placements", placements=[host.uid])
    assert rows_of(scene, "sweep")[0]["state"] == "ref_lost"
    assert extent(target)[0] == pytest.approx(1.02)


def test_plain_solid_source_survives_save_open(tmp_path):
    from tests.test_geometry_arrays import cube
    scene = Scene()
    h = History(scene)
    target = cube()
    body = cube()
    scene.groups.extend([target, body])
    apply(h, "move_placements", placements=[body.uid], delta=[0.5, 0, 0])
    original = solid_volume(target)
    row = dict(kind="cut", owner=target.uid, uid="c070000000000005",
               cutter={"body": body.uid}, operation="subtract")
    apply(h, "live_set", row=row)
    assert solid_volume(target) == pytest.approx(original / 2)
    file = tmp_path / "plain.built"
    igz.save_scene(scene, file)
    back = Scene()
    igz.load_into(back, file)
    apply(History(back), "live_set", row=dict(row, on=False))
    assert solid_volume(back.groups[0]) == pytest.approx(original)


def test_section_cutter_uses_sketch_curves_and_keeps_tool_out_of_model():
    from tests.test_sketch_rows import sketch, rect
    scene = Scene()
    h = History(scene)
    make_member(h, [[0, 0, 0], [2, 0, 0]])
    target = scene.groups[0]
    before = solid_volume(target)
    outline = sketch(rect(100, 100))
    outline["use"] = "section"
    for curve in outline["curves"]:
        for point in ("a", "b"):
            x, y = curve[point]
            curve[point] = [x + 1000, y - 50]
    apply(h, "sketch_commit", row=outline)
    apply(h, "live_set", row=dict(kind="cut", owner=target.uid, uid="c070000000000006",
                                  cutter={"section": outline["uid"]}, operation="subtract"))
    assert solid_volume(target) == pytest.approx(before * 0.95)
    assert list(scene.groups) == [target]


def test_plain_plane_and_exact_body_cut_interchange(tmp_path):
    from tests.test_geometry_arrays import cube
    from formats.ifc import save_ifc
    import ifcopenshell
    import ifcopenshell.geom
    from ifcopenshell.util.shape import get_volume

    scene = Scene()
    history = History(scene)
    target = cube()
    target.ifc = {"class": "IfcWall", "name": "Panel"}
    scene.groups.append(target)
    row = dict(kind="cut", uid="c070000000000007", owner=target.uid, operation="trim",
               cutter={"plane": {"origin": [.5, 0, 0], "normal": [1, 0, 0]}})
    apply(history, "live_set", row=row)
    assert solid_volume(target) == pytest.approx(.5)

    make_member(history, [[0, .5, .5], [1, .5, .5]])
    body = scene.groups[-1]
    apply(history, "live_set", row=dict(row, uid="c070000000000008",
                                       operation="subtract", cutter={"body": body.uid}))
    path = tmp_path / "panel.ifc"
    save_ifc(scene, path)
    model = ifcopenshell.open(path)
    wall, = model.by_type("IfcWall")
    rep, = wall.Representation.Representations
    assert rep.RepresentationType == "CSG"
    assert rep.Items[0].is_a("IfcBooleanResult")
    assert len(wall.HasOpenings) == 1
    shape = ifcopenshell.geom.create_shape(ifcopenshell.geom.settings(), wall)
    assert get_volume(shape.geometry) == pytest.approx(solid_volume(target), abs=1e-7)


def test_ifc_cope_opening_includes_the_clearance(tmp_path):
    import ifcopenshell
    import ifcopenshell.geom
    import numpy as np
    from ifcopenshell.util.shape import get_vertices
    from formats.ifc import save_ifc
    from tests.test_geometry_arrays import cube

    scene = Scene()
    history = History(scene)
    wall = cube()
    wall.ifc = {"class": "IfcWall", "name": "Panel"}
    scene.groups.append(wall)
    make_member(history, [[0, .5, .5], [1, .5, .5]])
    body = scene.groups[-1]
    row = dict(kind="cut", uid="c070000000000019", owner=wall.uid, operation="subtract",
               cutter={"body": body.uid}, clearance=20)
    apply(history, "live_set", row=row)
    path = tmp_path / "cope.ifc"
    save_ifc(scene, path)
    model = ifcopenshell.open(path)
    opening, = model.by_type("IfcOpeningElement")
    shape = ifcopenshell.geom.create_shape(ifcopenshell.geom.settings(), opening)
    size = np.ptp(get_vertices(shape.geometry), axis=0)
    np.testing.assert_allclose(size, np.asarray(extent(body)) + .04, atol=1e-6)


def test_mitred_round_member_preserves_analytic_step_and_ifc(tmp_path):
    import ifcopenshell
    from formats import step
    from formats.ifc import save_ifc

    scene = Scene()
    h = History(scene)
    apply(h, "create_sweeps", sweeps=[member({"library": "en/chs114.3x5"})])
    apply(h, "live_set", row=dict(kind="cut", owner=scene.groups[0].uid, uid="c070000000000009",
                                  operation="trim", cutter={"plane": {"origin": [1., 0., 0.],
                                                                     "normal": [1., 1., 0.]}}))
    path = tmp_path / "mitre.ifc"
    save_ifc(scene, path)
    assert ifcopenshell.open(path).by_type("IfcBooleanClippingResult")
    path = tmp_path / "mitre.step"
    step.save_step(scene, path)
    assert "CYLINDRICAL_SURFACE" in path.read_text()


def test_bisector_reacts_to_member_move_and_copy_has_its_own_cut():
    scene = Scene()
    h = History(scene)
    make_member(h, [[0, 0, 0], [2, 0, 0]])
    make_member(h, [[2, 0, 0], [2, 1, 0]])
    target, other = scene.groups
    cut = dict(kind="cut", owner=target.uid, uid="c070000000000011",
               operation="trim", cutter={"bisector": other.uid})
    apply(h, "live_set", row=cut)
    ids = all_ids(target.mesh)
    assert extent(target)[0] == pytest.approx(2.0225)
    apply(h, "move_placements", placements=[other.uid], delta=[-.4, 0, 0])
    assert extent(target)[0] == pytest.approx(1.6225)
    assert all_ids(target.mesh) == ids
    apply(h, "copy_placements", placements=[target.uid], delta=[0, .1, 0])
    clone = scene.groups[-1]
    original, copied = rows_of(scene, "cut")
    assert original["uid"] == cut["uid"] and copied["uid"] != original["uid"]
    assert copied["owner"] == clone.uid
    apply(h, "live_set", row=dict(copied, on=False))
    assert extent(clone)[0] == 2 and extent(target)[0] == pytest.approx(1.6225)


def test_four_body_cuts_keep_donor_ids_and_identify_a_miss():
    s = Scene()
    h = History(s)
    make_member(h, [[0, 0, 0], [4, 0, 0]])
    target = s.groups[0]
    before = solid_volume(target)
    for i in range(4):
        x = .6 + .8*i
        make_member(h, [[x, -1, 0], [x, 1, 0]])
        body = s.groups[-1]
        apply(h, "live_set", row=dict(kind="cut", uid=f"c0700000000010{i:02x}", owner=target.uid,
                                      cutter={"body": body.uid}, operation="subtract"))
    ids = {r["uid"] for r in rows_of(s, "cut")}
    assert {f.attrs.get("built.cut_face") for f in target.mesh.faces} == {None, *ids}
    assert solid_volume(target) == pytest.approx(before - 4*.045**2*.145)
    apply(h, "move_placements", placements=[body.uid], delta=[30, 0, 0])
    assert solid_volume(target) == pytest.approx(before - 3*.045**2*.145)
    assert [r["state"] for r in rows_of(s, "cut")].count("cut_misses") == 1
    apply(h, "undo")
    assert all(r["state"] == "ok" for r in rows_of(s, "cut"))


def test_hosted_opening_on_a_pitched_wall_follows_its_placement(tmp_path):
    from built_core import Matrix4, Vec3
    from built_core.group import Group
    from tests.test_geometry_arrays import CUBE

    scene = Scene()
    h = History(scene)
    wall = Group(name="Wall")
    wall.ifc = {"class": "IfcWall"}
    wall.xform = Matrix4()
    profile = [(0, 0), (8, 0), (8, 3), (6, 3), (0, 2.7)]
    front, back = [[Vec3(x, y, z) for x, z in profile] for y in [0, .25]]
    wall.mesh.add_face(front)
    wall.mesh.add_face(back[::-1])
    for i in range(len(profile)):
        j = (i + 1) % len(profile)
        wall.mesh.add_face([front[j], front[i], back[i], back[j]])
    scene.groups.append(wall)
    before = solid_volume(wall)
    window = Group(name="Window")
    window.ifc = {"class": "IfcWindow"}
    window.ext = {"library_host": "wall"}
    window.xform = Matrix4()
    for face in CUBE:
        window.mesh.add_face([Vec3(x, .12*y, z) for x, y, z in face])
    apply(h, "place_group", template=window.uid, _pending=window,
          basis=[[1, 0, 0], [0, 1, 0], [0, 0, 1]], offset=[2, 0, 1],
          wall_host=dict(uid=wall.uid, outline=[[2, 0, 1], [3, 0, 1], [3, 0, 2], [2, 0, 2]],
                         normal=[0, -1, 0], initial_offset=0, offset=0, sill=1, flipped=False))
    assert solid_volume(wall) == pytest.approx(before - .25)
    apply(h, "move_placements", placements=[wall.uid], delta=[1, 2, 0])
    assert window.xform.data()[12:15] == (3., 2., 1.)
    path = tmp_path / "pitched.built"
    igz.save_scene(scene, path)
    reopened = Scene()
    igz.load_into(reopened, path)
    assert rows_of(reopened, "cut")[0]["state"] == "ok"
    assert solid_volume(reopened.groups[0]) == pytest.approx(before - .25)


def test_pushpull_on_a_mitre_face_edits_its_live_joint(monkeypatch):
    from tests.test_fuzz_engine import _push, _StubVP

    monkeypatch.setattr(_StubVP, "notify_scene_changed", lambda self: None, raising=False)

    scene = Scene()
    h = History(scene)
    make_member(h, [[0, 0, 0], [2, 0, 0]])
    make_member(h, [[2, 0, 0], [2, 1, 0]])
    target, other = scene.groups
    apply(h, "live_set", row=dict(kind="cut", owner=target.uid, uid="c070000000000012",
                                  operation="trim", cutter={"bisector": other.uid}))
    face = max(target.mesh.faces, key=lambda f: f.centroid().x())
    _push(scene, h, face, .01, group=target)
    assert rows_of(scene, "cut")[0]["clearance"] == pytest.approx(10)
    assert extent(target)[0] == pytest.approx(2.0225 + 2**.5*.01)
    apply(h, "undo")
    assert extent(target)[0] == pytest.approx(2.0225)


def test_multiple_targets_share_a_history_step(tmp_path):
    from built_core import command_protocol as commands

    scene = Scene()
    h = History(scene)
    for y in [0, .5]:
        make_member(h, [[0, y, 0], [2, y, 0]])
    targets = list(scene.groups)
    cuts = [dict(kind="cut", uid=f"c07000000000200{i}", owner=g.uid,
                 cutter={"plane": {"origin": [1, 0, 0], "normal": [1, 0, 0]}})
            for i, g in enumerate(targets)]
    apply(h, "live_set", row=cuts)
    ids = [all_ids(g.mesh) for g in targets]
    assert [extent(g)[0] for g in targets] == [1., 1.]
    assert len(rows_of(scene, "cut")) == 2
    apply(h, "undo")
    assert [extent(g)[0] for g in targets] == [2., 2.]
    assert rows_of(scene, "cut") == []
    apply(h, "redo")
    assert [all_ids(g.mesh) for g in targets] == ids
    scene.show_all_cutters = True
    assert len(commands.live_cut_display(scene)) == 2
    scene.show_all_cutters = False
    scene.select([targets[1]])
    handles, = commands.live_cut_display(scene)
    assert handles["row"]["owner"] == targets[1].uid
    assert [g["kind"] for g in handles["grips"]] == ["offset", "angle"]
    path = tmp_path / "pair.built"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    assert [all_ids(g.mesh) for g in back.groups] == ids


@pytest.mark.parametrize("corner", ["butt", "mitre"])
@pytest.mark.parametrize("span,end", [(0, "start"), (0, "end"), (1, "start"), (1, "end")])
def test_fitting_individual_span_end(corner, span, end):
    scene = Scene()
    h = History(scene)
    spec = member({"library": "basic/timber"}, [[0, 0, 0], [2, 0, 0], [2, 2, 0]])
    apply(h, "create_sweeps", sweeps=[dict(spec, corner=corner)])
    g = scene.groups[0]
    point = .4 if end == "start" else 1.5
    normal = [1, .2, 0] if span == 0 else [.2, 1, 0]
    origin = [point, 0, 0] if span == 0 else [2, point, 0]
    cut = dict(kind="cut", uid="c070000000002010", owner=g.uid, span=span, end=end,
               cutter={"plane": {"origin": origin, "normal": normal}})
    apply(h, "live_set", row=cut)
    assert rows_of(scene, "cut")[0]["state"] == "ok"
    planes = g.ext["built.live"]["butt_planes"]
    assert len(planes) == 2
    assert planes[span][int(end == "end")][0] == pytest.approx(origin)
    ids = all_ids(g.mesh)
    apply(h, "live_set", row=dict(cut, offset=10))
    assert all_ids(g.mesh) == ids
    apply(h, "live_set", row=dict(cut, span=5))
    assert rows_of(scene, "cut")[0]["state"] == "end_unresolved"
    assert all_ids(g.mesh) == ids


def test_shift_pick_toggles_targets_and_keeps_each_span():
    import json
    from built_core import ToolMachine

    class Host:
        def __init__(self):
            self.commands = []

        def call(self, name, arg):
            if name == "cut_pick":
                index = int(arg[0])
                return {"placement": str(index), "span": index, "end": "start"}

        def run(self, name, payload):
            self.commands.append((name, json.loads(payload)))
            return True, 1, 0, None

    machine = ToolMachine("cut")
    host = Host()
    for target, shift in [(1, False), (2, True), (3, True), (2, True), (4, False)]:
        machine.event(dict(type="click", px=(target, 0), mods=0x02000000 if shift else 0), host)
    (name, args), = host.commands
    assert name == "live_set"
    assert [(r["owner"], r["span"], r["cutter"]) for r in args["row"]] == [
        ("1", 1, {"body": "4"}), ("3", 3, {"body": "4"})]


@pytest.mark.parametrize("kind, cutter, anchor, expected", [
    ("angle", {"plane": {}}, None, 30),
    ("offset", {"section": "section"}, None, 30),
    ("offset", {"plane": {}}, {"ref": {"placement": "host"}}, 30),
])
def test_grip_typed_value_edits_its_own_field(kind, cutter, anchor, expected):
    import json
    from built_core import ToolMachine

    class Host:
        def call(self, name, arg):
            if name == "cut_grip":
                return dict(kind=kind, at=[0, 0, 0], axis=[0, 0, 1], radius=.2,
                            row=dict(cutter=cutter, anchor=anchor))

        def run(self, name, payload):
            assert name == "live_set"
            self.edited = json.loads(payload)["row"]
            return True, 1, 0, None

    host = Host()
    machine = ToolMachine("cut")
    machine.event(dict(type="click", world=[0, 0, 0]), host)
    assert machine.get("editing_grip")
    machine.event(dict(type="value", value=30 if kind == "angle" else .03), host)
    edited = host.edited
    if kind == "angle":
        assert edited["cutter"]["plane"]["angle"] == expected
    elif anchor:
        assert edited["anchor"]["offset"] == expected
        assert edited["anchor"]["ref"] == anchor["ref"]
    else:
        assert edited["offset"] == expected


def test_tree_cut_eye_and_release_use_the_target_history():
    from tests.test_project_drawing_filters import Project

    project = Project()
    h = project.win.viewport.history
    scene = project.scene
    make_member(h, [[0, 0, 0], [2, 0, 0]])
    target = scene.groups[0]
    uid = "c070000000004001"
    apply(h, "live_set", row=dict(kind="cut", owner=target.uid, uid=uid,
                                  cutter={"plane": {"origin": [1, 0, 0]}}))
    rows = project.rows()
    owner_key = "i:" + target.uid
    if owner_key not in rows:
        rows = project.action("toggle", "n:")
    assert rows[owner_key][1][7]
    rows = project.action("toggle", owner_key)
    key = "cut:" + uid
    assert rows[key][1][0] == rows[owner_key][1][0] + 1
    assert project.action("eye", key)[key][1][5] == 0
    assert extent(target)[0] == pytest.approx(2)
    project.action("eye", key)
    assert extent(target)[0] == pytest.approx(1)
    assert key not in project.action("releasecut", key)
    assert extent(target)[0] == pytest.approx(1)
    apply(h, "undo")
    assert rows_of(scene, "cut")[0]["uid"] == uid
