# SPDX-License-Identifier: GPL-3.0-or-later
import json
from pathlib import Path

import pytest
from built_core.history import History
from built_core.protocol import queries
from built_core.scene import Scene
from formats import igz
from tests.test_live_recipes import apply
from tests.test_persistent_ids import all_ids

CATALOGUE = json.loads((Path(__file__).parents[1] / "resources/profiles/starter.builtprofile").read_text())


def spec(key):
    return dict(profile=dict(library=key), path=dict(points=[[0, 0, 0], [2, 0, 0]]))


@pytest.mark.parametrize("size", CATALOGUE["table"]["sizes"], ids=lambda s: s["label"])
def test_profile_roundtrip_and_role_ids(tmp_path, size):
    scene = Scene()
    h = History(scene)
    apply(h, "create_sweeps", sweeps=[spec(size["key"])])
    row, = queries.live_rows(scene)
    assert row["state"] == "ok"
    g, = scene.groups
    assert g.ifc["class"] == ("IfcWall" if size["key"] == "basic/wall" else "IfcBeam")
    old = g.mesh
    ids = all_ids(old)
    apply(h, "live_set", row=dict(row, path=dict(points=[[0, 0, 0], [4, 2, 1]])))
    assert queries.live_rows(scene)[0]["state"] == "ok"
    assert all_ids(g.mesh) == ids
    apply(h, "undo")
    assert g.mesh is old
    apply(h, "redo")
    saved = tmp_path / "member.built"
    igz.save_scene(scene, saved)
    back = Scene()
    igz.load_into(back, saved)
    assert queries.live_rows(back)[0]["state"] == "ok"
    assert all_ids(back.groups[0].mesh) == ids
    previous = g.mesh
    apply(h, "live_set", row=dict(row, path=dict(points=[[0, 0, 0]])))
    assert g.mesh is previous and queries.live_rows(scene)[0]["state"] == "invalid_path"


def test_ifc_body_and_reference_view(tmp_path):
    import ifcopenshell
    import ifcopenshell.geom
    from formats.ifc.export import save_ifc
    from core import bim_registry

    scene = Scene()
    apply(History(scene), "create_sweeps", sweeps=[spec("en/ipe240"), spec("en/chs114.3x5")])
    out = tmp_path / "members.ifc"
    save_ifc(scene, out)
    model = ifcopenshell.open(out)
    assert len(model.by_type("IfcExtrudedAreaSolid")) == 2
    for member in model.by_type("IfcBeam"):
        assert len(ifcopenshell.geom.create_shape(ifcopenshell.geom.settings(), member).geometry.verts)
    registry = bim_registry.get(scene)
    registry.setdefault("export_profiles", {})["ref"] = {"view": "ReferenceView"}
    scene.plugin_data["bim_registry"] = registry
    save_ifc(scene, out, profile="ref")
    assert len(ifcopenshell.open(out).by_type("IfcPolygonalFaceSet")) == 2


def test_anchored_end_reacts_to_host_then_release_bakes():
    from tests.test_live_recipes import setup_groups, box
    from tests.test_persistent_ids import face_id

    scene, h, (host, beam) = setup_groups(2)
    apply(h, "live_set", row=box(host, size=[3, 2, 2]))
    face = next(f for f in host.mesh.faces if all(v.position_tuple()[0] == 3 for v in f.loop))
    target = {"path": [host.uid], "id": face_id(host.mesh, face)}
    row = dict(spec("basic/timber"), kind="sweep", owner=beam.uid, uid="0000000000000002")
    row["path"] = dict(points=[[0, 1, 1], [2, 1, 1]], ends=[
        [0, 1, 1], dict(to=target, offset=25, extend=True)])
    apply(h, "live_set", row=row)
    assert max(v.position_tuple()[0] for v in beam.mesh.vertices) == pytest.approx(2.975)
    before = beam.mesh
    apply(h, "move_placements", placements=[host.uid], delta=[1, 0, 0])
    assert max(v.position_tuple()[0] for v in beam.mesh.vertices) == pytest.approx(3.975)
    apply(h, "undo")
    assert beam.mesh is before
    apply(h, "live_release", uid=row["uid"])
    assert beam.mesh is before and len(queries.live_rows(scene)) == 1


def test_curved_chain_owns_exact_supports_and_mitre_trims():
    scene = Scene()
    sweep = spec("en/chs114.3x5")
    sweep["path"]["points"].append([2, 3, 0])
    apply(History(scene), "create_sweeps", sweeps=[sweep])
    mesh = scene.groups[0].mesh
    assert queries.live_rows(scene)[0]["state"] == "ok"
    assert sum(bool(mesh._store.exact_face(f.key)) for f in mesh.faces) == 96
    assert any("ellipse" in (mesh._store.exact_edge(e.key) or "") for e in mesh.edges)


def test_wall_stands_on_its_path():
    scene = Scene()
    h = History(scene)
    apply(h, "create_sweeps", sweeps=[dict(spec("basic/wall"), attachment="bottom_left")])
    wall, = scene.groups
    positions = [v.position_tuple() for v in wall.mesh.vertices]
    assert min(p[2] for p in positions) == 0.
    assert max(p[2] for p in positions) == pytest.approx(2.7)
    apply(h, "undo")
    assert not scene.groups and not queries.live_rows(scene)
    apply(h, "redo")
    assert scene.groups[0] is wall


@pytest.mark.perf
def test_member_edit_budget():
    import statistics
    import time

    scene = Scene()
    h = History(scene)
    apply(h, "create_sweeps", sweeps=[spec("en/ipe240")])
    row, = queries.live_rows(scene)
    samples = []
    for i in range(15):
        row["path"]["points"][1][0] += .05
        start = time.perf_counter()
        apply(h, "live_set", row=row)
        samples.append((time.perf_counter() - start) * 1000)
    assert queries.live_rows(scene)[0]["state"] == "ok"
    assert statistics.median(samples) < 16, samples


def test_step_reopens_the_tube_as_an_exact_solid(tmp_path):
    from formats import step
    scene = Scene()
    apply(History(scene), "create_sweeps", sweeps=[spec("en/chs114.3x5")])
    path = tmp_path / "tube.step"
    step.save_step(scene, path)
    reopened = step.prepare_step(path)
    assert reopened["exact"] == 1 and not reopened["problems"]
    assert "CYLINDRICAL_SURFACE(" in path.read_text()


def test_drawn_profile_is_shared_editable_and_roundtrips(tmp_path):
    """Library drawings, document sketches and face captures share curve IDs."""
    from formats.ifc.export import save_ifc
    import ifcopenshell

    s = Scene()
    h = History(s)
    drawing = dict(name="Offset tube", use="profile", curves=[
        dict(id=2, kind="circle", c=[30, 50], r=20., segments=24),
        dict(id=7, kind="circle", c=[30, 50], r=15., segments=24)],
        points=[dict(name="pin", at=[30, 50])], dims=[], next_id=8)
    sweep = dict(spec(""), profile=dict(drawing=drawing), attachment="pin")
    apply(h, "create_sweeps", sweeps=[sweep])
    sketch, member = queries.live_rows(s)
    assert sketch["name"] == drawing["name"] and sketch["points"] == drawing["points"]
    assert member["state"] == "ok" and member["profile"]["sketch"] == sketch["uid"]
    apply(h, "create_sweeps", sweeps=[dict(sweep, profile=dict(sketch=sketch["uid"]))])
    meshes = [g.mesh for g in s.groups]
    ids = [all_ids(mesh) for mesh in meshes]
    sketch["curves"][0]["r"] = 22.5
    apply(h, "live_set", row=sketch)
    assert all(g.mesh is not old for g, old in zip(s.groups, meshes))
    assert [all_ids(g.mesh) for g in s.groups] == ids
    assert all(row['state'] == 'ok' for row in queries.live_rows(s))
    apply(h, "undo")
    assert [g.mesh for g in s.groups] == meshes
    apply(h, "redo")
    path = tmp_path / "sketch.built"
    igz.save_scene(s, path)
    reopened = Scene()
    igz.load_into(reopened, path)
    assert queries.live_rows(reopened) == queries.live_rows(s)
    out = tmp_path / "drawn.ifc"
    save_ifc(reopened, out)
    assert len(ifcopenshell.open(out).by_type("IfcArbitraryProfileDefWithVoids")) == 2


def test_ifc_mitre_body_volume(tmp_path):
    import ifcopenshell
    import ifcopenshell.geom
    from ifcopenshell.util.shape import get_volume
    from formats.ifc.export import save_ifc

    scene = Scene()
    member = spec("basic/timber")
    member["path"]["points"].append([2, 3, 0])
    apply(History(scene), "create_sweeps", sweeps=[member])
    out = tmp_path / "corner.ifc"
    save_ifc(scene, out)
    model = ifcopenshell.open(out)
    assert len(model.by_type("IfcBooleanClippingResult")) == 2
    shape = ifcopenshell.geom.create_shape(ifcopenshell.geom.settings(), model.by_type("IfcBeam")[0])
    assert get_volume(shape.geometry) == pytest.approx(.045 * .145 * 5, rel=1e-7)
