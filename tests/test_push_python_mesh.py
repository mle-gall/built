# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A push always runs on a store (docs/rust-tools.md §24): a Python mesh —
materialised after another operation's refusal, or made with
``INGETRAZO_STORE=0`` — becomes store-backed before the push, outside any
History step. The steps before keep working: undoing them turns the mesh
back into their Python objects, redoing comes back to the store."""
from __future__ import annotations

import os
from unittest.mock import patch

import pytest
from ingetrazo_core import Vec3 as V

import core.store_mesh as S
from core import mesh as M
from core.history import AddFaceCommand, History
from core.protocol.commands import push_pull
from core.protocol.keys import to_json_key
from core.scene import Scene
from tests.test_pushpull_cases import _box, _face

pytestmark = pytest.mark.skipif(not S.enabled(), reason="store off")


def _shape(mesh):
    """Each face's corners (rounded), outline and holes, as a sorted list:
    the geometry with no keys or objects."""
    def ring(lp):
        return tuple(sorted(tuple(round(c, 6) for c in v.position.toTuple()) for v in lp))
    return sorted((ring(f.loop), tuple(sorted(ring(h) for h in f.hole_loops)),
                   bool(f.interior)) for f in mesh.faces)


def _scene(python: bool):
    if python:
        with patch.dict(os.environ, INGETRAZO_STORE="0"):
            scene = Scene()
            assert type(scene.mesh) is M.Mesh
    else:
        scene = Scene()
    return scene, History(scene)


@pytest.mark.parametrize("kind", ["materialised", "python"])
def test_a_python_mesh_is_pushed_through_a_store_and_undo_holds(kind):
    scene, hist = _scene(kind == "python")
    _box(hist, 0, 0, 0, 2, 2, 2)
    mesh = scene.mesh
    if kind == "materialised":
        S._materialise(mesh)
        assert type(mesh) is S.MaterialisedMesh
    # A step made on the Python objects: its undo holds them.
    hist.execute(AddFaceCommand([V(0.5, 0.5, 2), V(1.5, 0.5, 2), V(1.5, 1.5, 2), V(0.5, 1.5, 2)]))
    s1 = _shape(mesh)
    top = _face(mesh, (1, 1, 2), (0, 0, 1))
    scene.selection.add(top)
    answer = push_pull(hist, to_json_key(top.key), 0.5)
    assert answer["ok"]
    assert type(mesh) is S.StoreMesh
    s2 = _shape(mesh)
    assert s2 != s1
    # The selection was carried to the new elements (no Python object left).
    assert all(type(x) in (S.FaceView, S.EdgeView, S.VertexView) for x in scene.selection)
    assert hist.undo() and _shape(mesh) == s1
    assert hist.undo()                               # the Python step
    s0 = _shape(mesh)
    assert s0 != s1
    assert hist.redo() and _shape(mesh) == s1
    assert hist.redo() and _shape(mesh) == s2
    assert type(mesh) is S.StoreMesh


def test_the_same_push_on_a_python_mesh_and_a_store():
    shapes = []
    for python in (True, False):
        scene, hist = _scene(python)
        _box(hist, 0, 0, 0, 2, 2, 2)
        top = _face(scene.mesh, (1, 1, 2), (0, 0, 1))
        assert push_pull(hist, to_json_key(top.key), -0.75)["ok"]
        shapes.append(_shape(scene.mesh))
    assert shapes[0] == shapes[1]


def test_paint_and_curves_come_along():
    scene, hist = _scene(True)
    _box(hist, 0, 0, 0, 2, 2, 2)
    mesh = scene.mesh
    side = _face(mesh, (1, 0, 1), (0, 1, 0))
    side.attrs["color"] = [0.2, 0.7, 0.4]
    paint = side.attrs
    e = mesh.edges[0]
    e.soft = True
    e.curve = 77
    views = S.to_store(mesh)
    assert type(mesh) is S.StoreMesh
    assert views[id(side)].attrs is paint
    ev = views[id(e)]
    assert ev.soft and ev.curve == 77


def test_annotations_on_the_python_mesh_reattach():
    from core.annotation_anchor import Attachment
    from core.textlabel import TextLabel
    scene, hist = _scene(True)
    _box(hist, 0, 0, 0, 2, 2, 2)
    mesh = scene.mesh
    # On the bottom: the push of the top leaves it where it is.
    bottom = _face(mesh, (1, 1, 0), (0, 0, 1))
    att = Attachment.capture(scene, bottom, None, V(0.5, 1.5, 0))
    assert att is not None
    scene.text_labels.append(TextLabel(V(0.5, 1.5, 0), V(0, 0, -1), "bottom", attachment=att))
    where = att.position().toTuple()
    top = _face(mesh, (1, 1, 2), (0, 0, 1))
    assert push_pull(hist, to_json_key(top.key), 0.5)["ok"]
    assert type(mesh) is S.StoreMesh
    label = scene.text_labels[0]
    assert label.attachment is not None
    got = label.attachment.position()
    assert got is not None and got.toTuple() == pytest.approx(where)
