"""Tools retain their gesture and measurement input across native chrome actions."""
import pytest

from core.input_types import QPointF, Qt
from ingetrazo_core import Vec3 as V
from tests.shell_isolation import native_question_host, restore_shell_patches  # noqa: F401
from tools.base import ToolContext


def key(host, code, text="", modifiers=0):
    host.key(True, int(code), modifiers, text)
    host.key(False, int(code), modifiers, text)


def type_value(host, entry):
    for char in entry:
        key(host, ord(char.upper()), char)


def ctx(host, point):
    return ToolContext(host.view.tools, point, QPointF(), Qt.NoModifier, None)


def begin(host, shortcut):
    key(host, ord(shortcut), shortcut.lower())
    tools = host.view.tools
    tools.sync()
    return tools.tool


def test_tool_shortcuts_and_tab_reach_the_initial_model(native_host):
    line = begin(native_host, "L")
    assert line is native_host.window._tools["line"]
    line.on_click(ctx(native_host, V()))
    for modifiers in (0, Qt.ShiftModifier.value):
        key(native_host, Qt.Key_Tab, modifiers=modifiers)
        assert native_host.viewport.active_tool is line and line.start_point == V()


def test_dismissing_a_chrome_question_returns_arrow_and_letter_keys_to_the_live_tool(native_question_host):
    from views import chrome_forms
    host = native_question_host
    line = begin(host, "L")
    line.on_click(ctx(host, V()))
    chrome_forms.ask_text(host.window, "Name", "Name", "", done=lambda _: None)
    key(host, ord("R"), "r")
    assert host.viewport.active_tool is line
    chrome_forms.current(host.window).press("cancel")
    assert line.start_point == V()
    key(host, Qt.Key_Right)
    assert host.viewport.axis_lock == "x"
    key(host, ord("R"), "r")
    assert host.viewport.active_tool is host.window._tools["rectangle"]


def test_navigation_preserves_the_line_and_switching_tools_ends_it(native_host):
    host = native_host
    line = begin(host, "L")
    line.on_click(ctx(host, V()))
    for shortcut, mode in [("O", "orbit"), ("H", "pan")]:
        key(host, ord(shortcut), shortcut.lower())
        assert host.viewport.nav_mode == mode
    key(host, ord("L"), "l")
    assert host.viewport.active_tool is line and line.start_point == V()
    key(host, ord("R"), "r")
    assert line.start_point is None


@pytest.mark.parametrize("shortcut,entry,count", [("L", "3", 1), ("R", "3;1", 4), ("C", "3", 24)])
def test_retyping_a_measurement_replaces_the_placed_shape(native_host, shortcut, entry, count):
    host = native_host
    tool = begin(host, shortcut)
    for point in (V(), V(2, int(shortcut == "R"), 0)):
        tool.on_hover(ctx(host, point))
        tool.on_click(ctx(host, point))
    history = host.viewport.history
    steps = len(history.undo_stack)
    type_value(host, entry)
    key(host, Qt.Key_Return, "\r")
    assert len(history.undo_stack) == steps
    mesh = host.viewport.scene.mesh
    assert len(mesh.edges) == count
    assert max(v.position.x() for v in mesh.vertices) == pytest.approx(3.0)


def test_expression_beats_model_shortcuts_and_draws_its_result(native_host):
    host = native_host
    line = begin(host, "L")
    line.on_click(ctx(host, V()))
    line.on_hover(ctx(host, V(2, 0, 0)))
    entry = "=(1.20m+35cm)*2/2-5cm"
    type_value(host, entry)
    assert host.viewport._value_buffer == entry
    key(host, Qt.Key_Return, "\r")
    assert line.start_point.x() == pytest.approx(1.5)
    assert host.viewport.active_tool is line


def test_zoom_accepts_calculated_lens_and_field_of_view(native_host):
    cam = native_host.viewport.camera
    cam.set_projection("perspective")
    native_host.viewport.set_nav_mode("zoom")
    type_value(native_host, "=20mm+1.5cm")
    key(native_host, Qt.Key_Return, "\r")
    assert cam.focal_mm == pytest.approx(35)
    type_value(native_host, "=90/2")
    key(native_host, Qt.Key_Return, "\r")
    assert cam.fov_deg == pytest.approx(45)
