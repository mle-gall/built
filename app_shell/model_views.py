"""Connect the Rust pane layout and GPU targets to the document's editing controllers."""
from __future__ import annotations

from ingetrazo_render import DocumentViews, view_layouts
from views.documents import Document
from app_shell.view import ShellView
from views.document_controller import DocumentController
from views.resident_peer import ResidentPeer, TargetRenderer, raw_renderer


class ShellDocument(Document):
    def __init__(self):
        super().__init__()
        self.model_views = DocumentViews()

    @property
    def view_layout(self):
        return self.model_views.layout


class PaneHost:
    def __init__(self, window_host):
        self.window_host = window_host
        self.width = self.height = 1
        self.x = self.y = 0

    def __getattr__(self, name):
        return getattr(self.window_host, name)


class Pane(ResidentPeer, ShellView):
    @property
    def chrome_offset(self):
        return self._host.x, self._host.y

    def mapToGlobal(self, p):
        return type(p)(p.x() + self._host.x, p.y() + self._host.y)

    def mapFromGlobal(self, p):
        return type(p)(p.x() - self._host.x, p.y() - self._host.y)

    def _attach(self):
        if self.renderer is None:
            self.primary.add_peer(self)
            raw = raw_renderer(self.primary.renderer)
            self._physical_size = (self.width(), self.height(), self.devicePixelRatioF())
            self._target = raw.add_tile_target(*self._physical_size)
            self.renderer = TargetRenderer(raw, self._target)
        return True

    def draw_frame(self):
        if self.primary.packed is None:
            return None
        self._attach()
        size = (self.width(), self.height(), self.devicePixelRatioF())
        if size != self._physical_size:
            self._finish_resize()
        return self._present_frame()


    def dispose(self):
        self._closed = True
        for timer in self._presentation_timers():
            if timer is not None:
                timer.stop()
        self.release()


def make_pane(doc, reuse_main, initial_view, saved):
    """Bind one Rust pane to the existing Python camera, history and tools."""
    main, primary = doc.viewport, doc.stand_in
    host = PaneHost(primary._host)
    display = Pane(primary, host)
    display.idle_tools = display.tools
    if reuse_main:
        vp = main
        vp._host = host
    else:
        vp = DocumentController(host)
        vp.scene, vp.history = main.scene, main.history
        vp._bind_bake_store()
        for key in ("_ext_overlays", "_ext_snap_providers", "_ext_pickables"):
            if hasattr(main, key):
                setattr(vp, key, getattr(main, key))
        if saved is None:
            vp.camera.target = type(main.camera.target)(main.camera.target)
            vp.camera.distance = main.camera.distance
            vp.camera.perspective = False
            vp.camera.set_view(initial_view)
        else:
            vp.camera, vp.view_style = saved
    display.source_viewport = vp
    vp.keys_stand_in = display
    display.camera = vp.camera
    return display


def choose(win):
    from core.i18n import tr
    from views.chrome_forms import ask_choice
    doc = win._doc
    options = {tr(label): key for key, label, _, _ in view_layouts()}
    ask_choice(win, tr("Model Views"), tr("Layout"), options,
               list(options.values()).index(doc.view_layout),
               done=lambda label: doc.model_views.choose(doc, options[label]) if label in options else None)
