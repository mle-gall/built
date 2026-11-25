# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Fillet 3D bevels: a Ctrl tap walks how the size is read (radius →
distance → width → depth), «0.3p» in the value box shapes the profile,
and one segment is a flat chamfer — one undo step, as before. It bevels
corners (vertices) too. The tool strip and inspector follow edits made with the keyboard."""
from __future__ import annotations

from ingetrazo_render import KeyEvent
import pytest
from ingetrazo_core import ScreenPoint as QPointF, Input as Qt
from ingetrazo_core import Vec3 as QVector3D

from core.orient import is_closed, signed_volume
from tools.base import ToolContext
from tools.fillet import FilletTool
from tests.test_fillet import box, edge_at


@pytest.fixture(scope="module")
def viewport():
    from tests.controller_surface import ControllerSurface as Viewport
    vp = Viewport(None)
    vp.resize(1000, 700)
    vp.camera.set_aspect(1000, 700)
    vp.flash_status = lambda *a, **k: None
    return vp


@pytest.fixture(autouse=True)
def plain_fillet():
    """The tool's settings are class-wide: every test starts from the
    plain fillet and leaves it so."""
    saved = (FilletTool.radius, FilletTool.segments, FilletTool.profile,
             FilletTool.measure, FilletTool.corners, FilletTool.affect,
             FilletTool.shape)
    FilletTool.radius, FilletTool.segments = 0.1, 8
    FilletTool.profile, FilletTool.measure = 0.5, "radius"
    FilletTool.corners, FilletTool.affect = "round", "edges"
    FilletTool.shape = None
    yield
    (FilletTool.radius, FilletTool.segments, FilletTool.profile,
     FilletTool.measure, FilletTool.corners, FilletTool.affect,
     FilletTool.shape) = saved


def _box_scene(vp):
    vp.scene.mesh.clear()
    vp.history.undo_stack.clear()
    vp.history.redo_stack.clear()
    for f in box().faces:
        vp.scene.mesh.add_face(f.vertices)
    vp.scene.version += 1
    return vp.scene.mesh


def _ctx(vp, world):
    return ToolContext(viewport=vp, world=QVector3D(*world), screen=QPointF(0, 0),
                       modifiers=Qt.NoModifier, snap=None)


def _tap_ctrl(vp, tool, alone=True):
    assert tool.on_key(vp, Qt.Key_Control, Qt.ControlModifier)
    vp._ctrl_alone = alone
    return tool.on_key_release(vp, Qt.Key_Control)


def _type(vp, text):
    for ch in text:
        ev = KeyEvent(True, 0, 0, ch)
        assert vp._handle_value_key(ev), ch
    ev = KeyEvent(True, Qt.Key_Return)
    return vp._handle_value_key(ev)


def test_a_ctrl_tap_walks_the_measures(viewport):
    _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    assert tool.vcb_caption() == "Radius"
    seen = []
    for _ in range(4):
        assert _tap_ctrl(viewport, tool)
        seen.append(FilletTool.measure)
    assert seen == ["offset", "width", "depth", "radius"]
    FilletTool.measure = "width"
    assert tool.vcb_caption() == "Width"
    assert "Width" in tool.status_clause()


def test_ctrl_inside_a_shortcut_leaves_the_measure(viewport):
    _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    assert not _tap_ctrl(viewport, tool, alone=False)      # Ctrl+Z, say
    assert FilletTool.measure == "radius"


def test_the_preview_follows_the_measure(viewport):
    mesh = _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    viewport._hover_edge = edge_at(mesh, (0, 0, 1), (2, 0, 1))
    tool.on_hover(_ctx(viewport, (1, 0, 1)))
    FilletTool.segments = 1
    tool._replan()
    # Radius 0.1 at 90°: the chamfer runs 0.1 back on each face …
    ys = sorted(q.y() for q in tool.preview_faces()[0].vertices)
    assert ys[-1] == pytest.approx(0.1, abs=1e-6)
    # … read as width, 0.1 is the chord, so 0.1/√2 on each face.
    FilletTool.measure = "width"
    tool._replan()
    ys = sorted(q.y() for q in tool.preview_faces()[0].vertices)
    assert ys[-1] == pytest.approx(0.1 / 2 ** 0.5, abs=1e-6)
    assert tool.value_label()[0].startswith("Width 0.100")


def test_a_typed_profile_shapes_the_bevel(viewport):
    mesh = _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    viewport.scene.select([edge_at(mesh, (0, 0, 1), (2, 0, 1))])
    tool.on_activate(viewport)
    _type(viewport, "4s")
    assert FilletTool.segments == 4
    _type(viewport, "0,3p")                   # the comma is a decimal mark
    assert FilletTool.profile == pytest.approx(0.3)
    assert "0.30" in tool.value_label()[0]
    depth = len(viewport.history.undo_stack)
    _type(viewport, "0.2")
    assert len(viewport.history.undo_stack) == depth + 1
    assert len(mesh.faces) == 6 + 4 and is_closed(mesh)
    viewport.history.undo()
    assert len(mesh.faces) == 6 and abs(signed_volume(mesh)) == pytest.approx(2.0)


def test_a_profile_outside_zero_one_only_speaks(viewport):
    _box_scene(viewport)
    said = []
    viewport.flash_status = lambda text, *a, **k: said.append(text)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    assert not tool.on_profile_value(viewport, 1.5)
    assert not tool.on_profile_value(viewport, 0.0)
    assert FilletTool.profile == 0.5
    assert any("profile" in s for s in said)
    viewport.flash_status = lambda *a, **k: None


def test_a_typed_chamfer_by_depth(viewport):
    mesh = _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    viewport.scene.select([edge_at(mesh, (0, 0, 1), (2, 0, 1))])
    tool.on_activate(viewport)
    FilletTool.measure = "depth"
    assert tool.on_segments_value(viewport, 1)
    assert tool.on_value(viewport, 0.2 / 2 ** 0.5)       # t = 0.2 on each face
    assert len(mesh.faces) == 7 and is_closed(mesh)
    assert abs(signed_volume(mesh)) == pytest.approx(2.0 - 0.02 * 2.0, abs=1e-6)


def test_the_value_box_reads_a_profile_suffix():
    from tests.controller_surface import ControllerSurface as Viewport
    assert Viewport._parse_value_buffer("0.3p") == ("profile", 0.3)
    assert Viewport._parse_value_buffer("0,75p") == ("profile", 0.75)
    assert Viewport._parse_value_buffer("p") is None


def test_p_stays_push_pull_for_other_tools(viewport):
    from tools.rust_tools import LineTool
    viewport.set_active_tool(LineTool())
    viewport._set_value_buffer("")
    assert viewport._handle_value_key(KeyEvent(True, 0, 0, '2'))
    assert not viewport._handle_value_key(KeyEvent(True, 0, 0, 'p'))
    viewport._set_value_buffer("")


# ---- The panel -----------------------------------------------------------------

def _edit(viewport, key, value=None):
    from ingetrazo_render import bevel_edit
    bevel_edit(viewport, viewport.active_tool, key, value)


def test_panel_edits_reach_the_tool_and_the_preview(viewport):
    mesh = _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    viewport.scene.select(list(mesh.edges))
    tool.on_activate(viewport)
    _edit(viewport, "segments", 3)
    assert FilletTool.segments == 3
    _edit(viewport, "profile", 0.7)
    assert FilletTool.profile == pytest.approx(0.7)
    _edit(viewport, "measure", "width")
    assert FilletTool.measure == "width"
    _edit(viewport, "corners", "cut")
    assert FilletTool.corners == "cut"
    _edit(viewport, "amount", "0.15")
    assert FilletTool.radius == pytest.approx(0.15)
    assert len(mesh.faces) == 6                       # nothing applied yet
    # Live preview: 12 strips of 3 plus 8 corners capped by 3 flat faces.
    assert len(tool.preview_faces()) == 12 * 3 + 8 * 3
    depth = len(viewport.history.undo_stack)
    _edit(viewport, "apply")
    assert len(viewport.history.undo_stack) == depth + 1
    assert len(mesh.faces) == 6 + 12 * 3 + 8 * 3 and is_closed(mesh)
    viewport.history.undo()
    assert len(mesh.faces) == 6


def test_the_panel_follows_the_keys(viewport):
    _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    _tap_ctrl(viewport, tool)
    assert tool.measure == "offset"
    _type(viewport, "5s")
    assert tool.segments == 5
    _type(viewport, "0,4p")
    assert tool.profile == pytest.approx(0.4)
    _type(viewport, "0.3")                            # no edge: only remembered
    assert tool.radius == pytest.approx(0.3)


def test_a_bad_amount_in_the_panel_is_ignored(viewport):
    _box_scene(viewport)
    viewport.set_active_tool(FilletTool())
    for text in ("abc", "-1", "0"):
        _edit(viewport, "amount", text)
        assert FilletTool.radius == pytest.approx(0.1)
    assert viewport.active_tool.radius == pytest.approx(0.1)


def test_apply_with_nothing_picked_only_speaks(viewport):
    _box_scene(viewport)
    said = []
    viewport.flash_status = lambda text, *a, **k: said.append(text)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    viewport._hover_edge = None
    tool.hover_edge = None
    depth = len(viewport.history.undo_stack)
    _edit(viewport, "apply")
    assert len(viewport.history.undo_stack) == depth
    assert any("Click an edge" in s for s in said)
    viewport.flash_status = lambda *a, **k: None


# ---- Corners (vertices) ---------------------------------------------------------

def test_hovering_near_an_end_picks_that_corner(viewport):
    mesh = _box_scene(viewport)
    FilletTool.affect = "vertices"
    tool = FilletTool()
    viewport.set_active_tool(tool)
    viewport._hover_edge = edge_at(mesh, (0, 0, 1), (2, 0, 1))
    tool.on_hover(_ctx(viewport, (1.8, 0, 1)))
    assert tool.hover_vertex == mesh.vertex_at(QVector3D(2, 0, 1))
    assert tool.vcb_caption() == "Distance"
    assert "Vertices" in tool.status_clause()
    assert len(tool.preview_faces()) == 8 * 8          # the round cap
    tool.on_click(_ctx(viewport, (1.8, 0, 1)))
    assert tool.sizing and tool.edges == [tool.hover_vertex]
    tool.on_hover(_ctx(viewport, (1.75, 0, 1)))       # 0.25 from the corner
    assert FilletTool.radius == pytest.approx(0.25)
    assert tool.value_label()[0].startswith("Distance 0.250")
    depth = len(viewport.history.undo_stack)
    tool.on_click(_ctx(viewport, (1.75, 0, 1)))
    assert len(viewport.history.undo_stack) == depth + 1
    assert len(mesh.faces) == 6 + 64 and is_closed(mesh)
    viewport.history.undo()
    assert len(mesh.faces) == 6 and len(mesh.edges) == 12


def test_a_selected_box_bevels_its_eight_corners(viewport):
    mesh = _box_scene(viewport)
    FilletTool.affect = "vertices"
    viewport.scene.select(list(mesh.faces) + list(mesh.edges))
    tool = FilletTool()
    viewport.set_active_tool(tool)
    assert len(tool.edges) == 8
    assert tool.on_segments_value(viewport, 1)
    assert tool.on_value(viewport, 0.1)
    assert len(mesh.faces) == 6 + 8 and is_closed(mesh)
    assert abs(signed_volume(mesh)) == pytest.approx(2.0 - 8 * 0.1 ** 3 / 6, abs=1e-6)


def test_ctrl_leaves_the_measure_while_bevelling_corners(viewport):
    _box_scene(viewport)
    FilletTool.affect = "vertices"
    tool = FilletTool()
    viewport.set_active_tool(tool)
    assert _tap_ctrl(viewport, tool)
    assert FilletTool.measure == "radius"


def test_the_panel_switches_to_corners_and_carries_the_pick(viewport):
    mesh = _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    viewport.scene.select([edge_at(mesh, (0, 0, 1), (2, 0, 1))])
    tool.on_activate(viewport)
    _edit(viewport, "affect", "vertices")
    assert FilletTool.affect == "vertices"
    assert {tuple((v.position.x(), v.position.y(), v.position.z()))
            for v in tool.edges} == {(0.0, 0.0, 1.0), (2.0, 0.0, 1.0)}
    _edit(viewport, "affect", "edges")
    assert FilletTool.affect == "edges" and not tool.edges


def test_a_refused_corner_only_speaks(viewport):
    mesh = _box_scene(viewport)
    said = []
    viewport.flash_status = lambda text, *a, **k: said.append(text)
    FilletTool.affect = "vertices"
    tool = FilletTool()
    viewport.set_active_tool(tool)
    viewport._hover_edge = edge_at(mesh, (0, 0, 1), (2, 0, 1))
    tool.on_hover(_ctx(viewport, (0.1, 0, 1)))
    tool.on_click(_ctx(viewport, (0.1, 0, 1)))
    assert tool.on_value(viewport, 1.5)               # longer than the edges
    assert len(mesh.faces) == 6
    assert any("too large" in s for s in said)
    viewport.flash_status = lambda *a, **k: None


def test_the_profile_shape_picks_round_flat_or_custom(viewport):
    mesh = _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    assert tool.profile == 0.5 and tool.shape is None
    _edit(viewport, "shape", "chamfer")
    assert FilletTool.profile == pytest.approx(0.25)
    _edit(viewport, "shape", "profile value")
    assert tool.profile == pytest.approx(0.25) and tool.shape is None
    _edit(viewport, "profile", 0.7)
    assert FilletTool.profile == pytest.approx(0.7)
    _edit(viewport, "shape", "round")
    assert FilletTool.profile == pytest.approx(0.5)
    _type(viewport, "0,3p")                           # a typed number is custom
    assert tool.profile == pytest.approx(0.3) and tool.shape is None


def test_a_flat_profile_with_and_without_the_corner_fill(viewport):
    for corners, per_corner in (("round", 9), ("cut", 3)):
        mesh = _box_scene(viewport)
        tool = FilletTool()
        viewport.set_active_tool(tool)
        viewport.scene.select(list(mesh.edges))
        tool.on_activate(viewport)
        _edit(viewport, "shape", "chamfer")
        _edit(viewport, "segments", 3)
        _edit(viewport, "corners", corners)
        _edit(viewport, "amount", "0.1")
        _edit(viewport, "apply")
        assert len(mesh.faces) == 6 + 12 * 3 + 8 * per_corner and is_closed(mesh)


def test_a_drawn_profile_from_the_panel(viewport):
    mesh = _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    viewport.scene.select([edge_at(mesh, (0, 0, 1), (2, 0, 1))])
    tool.on_activate(viewport)
    for name in ("steps", "supports", "cornice", "crown"):
        _edit(viewport, "shape", name)
        assert tool.shape == name
    _edit(viewport, "shape", "steps")
    assert FilletTool.shape == "steps"
    _edit(viewport, "segments", 6)
    assert len(tool.preview_faces()) == 6               # one strip per step
    assert "Steps" in tool.value_label()[0]
    _edit(viewport, "amount", "0.2")
    _edit(viewport, "apply")
    assert len(mesh.faces) == 6 + 6 and is_closed(mesh)
    assert abs(signed_volume(mesh)) == pytest.approx(2.0 - (1 - 1 / 3) * 0.04 * 2, abs=1e-6)


def test_a_typed_profile_or_round_leaves_the_drawn_one(viewport):
    _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    _edit(viewport, "shape", "cornice")
    assert FilletTool.shape == "cornice"
    _type(viewport, "0,3p")
    assert FilletTool.shape is None and tool.profile == pytest.approx(0.3)
    _edit(viewport, "shape", "crown")
    _edit(viewport, "shape", "round")
    assert FilletTool.shape is None and FilletTool.profile == pytest.approx(0.5)


def test_drawn_profiles_are_for_edges_only(viewport):
    _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    _edit(viewport, "shape", "steps")
    tool.set_affect(viewport, "vertices")
    assert FilletTool.shape is None                     # switching lets it go
    said = []
    viewport.flash_status = lambda text, *a, **k: said.append(text)
    _edit(viewport, "shape", "crown")
    assert FilletTool.shape is None
    assert any("edges only" in s for s in said)
    viewport.flash_status = lambda *a, **k: None


def test_the_editor_draws_the_profile_the_tool_uses(viewport):
    mesh = _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    viewport.scene.select(list(mesh.edges))
    tool.on_activate(viewport)
    from ingetrazo_core.bevel_profiles import controls
    _edit(viewport, "shape", "cornice")
    points = controls(tool.shape, tool.segments)
    assert len(points) == 13
    x, y, smooth = points[6]
    points[6] = (x - 0.1, y + 0.05, smooth)
    _edit(viewport, "curve", points)
    assert isinstance(tool.shape, tuple)
    assert tool.shape[6][0] == pytest.approx(x - 0.1)
    depth = len(viewport.history.undo_stack)
    _edit(viewport, "amount", "0.15")
    _edit(viewport, "apply")
    assert len(viewport.history.undo_stack) == depth + 1
    assert is_closed(mesh) and all(len(e.faces) == 2 for e in mesh.edges)


def test_choosing_drawn_starts_from_the_current_outline(viewport):
    _box_scene(viewport)
    tool = FilletTool()
    viewport.set_active_tool(tool)
    _edit(viewport, "shape", "steps")
    _edit(viewport, "shape", "drawn")
    assert isinstance(FilletTool.shape, tuple)
    assert all(not sm for _x, _y, sm in FilletTool.shape)   # steps are corners
