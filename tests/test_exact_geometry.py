# SPDX-License-Identifier: GPL-3.0-or-later
"""Exact support evaluation and the whole-interval trim admission boundary."""
import json
import math
from collections import Counter

import pytest

from built_core.command_protocol import ExactGeometry


def frame(z=0):
    return dict(origin=[0, 0, z], axis=[0, 0, 1], reference=[1, 0, 0])


def cylinder():
    t = math.tau
    curves = [dict(type="circle", frame=frame(z), radius=1) for z in (0, 2)]
    curves.append(dict(type="line", origin=[1, 0, 0], direction=[0, 0, 1]))
    edges = [
        dict(id=10+k, curve=k, vertices=ends, interval=[0, stop])
        for k, (ends, stop) in enumerate([([0, 0], t), ([1, 1], t), ([0, 1], 2)])
    ]
    vertices = [[0, 0], [t, 0], [t, 2], [0, 2]]
    ring = [dict(edge=edge, reversed=reverse, tolerance=0.005,
                 uv=[vertices[i], vertices[(i+1) % 4]])
            for i, (edge, reverse) in enumerate([(10, False), (12, False),
                                                  (11, True), (12, True)])]
    return dict(curves=curves, surfaces=[dict(type="cylinder", frame=frame(), radius=1)],
                topology=dict(tolerance=0.01, vertices={0: [1, 0, 0], 1: [1, 0, 2]},
                              edges=edges, faces=[dict(id=20, surface=0, reversed=False,
                                                       loops=[ring])]))


def test_closed_cylinder_edge_and_two_seam_uses():
    geometry = ExactGeometry(json.dumps(cylinder()))
    mesh = json.loads(geometry.tessellate())
    assert set(mesh["face_ids"]) == {20}
    incidence = Counter()
    for a, b, c in mesh["triangles"]:
        for i, j in [(a, b), (b, c), (c, a)]:
            incidence[tuple(sorted([i, j]))] += 1
        tri = [mesh["points"][i] for i in (a, b, c)]
        center = [sum(p[k] for p in tri)/3 for k in range(3)]
        assert abs(math.hypot(*center[:2]) - 1) <= 0.01
    for (a, b), count in incidence.items():
        if count == 1:
            z = mesh["points"][a][2]
            assert mesh["points"][b][2] == z and z in (0, 2)
        else:
            assert count == 2


def test_reject_coincident_endpoints_with_wrong_period_count():
    rows = cylinder()
    rows["topology"]["edges"][0]["interval"][1] *= 2
    with pytest.raises(ValueError, match="pcurve disagree"):
        ExactGeometry(json.dumps(rows))


def test_spline_quarter_circle_and_generator():
    axis = dict(degree=2, knots=[-1, 1], multiplicities=[3, 3], periodic=False)
    spline = dict(type="spline", axis=axis,
                  controls=[[1, 0, 0, 1], [1, 1, 0, math.sqrt(0.5)], [0, 1, 0, 1]])
    extrusion = dict(type="extrusion", generator=0, vector=[0, 0, 2])
    pool = dict(curves=[spline], surfaces=[extrusion])
    geometry = ExactGeometry(json.dumps(pool))
    assert geometry.surface_point(0, 0, 3) == pytest.approx([math.sqrt(0.5)] * 2 + [6])
    with pytest.raises(ValueError, match="outside"):
        geometry.curve_point(0, -2)
    spline["controls"][0][-1] = 0
    with pytest.raises(ValueError, match="weight"):
        ExactGeometry(json.dumps(pool))


def test_materialized_patch_is_one_history_step():
    from built_core.history import History
    from built_core.protocol import commands
    from built_core.scene import Scene

    scene = Scene()
    history = History(scene)
    result = commands.run(history, "materialize_exact_geometry", geometry=cylinder())
    assert result["ok"]
    ids = list(scene.mesh._store.ids("face"))
    assert len(ids) > 10
    history.undo()
    assert len(scene.mesh.faces) == 0
    history.redo()
    assert list(scene.mesh._store.ids("face")) == ids
