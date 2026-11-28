# SPDX-License-Identifier: GPL-3.0-or-later
"""Junction polish (hands-on 2026-11-28): members placed from Library ›
Profiles in a Steel fabrication model, their axes, the plate and bolts made
at a knee, the frame copied, the knee saved as a detail and applied."""
from math import isclose

import pytest

from built_core import Vec3, command_protocol as live
from built_core.history import History
from built_core.scene import Scene
from tests.test_live_recipes import apply


def xyz(v):
    return tuple(c() if callable(c) else c for c in (v.x, v.y, v.z))


def world(g, p):
    return xyz(g.xform.map(Vec3(*p)))


def steel_model():
    """A new model from the Steel fabrication template: new items go to
    Steel › Beams."""
    from built_render import wgpu_start as ws
    scene = Scene()
    t = next(t for t in ws.TEMPLATES if t[0] == "steel")
    assert ws.apply_structure(scene, t[3], t[4])
    return scene, History(scene)


def member(library, a, b, **extra):
    # Catalogue sizes come from the add-on packs (addons/), as a user's
    # installed Library gives them.
    from tests.addon_cases import profile
    return dict(profile=profile(library), path=dict(points=[list(a), list(b)]), **extra)


def by_name(scene, name):
    return [g for g in scene.groups if g.name == name]


def test_a_member_has_its_origin_and_axes_on_its_profile():
    scene, history = steel_model()
    apply(history, "create_sweeps", sweeps=[member("en/he200a", (2, 1, 0), (2, 1, 3)),
                                            member("en/ipe240", (2, 1.095, 2.8), (2, 5.095, 2.8))])
    column, beam = scene.groups[-2:]
    # The origin is the path start; X runs along the member, Y and Z are the
    # section's width and height axes.
    assert world(column, (0, 0, 0)) == pytest.approx((2, 1, 0))
    assert world(column, (1, 0, 0)) == pytest.approx((2, 1, 1))
    assert world(beam, (0, 0, 0)) == pytest.approx((2, 1.095, 2.8))
    assert world(beam, (1, 0, 0)) == pytest.approx((2, 2.095, 2.8))
    assert world(beam, (0, 0, 1)) == pytest.approx((2, 1.095, 3.8))  # the web stands up
    # The box hugs the member: its local extent is the length by the section.
    pts = [v.position_tuple() for v in beam.mesh.vertices]
    size = [max(p[i] for p in pts) - min(p[i] for p in pts) for i in range(3)]
    assert size == pytest.approx([4.0, 0.12, 0.24], abs=1e-6)
    row = live.live_sweep(scene, beam.uid)
    assert row["path"]["definition_space"] and row["path"]["points"][0] == [0, 0, 0]
    # Undo and redo keep the frame.
    apply(history, "undo")
    apply(history, "redo")
    beam = scene.groups[-1]
    assert world(beam, (1, 0, 0)) == pytest.approx((2, 2.095, 2.8))


def test_a_plate_keeps_its_class_and_files_under_plates_not_the_active_node():
    scene, history = steel_model()
    plate = next(p for p, _ in live.preset_catalog(scene) if p["name"] == "Plate")
    apply(history, "create_sweeps", sweeps=[
        member("en/he200a", (0, 0, 0), (0, 0, 3)),
        dict(preset_uid=plate["uid"], profile={"library": "basic/flat", "params": {"width": 180, "height": 240}},
             path=dict(points=[[0, 0.095, 2.8], [0, 0.11, 2.8]]))])
    column, pl = scene.groups[-2:]
    assert column.ifc["class"] == "IfcBeam"  # the active node files what is drawn
    assert pl.ifc["class"] == "IfcPlate"
    from built_core import structure
    assert structure.node_of(pl) == "steel.plates"


def test_a_copied_frame_anchors_its_beam_to_its_own_copied_column():
    scene, history = steel_model()
    from tests.test_junction_capture import facing_face
    apply(history, "create_sweeps", sweeps=[member("en/he200a", (0, 0, 0), (0, 0, 3))])
    column = scene.groups[-1]
    face = facing_face(column, (0, 1, 0))
    apply(history, "create_sweeps", sweeps=[dict(profile=member("en/ipe240", (0, 0, 0), (1, 0, 0))["profile"], path=dict(
        points=[[0, 0.095, 2.8], [0, 4.095, 2.8]],
        ends=[{"to": {"path": [column.uid], "id": face}, "extend": True}, [0, 4.095, 2.8]]))])
    beam = scene.groups[-1]
    reply = apply(history, "copy_placements", placements=[column.uid, beam.uid], delta=[5, 0, 0], count=2)
    copies = reply["placements"]
    assert len(copies) == 4
    groups = scene.groups_by_uid()
    for k in range(2):
        col, bm = groups[copies[2 * k]], groups[copies[2 * k + 1]]
        row = live.live_sweep(scene, bm.uid)
        anchor = row["path"]["ends"][0]["to"]["path"]
        assert anchor == [col.uid], (k, anchor)
    # Moving the first column no longer drags the copies' beams.
    before = [c for u in copies[1::2] for c in world(groups[u], (0, 0, 0))]
    apply(history, "move_placements", placements=[column.uid], delta=[0, -0.5, 0])
    after = [c for u in copies[1::2] for c in world(scene.groups_by_uid()[u], (0, 0, 0))]
    assert after == pytest.approx(before)
