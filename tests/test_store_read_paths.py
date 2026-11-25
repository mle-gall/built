# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Read paths behave the same on a store-backed mesh as on a Python one.

Iterating a store-backed mesh (``core/store_mesh.py``) hands out views made
on demand: two views of one face are equal but never the same object. Code
that compared elements with ``is`` worked on Python meshes (one object per
element) and silently failed on the store, where nothing materialises
before a read: the hover over a selected face, an edge looked up between
two loop vertices, a face skipped as "itself" among its neighbours. Each
scenario here runs on the same document opened with the store off and on
and must answer the same; on the store it must also stay store-backed (a
read that materialised would hide the very bug it checks)."""
from __future__ import annotations

import random                                                     # noqa: E402
from types import SimpleNamespace                                 # noqa: E402

import pytest                                                     # noqa: E402
from core.input_codes import Input
from ingetrazo_core import ScreenPoint, Vec3 as QVector3D

import core._native as _native                                    # noqa: E402
from core import store_mesh as S                                  # noqa: E402
from core.history import History                                  # noqa: E402
from core.mesh import Edge, Face                                  # noqa: E402
from core.scene import Scene                                      # noqa: E402
from formats import igz                                           # noqa: E402
from tests.test_fuzz_engine import V, _draw_rect, _push           # noqa: E402
from tools.base import ToolContext                                # noqa: E402


pytestmark = pytest.mark.skipif(
    _native.available() is None
    or not hasattr(_native.available(), "MeshStore"),
    reason="native module without MeshStore")


def _box_and_disc() -> Scene:
    """A 4 x 3 x 2 box, plus a 16-sided disc beside it tagged as one drawn
    circle (its edges share a curve id)."""
    scene = Scene()
    hist = History(scene)
    _draw_rect(scene, hist, [V(0, 0), V(4, 0), V(4, 3), V(0, 3)], [])
    assert _push(scene, hist, scene.mesh.faces[0], 2.0)
    import math
    ring = [QVector3D(10 + 2 * math.cos(2 * math.pi * i / 16),
                      2 * math.sin(2 * math.pi * i / 16), 0)
            for i in range(16)]
    scene.mesh.add_face(ring)
    scene.mesh.tag_curve(ring, closed=True)
    return scene


@pytest.fixture
def doc(monkeypatch, tmp_path):
    """The same saved document, reopened with the store off or on."""
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)
    monkeypatch.setattr(_native, "wanted", lambda: True)
    path = tmp_path / "doc.igz"
    igz.save_scene(_box_and_disc(), path)
    scene = Scene()
    igz.load_into(scene, path)
    len(scene.mesh.faces)                               # first touch
    assert type(scene.mesh) is S.StoreMesh
    yield SimpleNamespace(scene=scene)
    assert type(scene.mesh) is S.StoreMesh


def _top(scene):
    return next(f for f in scene.mesh.faces
                if f.normal().z() > 0.9 and f.centroid().z() > 1.9)


def _corner_edge(scene):
    """The top face's edge from (0, 0, 2) to (4, 0, 2)."""
    return next(e for e in scene.mesh.edges
                if {e.a.toTuple(), e.b.toTuple()}
                == {(0.0, 0.0, 2.0), (4.0, 0.0, 2.0)})


class _Viewport:
    """Picks answer chosen entities, fetched afresh on every call as the
    real picker does (so on the store each pick is a new view)."""

    def __init__(self, scene, face=None, edge=None):
        self.scene = scene
        self.history = History(scene)
        self._face, self._edge = face, edge
        self._hover_entity = None
        self._hover_placement = None
        self.updates = 0

    def _fresh(self, ent):
        if ent is None:
            return None
        seq = (self.scene.mesh.faces if isinstance(ent, Face)
               else self.scene.mesh.edges)
        return seq[seq.index(ent)]

    def pick_face(self, _x, _y):
        return self._fresh(self._face)

    def pick_face_any(self, _x, _y):
        return self._fresh(self._face), None

    def pick_edge(self, _x, _y):
        return self._fresh(self._edge)

    def pick_group(self, _x, _y):
        return None

    def pick_dimension(self, _x, _y):
        return None

    def pick_geopath(self, _x, _y):
        return None

    def set_hover(self, entity, placement=None):
        from tests.controller_surface import ControllerSurface as Viewport
        Viewport.set_hover(self, entity, placement)

    def update(self):
        self.updates += 1

    def flash_status(self, *_a, **_k):
        pass


def _ctx(vp, modifiers=Input.NoModifier):
    return ToolContext(viewport=vp, world=QVector3D(), screen=ScreenPoint(0, 0),
                       modifiers=modifiers, snap=None)


def test_views_of_one_face_are_never_identical(doc):
    """The premise: on the store, two reads of a face are equal, not the
    same object; on a Python mesh they are the same object."""
    a, b = _top(doc.scene), _top(doc.scene)
    assert a == b
    assert a is not b


def test_hover_over_a_selected_face(doc):
    """The selection holds one view, the hover another: the hover still
    reads as selected (no blue dots over the orange), and a second
    set_hover of the same face is no change (no repaint per mouse move)."""
    scene = doc.scene
    scene.select([_top(scene)])
    vp = _Viewport(scene, face=_top(scene))
    vp.set_hover(vp.pick_face(0, 0))
    vp.set_hover(vp.pick_face(0, 0))
    assert vp.updates == 1
    hovered = vp._hover_entity
    assert isinstance(hovered, Face) and hovered in scene.selection


def test_snap_to_an_endpoint_of_a_selected_edge(doc):
    from core.snap import compute_snap
    scene = doc.scene
    edge = _corner_edge(scene)
    scene.select([edge])
    # Oblique, so corners stacked in z land on different pixels.
    to_px = (lambda p: (p.x() * 100.0 + p.z() * 37.0,
                        -p.y() * 100.0 - p.z() * 61.0))
    near = QVector3D(4.03, 0.02, 2.0)
    hit = compute_snap(near, to_px(near), scene, to_px, 12.0,
                       reference_edge=_corner_edge(scene))
    assert hit is not None
    point, kind = hit.point, hit.kind
    assert kind == "endpoint"
    assert point.toTuple() == pytest.approx((4.0, 0.0, 2.0))


def test_paint_hover_marks_the_face_under_the_cursor(doc):
    from tools.paint import PaintTool
    scene = doc.scene
    scene.select([_top(scene)])
    vp = _Viewport(scene, face=_top(scene))
    PaintTool().on_hover(_ctx(vp))
    assert vp._hover_entity == _top(scene)
    assert vp._hover_entity in scene.selection


@pytest.mark.parametrize("modifiers,stays", [
    (Input.ShiftModifier, False),                          # toggle: out
    (Input.ControlModifier, True),                         # add: still in
    (Input.ShiftModifier | Input.ControlModifier, False),     # remove: out
])
def test_select_click_on_a_face_already_selected(doc, modifiers, stays):
    from tools.rust_tools import SelectTool
    scene = doc.scene
    side = next(f for f in scene.mesh.faces if f.normal().y() < -0.9)
    scene.select([_top(scene), side])
    vp = _Viewport(scene, face=_top(scene))
    SelectTool().on_click(_ctx(vp, modifiers))
    assert (_top(scene) in scene.selection) == stays
    assert side in scene.selection
    assert len(scene.selection) == (2 if stays else 1)


def test_pushpull_hover_and_lock_on_a_face(doc):
    """Hover sets the face to push; the classification the drag starts
    with (``_classify_base``) skips the base face among its neighbours by
    equality: on the store a face never "is" itself."""
    from tools.pushpull import PushPullTool
    scene = doc.scene
    vp = _Viewport(scene, face=_top(scene))
    tool = PushPullTool()
    tool.on_hover(_ctx(vp))
    assert tool.hovered_face == _top(scene)
    assert vp._hover_entity == _top(scene)
    tool.base_face = vp.pick_face(0, 0)
    target = tool._target_scene(scene)
    attached, prism_cap = tool._classify_base(target)
    assert (bool(attached), bool(prism_cap)) == (True, True)


def test_edges_found_between_loop_vertices(doc):
    """``Select ▸ Bounding Edges`` and the edge-between lookups walk a
    vertex's edges for the one whose other end is the next loop vertex."""
    from core.select_ops import bounding_edges
    from core.orient_faces import _edge_between
    scene = doc.scene
    top = _top(scene)
    edges = bounding_edges([top])
    assert len(edges) == 4 and all(isinstance(e, Edge) for e in edges)
    lp = top.loop
    assert _edge_between(lp[0], lp[1]) is not None


def test_a_drawn_circle_reads_as_one_closed_curve(doc):
    from core.curve_segments import read_curve
    from core.snap import _boundary_runs
    scene = doc.scene
    disc = next(f for f in scene.mesh.faces if len(f.loop) == 16)
    edge = scene.mesh.find_edge(disc.loop[0], disc.loop[1])
    shape = read_curve(scene.mesh, edge)
    assert shape is not None and shape.closed
    runs = _boundary_runs(disc.loop)
    assert len(runs) == 1 and len(runs[0][0]) >= 16
