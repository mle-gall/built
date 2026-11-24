"""Recording GPU targets driven by the native document, input and frame paths."""
import pytest

from app_shell.events import MouseEvent
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views.resident_peer import TargetRenderer

class FakeRenderer:
    """Records each call with the target live at the time."""



    def __init__(self, ns_view, width, height, scale):
        self.calls = []
        self.active = 0
        self.alive = {0}
        self.next = 1
        self.sizes = {0: (width, height)}


    # The view grid's API.
    def add_tile_target(self, width, height, scale):
        target, self.next = self.next, self.next + 1
        self.alive.add(target)
        self.sizes[target] = (width, height)
        self.calls.append((self.active, "add_target", (target,)))
        return target

    def bind_target(self, target):
        if target not in self.alive:
            raise ValueError("no such view target")
        self.active = target

    def remove_target(self, target):
        if target == 0 or target not in self.alive:
            raise ValueError("no such removable view target")
        self.alive.discard(target)
        if self.active == target:
            self.active = 0
        self.calls.append((self.active, "remove_target", (target,)))

    @property
    def active_target(self):
        return self.active

    # What a frame asks for an answer.
    def render(self, present=True):
        self.calls.append((self.active, "render", ()))
        return 1.0, None

    def pick(self, x, y):
        return -1

    def pick_rect(self, *a):
        return []

    gpu_memory = 1 << 20
    target_memory = 0
    pick_cache_valid = True

    def __getattr__(self, name):
        if name.startswith("__"):
            raise AttributeError(name)

        def record(*args, **kwargs):
            self.calls.append((self.active, name, args))
        return record

    def named(self, name, target=None):
        return [c for c in self.calls if c[1] == name
                and (target is None or c[0] == target)]



class ModelWindow:
    def __init__(self, host):
        self.host = host
        self.win = host.window
        self.renderer = FakeRenderer(0, host.width, host.height, 1.)
        self.renderer.ui_free_ndc = lambda: [-1., -1., 1., 1.]
        self.renderer.present_model_views = self.present
        self.targets = {}

    def present(self, rows, guides):
        self.targets = {name: (target, rect, active) for target, rect, active, name in rows}
        return 1., None

    def frame(self):
        view = self.win.view
        if not isinstance(view.renderer, TargetRenderer):
            view.renderer = TargetRenderer(self.renderer, 0)
        view._ui_attached = True
        scene = self.win.viewport.scene
        if getattr(view, "scene", None) is not scene or getattr(view, "_shell_resident_version", None) != scene.version:
            view.set_scene(scene)
            view._shell_resident_version = scene.version
        if view._scene_dirty:
            view._upload(force=True)
        return self.win._doc.model_views.frame(self.win._doc)

    def choose(self, layout):
        self.win.set_view_layout(layout)
        self.frame()

    def panes(self):
        return {pane._target: pane for pane in self.win.view.peers()}

    def point(self, title, kind="move"):
        target, (x, y, w, h), _ = self.targets[title]
        doc = self.win._doc
        button = 1 if kind in ("press", "release") else 0
        ev = MouseEvent(kind, x + w/3, y + h/3, button, int(kind=="press"), 0)
        method = {"move": "mouseMoveEvent", "press": "mousePressEvent", "release": "mouseReleaseEvent"}[kind]
        result = doc.model_views.route(doc, kind, method, ev, int(kind=="press"))
        assert result is self.panes()[target]
        return result.source_viewport

    def click(self, title):
        self.point(title, "press")
        return self.point(title, "release")


@pytest.fixture
def grid(native_host, monkeypatch):
    from views import wgpu_ui, wgpu_view
    window = ModelWindow(native_host)
    monkeypatch.setattr(wgpu_ui, "before_frame", lambda view: None)
    monkeypatch.setattr(wgpu_ui, "after_frame", lambda view: None)
    monkeypatch.setattr(wgpu_view, "_ui_claims", lambda *_: False)
    window.choose("four")
    return window
