"""The Zoom tool's lens goes through the same value field as the other
tools (ui/value_field.rs): what is typed shows by the cursor, and the lens
is part of the revision, so Enter reads the applied lens back."""
from types import SimpleNamespace

from built_core import PixelPoint as QPoint

from core.camera import OrbitCamera


class _Renderer:
    def __init__(self):
        self.sent = []

    def ui_set_cursor_value(self, *args):
        self.sent.append(args)


def _view(buffer, cam):
    source = SimpleNamespace(nav_mode="zoom", camera=cam, active_tool=None)
    return SimpleNamespace(tools=SimpleNamespace(active_tool=None, value_buffer=buffer),
                           source_viewport=source, renderer=_Renderer(), _cursor_pos=QPoint(400, 300))


def test_a_typed_lens_shows_in_the_value_field_and_its_change_is_a_new_revision():
    from views import wgpu_ui
    cam = OrbitCamera()
    cam.perspective = True
    win = SimpleNamespace(viewport=SimpleNamespace(history=SimpleNamespace(revision=7)))
    view = _view("50mm", cam)
    wgpu_ui._cursor_value(view, win)
    caption, value, typing, x, y, start, before = view.renderer.sent[-1]
    assert (caption, value, typing, start) == ("Field of view", "50mm", True, None)
    assert (x, y) == (400.0, 260.0)
    # Enter applied it: not typing, the view's focal length, a new revision.
    cam.fov_deg = cam.fov_for_focal(50.0)
    view.tools.value_buffer = ""
    wgpu_ui._cursor_value(view, win)
    _, value, typing, *_, after = view.renderer.sent[-1]
    assert (value, typing) == ("50 mm", False) and after != before
