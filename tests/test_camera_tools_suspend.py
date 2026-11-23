"""Orbit, Pan and Zoom in the middle of a tool keep the tool's state.

the modeller's camera tools are temporary: start a line, orbit (the wheel held,
or Orbit / Pan / Zoom from the toolbar or their keys), come back to the
Line, and the first point is still placed and the rubber band goes on —
the tool was suspended, then resumed. Here a camera mode deactivated the
tool (``on_deactivate`` resets every drawing tool) and the Metal view
cancelled it a second time when the tool changed; a middle-button orbit on
the Metal view also handed the tool a left-button release, which commits a
Push/Pull in progress. Picking any other tool still resets it, as before.
"""
import pytest
from PySide6.QtCore import QPointF, Qt
from PySide6.QtGui import QMouseEvent
from ingetrazo_core import Vec3 as V
from PySide6.QtWidgets import QApplication

from tools.arc import ArcTool
from tools.base import ToolContext
from tools.circle import CircleTool
from tools.line import LineTool
from tools.move import MoveTool
from tools.protractor import ProtractorTool
from tools.pushpull import PushPullTool
from tools.rectangle import RectangleTool
from tools.rotate import RotateTool
from tools.tape import TapeMeasureTool
from tests.controller_surface import ControllerSurface as Viewport

from tests.test_.skp_inference_gaps import (  # noqa: F401 (fixture)
    _event, click, move, refresh, view)

app = QApplication.instance() or QApplication([])

TOOLS = [LineTool, RectangleTool, MoveTool, RotateTool, TapeMeasureTool,
         ProtractorTool, PushPullTool, ArcTool, CircleTool]


def busy(tool):
    return Viewport._tool_busy(tool)


def middle_orbit(view, at):
    x, y = view._world_to_pixel(at)
    for kind, pos, buttons, button in (
            (QMouseEvent.MouseButtonPress, (x, y), Qt.MiddleButton, Qt.MiddleButton),
            (QMouseEvent.MouseMove, (x + 40, y + 25), Qt.MiddleButton, Qt.NoButton),
            (QMouseEvent.MouseButtonRelease, (x + 40, y + 25), Qt.NoButton, Qt.MiddleButton)):
        q = QPointF(*pos)
        QApplication.sendEvent(view, QMouseEvent(kind, q, q, button, buttons, Qt.NoModifier))


def left_drag(view, at):
    """A drag of the left button: what Orbit / Pan from the toolbar take."""
    x, y = view._world_to_pixel(at)
    for kind, pos, buttons, button in (
            (QMouseEvent.MouseButtonPress, (x, y), Qt.LeftButton, Qt.LeftButton),
            (QMouseEvent.MouseMove, (x + 30, y - 20), Qt.LeftButton, Qt.NoButton),
            (QMouseEvent.MouseButtonRelease, (x + 30, y - 20), Qt.NoButton, Qt.LeftButton)):
        q = QPointF(*pos)
        QApplication.sendEvent(view, QMouseEvent(kind, q, q, button, buttons, Qt.NoModifier))


def press(view, p):
    """A click that leaves the scene version alone: the shared ``click``
    bumps it, which Push/Pull reads as an outside edit and drops its drag."""
    _event(view, QMouseEvent.MouseButtonPress, p, Qt.LeftButton, Qt.LeftButton)
    _event(view, QMouseEvent.MouseButtonRelease, p, Qt.NoButton, Qt.LeftButton)


@pytest.fixture
def started(view, request):
    """``request.param``'s tool on a 2 m square, its first click made."""
    source = view.source_viewport
    view.scene.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    refresh(view)
    tool = request.param()
    source.set_active_tool(tool)
    view.tools.sync()
    move(view, V(0.7, 0.6, 0))
    press(view, V(0.7, 0.6, 0))
    move(view, V(1.2, 0.9, 0))
    assert busy(tool), type(tool).__name__
    return view, tool, QPointF(*view._world_to_pixel(V(0.7, 0.6, 0)))


def _same_state(tool, start):
    assert busy(tool), type(tool).__name__
    if start is not None:
        assert tool.start_point is not None and (tool.start_point - start).length() < 1e-6


@pytest.mark.parametrize("started", TOOLS, indirect=True,
                         ids=lambda t: t.__name__)
def test_a_middle_button_orbit_keeps_the_tool_going(started):
    view, tool, _ = started
    start = getattr(tool, "start_point", None)
    history = len(view.source_viewport.history.undo_stack)
    middle_orbit(view, V(1.2, 0.9, 0))
    _same_state(tool, start)
    assert view.tools.tool is tool
    # Nothing was committed by the orbit (a Push/Pull took its release).
    assert len(view.source_viewport.history.undo_stack) == history


@pytest.mark.parametrize("mode", ["orbit", "pan", "zoom"])
@pytest.mark.parametrize("started", TOOLS, indirect=True,
                         ids=lambda t: t.__name__)
def test_a_camera_tool_suspends_the_tool_and_picking_it_resumes(started, mode):
    view, tool, _ = started
    source = view.source_viewport
    start = getattr(tool, "start_point", None)
    source.set_nav_mode(mode)
    view.tools.sync()
    left_drag(view, V(1.0, 1.0, 0))
    source.set_nav_mode("pan" if mode != "pan" else "orbit")   # one more
    view.tools.sync()
    source.set_active_tool(tool)            # the tool's key, or its button
    assert view.tools.sync() in ("mesh", "tape")
    assert source.nav_mode is None and source.active_tool is tool
    _same_state(tool, start)
    assert view.tools.start_point is not None or start is None


def test_the_line_goes_on_from_its_first_point_after_an_orbit(view):
    source = view.source_viewport
    refresh(view)
    tool = LineTool()
    source.set_active_tool(tool)
    view.tools.sync()
    click(view, V(0, 0, 0))
    source.set_nav_mode("orbit")
    view.tools.sync()
    left_drag(view, V(0.5, 0.5, 0))
    source.set_active_tool(tool)
    view.tools.sync()
    move(view, V(1.3, 0.4, 0))
    assert tool.rubber_band_lines(), "the rubber band continues"
    click(view, V(1.3, 0.4, 0))
    ends = {(round(v.position.x(), 3), round(v.position.y(), 3))
            for e in view.scene.mesh.edges for v in (e.v0, e.v1)}
    assert (0.0, 0.0) in ends and (1.3, 0.4) in ends


def test_picking_another_tool_after_a_camera_tool_still_resets(view):
    source = view.source_viewport
    refresh(view)
    line = LineTool()
    source.set_active_tool(line)
    view.tools.sync()
    click(view, V(0, 0, 0))
    source.set_nav_mode("orbit")
    view.tools.sync()
    rect = RectangleTool()
    source.set_active_tool(rect)
    view.tools.sync()
    assert line.start_point is None
    # …and picking the line again now starts it fresh.
    source.set_active_tool(line)
    view.tools.sync()
    assert line.start_point is None


# ---- The GL viewport, on its own --------------------------------------------

def _gl_started(tool_cls=LineTool):
    vp = Viewport()
    tool = tool_cls()
    vp.set_active_tool(tool)
    tool.on_click(ToolContext(vp, V(1, 1, 0), QPointF(10, 10), Qt.NoModifier, None))
    assert tool.start_point is not None
    return vp, tool


def test_gl_camera_tool_suspends_and_the_same_tool_resumes():
    vp, tool = _gl_started()
    vp.axis_lock = "x"
    vp.set_nav_mode("orbit")
    assert vp.active_tool is None and vp.suspended_tool is tool
    vp.set_nav_mode("zoom")
    vp.set_active_tool(tool)
    assert vp.active_tool is tool and vp.nav_mode is None
    assert vp.suspended_tool is None
    assert tool.start_point == V(1, 1, 0)
    assert vp.axis_lock == "x"               # the line's lock goes on too
    # Picking it again, now active, still starts it over (#34).
    vp.set_active_tool(tool)
    assert tool.start_point is None
    vp.close()


def test_gl_another_tool_after_a_camera_tool_resets_the_suspended_one():
    vp, tool = _gl_started()
    vp.set_nav_mode("pan")
    other = RectangleTool()
    vp.set_active_tool(other)
    assert tool.start_point is None and vp.suspended_tool is None
    assert vp.active_tool is other
    vp.close()


def test_gl_select_after_a_camera_tool_resets_the_suspended_one():
    vp, tool = _gl_started()
    vp.set_nav_mode("orbit")
    vp.set_active_tool(None)
    assert tool.start_point is None and vp.suspended_tool is None
    vp.close()


def test_gl_clearing_the_camera_mode_leaves_no_tool_and_resets_the_suspended():
    vp, tool = _gl_started()
    vp.set_nav_mode("zoom")
    vp.set_nav_mode(None)
    assert vp.active_tool is None and vp.nav_mode is None
    assert vp.suspended_tool is None and tool.start_point is None
    vp.close()
