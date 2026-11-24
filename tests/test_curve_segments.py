# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The segment count of a circle, arc or polygon, changed after it is
drawn (Entity Info). The «24s» of the tools only reached the shape just
drawn; any other had to be erased and drawn again."""
from __future__ import annotations

import math
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest  # noqa: E402
from tests.native_tree import action, inspector
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views.wgpu_ui import _edit
from PySide6.QtCore import QPointF, Qt  # noqa: E402
from ingetrazo_core import Vec3 as V
from PySide6.QtWidgets import QApplication  # noqa: E402

from core.curve_segments import ResegmentCurveCommand, read_curve  # noqa: E402
from tools.base import ToolContext  # noqa: E402
from tools.circle import CircleTool, PolygonTool  # noqa: E402
from tools.rust_tools import RectangleTool  # noqa: E402

_app = QApplication.instance() or QApplication([])


@pytest.fixture
def viewport():
    from tests.controller_surface import ControllerSurface as Viewport
    vp = Viewport(None)
    vp.resize(1000, 700)
    vp.camera.set_aspect(1000, 700)
    vp.flash_status = lambda *a, **k: None
    return vp


def _draw(vp, tool, *points):
    tool.on_activate(vp)
    for p in points:
        tool.on_click(ToolContext(viewport=vp, world=p, screen=QPointF(0, 0),
                                  modifiers=Qt.NoModifier, snap=None))


def _curve_edge(mesh):
    return next(e for e in mesh.edges if e.curve is not None)


def _loops(mesh):
    return sorted((len(f.vertices), sorted(len(h) for h in f.holes))
                  for f in mesh.faces)


def _arc_with_chord(mesh, n=4, r=2.0):
    """A half disc: an arc of ``n`` segments closed by its diameter."""
    pts = [V(r * math.cos(math.pi * k / n), r * math.sin(math.pi * k / n), 0)
           for k in range(n + 1)]
    face = mesh.add_face(pts)
    mesh.tag_curve(pts, closed=False)
    return face, pts


@pytest.mark.parametrize("cls,before,after", [(CircleTool, 24, 8),
                                               (CircleTool, 24, 48),
                                               (PolygonTool, 6, 8)])
def test_a_shape_takes_a_new_count_in_place(viewport, cls, before, after):
    _draw(viewport, cls(), V(1, 1, 0), V(3, 1, 0))
    mesh = viewport.scene.mesh
    shape = read_curve(mesh, _curve_edge(mesh))
    assert shape.closed and shape.flat and shape.segments == before
    steps = len(viewport.history.undo_stack)
    viewport.history.execute(ResegmentCurveCommand(_curve_edge(mesh), after))
    assert len(viewport.history.undo_stack) == steps + 1
    assert _loops(mesh) == [(after, [])]
    radii = {round((v.position - V(1, 1, 0)).length(), 6)
             for v in mesh.vertices}
    assert radii == {2.0}
    assert len({e.curve for e in mesh.edges}) == 1     # still one curve
    assert len(viewport.scene.selection) == after      # and it stays picked
    viewport.history.undo()
    assert _loops(mesh) == [(before, [])]
    viewport.history.redo()
    assert _loops(mesh) == [(after, [])]


def test_the_hole_it_cut_follows_and_paint_stays(viewport):
    _draw(viewport, RectangleTool(), V(0, 0, 0), V(10, 10, 0))
    _draw(viewport, CircleTool(), V(5, 5, 0), V(7, 5, 0))
    mesh = viewport.scene.mesh
    for f in mesh.faces:
        f.attrs["color"] = (0.1, 0.2, 0.3) if len(f.vertices) == 4 \
            else (0.9, 0.8, 0.7)
    viewport.history.execute(ResegmentCurveCommand(_curve_edge(mesh), 12))
    assert _loops(mesh) == [(4, [12]), (12, [])]
    colors = {len(f.vertices): f.attrs.get("color") for f in mesh.faces}
    assert colors == {4: (0.1, 0.2, 0.3), 12: (0.9, 0.8, 0.7)}
    viewport.history.undo()
    assert _loops(mesh) == [(4, [24]), (24, [])]


def test_an_arc_keeps_its_ends_and_its_face(viewport):
    mesh = viewport.scene.mesh
    face, pts = _arc_with_chord(mesh)
    face.attrs["color"] = (0.5, 0.5, 0.5)
    edge = _curve_edge(mesh)
    shape = read_curve(mesh, edge)
    assert not shape.closed and shape.segments == 4
    assert shape.sweep == pytest.approx(math.pi)
    viewport.history.execute(ResegmentCurveCommand(edge, 10))
    assert _loops(mesh) == [(11, [])]
    assert mesh.vertex_at(pts[0]) is not None
    assert mesh.vertex_at(pts[-1]) is not None
    assert sum(1 for e in mesh.edges if e.curve is not None) == 10
    assert mesh.faces[0].attrs["color"] == (0.5, 0.5, 0.5)
    assert mesh.faces[0].area() == pytest.approx(
        0.5 * 10 * 4.0 * math.sin(math.pi / 10))


def test_an_uneven_stroke_is_not_a_curve_to_rebuild(viewport):
    mesh = viewport.scene.mesh
    pts = [V(0, 0, 0), V(1, 0.2, 0), V(1.5, 1, 0), V(3, 1.1, 0)]
    for a, b in zip(pts, pts[1:]):
        mesh.add_edge(a, b)
    mesh.tag_curve(pts, closed=False)
    assert read_curve(mesh, _curve_edge(mesh)) is None


def test_a_pushed_cylinder_is_left_alone(viewport):
    _draw(viewport, CircleTool(), V(0, 0, 0), V(2, 0, 0))
    mesh = viewport.scene.mesh
    ring = [v.position for v in mesh.faces[0].loop]
    for a, b in zip(ring, ring[1:] + ring[:1]):
        mesh.add_face([a, b, b + V(0, 0, 3), a + V(0, 0, 3)])
    edge = _curve_edge(mesh)
    shape = read_curve(mesh, edge)
    assert shape is not None and not shape.flat
    faces = len(mesh.faces)
    steps = len(viewport.history.undo_stack)
    viewport.history.execute(ResegmentCurveCommand(edge, 8))
    assert len(viewport.history.undo_stack) == steps     # rejected
    assert len(mesh.faces) == faces


# ---- The field in Entity Info ---------------------------------------------
def test_the_field_shows_for_one_curve_and_rebuilds_it(native_host):
    vp = native_host.viewport
    _draw(vp, PolygonTool(), V(0, 0, 0), V(2, 0, 0))
    mesh = vp.scene.mesh
    edges = [e for e in mesh.edges if e.curve == _curve_edge(mesh).curve]
    rows = inspector(native_host, edges)
    assert rows["Polygon"]["Sides"] == _edit("segments", "6")
    assert "Item" not in rows
    action(native_host, "edit", "segments", "9")
    assert _loops(mesh) == [(9, [])]
    assert inspector(native_host, list(mesh.edges))["Polygon"]["Sides"] == _edit("segments", "9")
    vp.history.undo()
    assert inspector(native_host, list(mesh.edges))["Polygon"]["Sides"] == _edit("segments", "6")


def test_the_field_hides_for_a_plain_edge_or_a_face(native_host):
    vp = native_host.viewport
    _draw(vp, RectangleTool(), V(0, 0, 0), V(4, 4, 0))
    for entity in (vp.scene.mesh.edges[0], vp.scene.mesh.faces[0]):
        rows = inspector(native_host, [entity])
        assert rows.keys().isdisjoint({"Arc", "Polygon", "Circle"})


def test_an_arc_reads_segments(native_host):
    mesh = native_host.viewport.scene.mesh
    _arc_with_chord(mesh, n=5)
    edges = [e for e in mesh.edges if e.curve == _curve_edge(mesh).curve]
    assert inspector(native_host, edges)["Arc"]["Segments"] == _edit("segments", "5")
    action(native_host, "edit", "segments", "1")
    assert _loops(mesh) == [(6, [])]
    action(native_host, "edit", "segments", "2")
    assert _loops(mesh) == [(3, [])]


def test_a_curve_is_described_by_its_shape(native_host):
    vp = native_host.viewport
    _draw(vp, CircleTool(), V(0, 0, 0), V(2, 0, 0))
    mesh = vp.scene.mesh
    rows = inspector(native_host, list(mesh.edges))
    assert {"Radius", "Diameter", "Perimeter"} <= rows["Circle"].keys()
    vp.history.execute(ResegmentCurveCommand(_curve_edge(mesh), 6))
    rows = inspector(native_host, list(mesh.edges))
    assert "Side" in rows["Polygon"]


def test_a_painted_face_shows_a_swatch_and_its_code(native_host):
    from tests.native_tree import inspector
    mesh = native_host.viewport.scene.mesh
    face = mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
    face.attrs["color"] = (.85, .45, .2)
    assert inspector(native_host, [face])["Geometry"]["Material"] == "\x1dswatch\x1d#D97333"


@pytest.mark.parametrize("offset,r", [(1000.0, 0.05), (10000.0, 0.5),
                                      (100000.0, 2.0)])
def test_a_small_circle_far_from_the_origin_is_still_read(offset, r):
    """Positions are single precision: the centre is worked out from the
    first vertex, and the rounding of far coordinates is allowed for."""
    from core.mesh import Mesh
    mesh = Mesh()
    c = V(offset, offset, 0)
    pts = [c + V(math.cos(2 * math.pi * k / 24),
                 math.sin(2 * math.pi * k / 24), 0) * r for k in range(24)]
    mesh.add_face(pts)
    mesh.tag_curve(pts, closed=True)
    shape = read_curve(mesh, _curve_edge(mesh))
    assert shape is not None and shape.radius == pytest.approx(r, rel=1e-2)
