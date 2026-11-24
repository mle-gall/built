"""Connect the Rust pane layout and GPU targets to the document's editing controllers."""
from __future__ import annotations

from ingetrazo_render import ModelViews, view_layouts
from app_shell.view import ShellView
from views.document_controller import DocumentController
from views.resident_peer import ResidentPeer, TargetRenderer, raw_renderer


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


class DocumentViews:
    def __init__(self, doc):
        self.doc = doc
        self.layout = ModelViews()
        self.displays = {}
        self.poses = {}
        self.visible = False
        from core.settings import settings
        try:
            self.snap_pixels = max(0, int(settings().value("views/divider_snap_px", 16)))
        except (TypeError, ValueError):
            self.snap_pixels = 16

    def choose(self, key):
        self.layout.choose(key)
        self.doc.view_layout = key
        if key == "single":
            self.suspend()
        self.doc.stand_in.update()

    def _pane(self, number, width, height):
        primary = self.doc.stand_in
        main = self.doc.viewport
        if number not in self.displays:
            host = PaneHost(primary._host)
            display = Pane(primary, host)
            display.idle_tools = display.tools
            if number == 1:
                vp = main
                vp._host = host
            else:
                vp = DocumentController(host)
                # A fresh controller can join the live document without the
                # scene version bump needed when replacing a used controller.
                vp.scene, vp.history = main.scene, main.history
                vp._bind_bake_store()
                for key in ("_ext_overlays", "_ext_snap_providers", "_ext_pickables"):
                    if hasattr(main, key):
                        setattr(vp, key, getattr(main, key))
                if number in self.poses:
                    vp.camera, vp.view_style = self.poses[number]
                else:
                    vp.camera.target = type(main.camera.target)(main.camera.target)
                    vp.camera.distance = main.camera.distance
                    vp.camera.perspective = False
                    vp.camera.set_view(self.layout.initial_view(number))
            display.source_viewport = vp
            vp.keys_stand_in = display
            display.camera = vp.camera
            self.displays[number] = display
        display = self.displays[number]
        display._host.width, display._host.height = max(1, width), max(1, height)
        return display

    def suspend(self):
        self.visible = False
        self.layout.cancel()
        main, primary = self.doc.viewport, self.doc.stand_in
        if self.displays:
            self.activate(main)
        main.keys_stand_in = primary
        main._host = primary._host
        self.doc.front_view = None
        primary.tools.view = primary
        main.tool_preview = True
        for number, display in self.displays.items():
            vp = display.source_viewport
            if number != 1:
                self.poses[number] = (vp.camera, getattr(vp, "view_style", None))
            display._closed = True
            for timer in display._presentation_timers():
                if timer is not None:
                    timer.stop()
            display.release()
        self.displays.clear()

    def activate(self, vp):
        previous = self.doc.front_view or self.doc.viewport
        # Transfer the in-progress operation intact. Deactivating a tool
        # here would cancel the first endpoint of a line drawn across panes.
        tools = self.doc.stand_in.tools
        display = vp.keys_stand_in
        if previous is not vp:
            vp.active_tool, previous.active_tool = previous.active_tool, None
            vp.suspended_tool, previous.suspended_tool = previous.suspended_tool, None
            vp.nav_mode, previous.nav_mode = previous.nav_mode, None
            vp._value_buffer = previous._value_buffer
            previous.tool_preview = False
        if tools.view is not display:
            if isinstance(tools.view, Pane):
                tools.view.tools = tools.view.idle_tools
            tools.view = display
            display.tools = tools
            display._preview_key = None
            tools.claim()
        vp.tool_preview = True
        self.doc.front_view = vp

    def pointer(self, kind, x, y, held):
        hit = self.layout.pointer(kind, x, y, bool(held))
        if hit is None:
            return None
        index, x, y = hit
        pane = self.displays[index]
        if self.layout.active == index:
            self.activate(pane.source_viewport)
        return pane, x, y

    def frame(self):
        from time import perf_counter
        from core.i18n import tr
        from views import wgpu_sheets, wgpu_ui
        primary = self.doc.stand_in
        self.visible = False
        if (self.doc.view_layout == "single" or primary.renderer is None or
                not getattr(primary, "_ui_attached", False) or wgpu_ui.chrome_only(primary)
                or wgpu_sheets.shown(primary) or primary.packed is None):
            return primary.draw_frame()
        area = primary.renderer.ui_free_ndc()
        if area is None:
            return primary.draw_frame()
        start = perf_counter()
        if primary._scene_dirty:
            primary._upload()
        panes = self.layout.arrange(primary.width(), primary.height(), area)
        if not panes:
            return primary.draw_frame()
        visible = {index for index, *_ in panes}
        for number, display in self.displays.items():
            if number not in visible and display.renderer is not None:
                display.release()
        displays = [self._pane(index, width, height) for index, _, _, width, height in panes]
        self.activate(self.displays[self.layout.active].source_viewport)
        batch = []
        for display, (index, x, y, width, height) in zip(displays, panes):
            display.source_viewport.tool_preview = self.layout.active == index
            display._host.x, display._host.y = x, y
            display.draw_frame()
            camera = display.source_viewport.camera
            title = self.layout.title(camera.forward().toTuple(), camera.projection_mode())
            batch.append((display._target, (x, y, width, height), self.layout.active == index, tr(title)))
        self.visible = True
        wgpu_ui.before_frame(primary)
        cpu, gpu = primary.renderer.present_model_views(batch, self.layout.guides())
        wgpu_ui.after_frame(primary)
        elapsed = (perf_counter()-start)*1000
        primary.frameReady.emit(cpu, gpu, elapsed)
        return cpu, gpu, elapsed


def for_document(doc):
    if not hasattr(doc, "model_views"):
        doc.model_views = DocumentViews(doc)
    return doc.model_views


def choose(win):
    from core.i18n import tr
    from views.chrome_forms import ask_choice
    grid = for_document(win._doc)
    options = {tr(label): key for key, label, _, _ in view_layouts()}
    current = list(options.values()).index(grid.layout.layout)
    ask_choice(win, tr("Model Views"), tr("Layout"), options, current,
               done=lambda label: grid.choose(options[label]) if label in options else None)
