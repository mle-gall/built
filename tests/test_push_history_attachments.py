# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Selection, history and attachment ownership across Push/Pull."""
from __future__ import annotations


import pytest
from ingetrazo_core import Vec3 as V

import ingetrazo_core.store_mesh as S
from core.history import AddFaceCommand, History
from core.protocol.commands import push_pull
from core.protocol.keys import to_json_key
from core.scene import Scene
from tests.test_pushpull_cases import _box, _face



def _shape(mesh):
    """Each face's corners (rounded), outline and holes, as a sorted list:
    the geometry with no keys or objects."""
    def ring(lp):
        return tuple(sorted(tuple(round(c, 6) for c in v.position.toTuple()) for v in lp))
    return sorted((ring(f.loop), tuple(sorted(ring(h) for h in f.hole_loops)),
                   bool(f.interior)) for f in mesh.faces)


def test_push_retains_selection_and_history():
    scene = Scene()
    hist = History(scene)
    _box(hist, 0, 0, 0, 2, 2, 2)
    mesh = scene.mesh
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
    assert hist.undo()
    s0 = _shape(mesh)
    assert s0 != s1
    assert hist.redo() and _shape(mesh) == s1
    assert hist.redo() and _shape(mesh) == s2
    assert type(mesh) is S.StoreMesh


def test_annotations_survive_a_push():
    from ingetrazo_core.annotation_anchor import Attachment
    from core.textlabel import TextLabel
    scene = Scene()
    hist = History(scene)
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
