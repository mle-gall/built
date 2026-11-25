"""Down arrow over an edge: parallel, then perpendicular, then free.

the modeller's Line tool: with the cursor over an edge, the Down arrow locks the
line parallel to that edge (magenta); Down again turns the lock
perpendicular to it, in the plane being drawn; a third Down frees it. The
edge is the one under the cursor or, the cursor having left it, the last
edge it referenced while the line was drawn.

The GL viewport had the cycle, but read an edge that only its own hover
picks set: on the Metal view nothing fed it, so Down did nothing there.
Both views are driven here with real key events.
"""
from built_render import KeyEvent
import math

import pytest
from built_core import Input as Qt
from built_core import Vec3 as V

from tests.test_.skp_inference_gaps import (  # noqa: F401 (fixture)
    click, move, refresh, same, turned, view)


def key(widget, k, modifiers=Qt.NoModifier):
    widget.keyPressEvent(KeyEvent(True, k, modifiers))
    widget.keyReleaseEvent(KeyEvent(False, k, modifiers))


@pytest.fixture
def drawing(view):
    """A line started a metre off an edge turned 30° (no axis in play)."""
    view.scene.mesh.add_edge(turned(0, 0), turned(2, 0))
    refresh(view)
    click(view, turned(1, -1))
    return view


def test_down_over_an_edge_locks_parallel_then_perpendicular_then_frees(drawing):
    view = drawing
    move(view, turned(1.5, 0))                 # over the edge
    key(view, Qt.Key_Down)
    source = view.source_viewport
    assert source.reference_mode == "parallel"
    # Far from parallel and far from the edge: the lock still holds.
    snap = move(view, turned(1.9, -0.4))
    assert snap.kind == "reference", snap.kind
    assert snap.color == pytest.approx((0.85, 0.30, 0.80))
    # On the parallel through the start point.
    assert abs(_local(snap.point)[1] + 1) < 2e-3, _local(snap.point)

    key(view, Qt.Key_Down)
    assert source.reference_mode == "perpendicular"
    snap = move(view, turned(1.4, -0.3))
    assert snap.kind == "reference", snap.kind
    assert abs(_local(snap.point)[0] - 1) < 2e-3, _local(snap.point)

    key(view, Qt.Key_Down)
    assert source.reference_mode is None and source.reference_edge is None
    snap = move(view, turned(1.4, -0.3))
    assert snap.kind != "reference" or not same(snap.point, turned(1, -0.3))


def test_down_uses_the_last_referenced_edge_once_the_cursor_left_it(drawing):
    view = drawing
    move(view, turned(1.5, 0))                 # reference the edge…
    move(view, turned(1.3, -0.6))              # …and leave it
    key(view, Qt.Key_Down)
    assert view.source_viewport.reference_mode == "parallel"
    snap = move(view, turned(1.9, -0.4))
    assert snap.kind == "reference"
    assert abs(_local(snap.point)[1] + 1) < 2e-3, _local(snap.point)


def test_down_with_no_edge_referenced_keeps_todays_behaviour(view):
    """Nothing to be parallel to: Down locks nothing."""
    refresh(view)
    click(view, V(0, 0, 0))
    move(view, V(0.7, 0.4, 0))
    key(view, Qt.Key_Down)
    assert view.source_viewport.reference_mode is None


def test_escape_releases_the_reference_first(drawing):
    view = drawing
    move(view, turned(1.5, 0))
    key(view, Qt.Key_Down)
    assert view.source_viewport.reference_mode == "parallel"
    view.source_viewport.escape()
    assert view.source_viewport.reference_mode is None
    assert view.tools.tool.start_point is not None   # the line goes on


_C, _S = math.cos(math.radians(30)), math.sin(math.radians(30))


def _local(p):
    """``p`` in the turned frame of :func:`turned`."""
    x, y = p.x() - 1, p.y()
    return (_C * x + _S * y, -_S * x + _C * y)


# ---- The GL viewport ---------------------------------------------------------

def test_gl_down_takes_the_hovered_edge_else_the_last_referenced_one():
    from types import SimpleNamespace
    from tools.rust_tools import LineTool
    from tests.controller_surface import ControllerSurface as Viewport
    vp = Viewport()
    vp.active_tool = LineTool()
    hovered = SimpleNamespace(a=V(0, 0, 0), b=V(1, 1, 0))
    referenced = SimpleNamespace(a=V(0, 0, 0), b=V(1, 2, 0))
    vp.inference._acquired_edge = referenced
    vp._hover_edge = hovered
    key(vp, Qt.Key_Down)
    assert vp.reference_mode == "parallel" and vp.reference_edge is hovered
    key(vp, Qt.Key_Down)
    assert vp.reference_mode == "perpendicular" and vp.reference_edge is hovered
    key(vp, Qt.Key_Down)
    assert vp.reference_mode is None and vp.reference_edge is None
    vp._hover_edge = None                      # the cursor left the edge
    key(vp, Qt.Key_Down)
    assert vp.reference_mode == "parallel" and vp.reference_edge is referenced
    vp.inference._acquired_edge = None
    key(vp, Qt.Key_Down)
    key(vp, Qt.Key_Down)
    assert vp.reference_mode is None
    key(vp, Qt.Key_Down)                       # nothing referenced: no lock
    assert vp.reference_mode is None
    vp.close()
