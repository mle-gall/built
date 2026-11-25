"""Native Metal input and presentation sharing the document camera and selection."""
import ingetrazo_core
from contextlib import ExitStack, contextmanager
import math
import time
import numpy as np
from core.input_codes import Input as Qt, Type as QEvent
from core.host_callbacks import Signal
from core.timers import later
from ingetrazo_render import PaintColor as QColor
from ingetrazo_core import Vec3 as QVector3D
from core.camera import OrbitCamera
from ingetrazo_core import pack_scene
from tools.rust_tools import selection_mode


def _tool_frame(method):
    from functools import wraps
    @wraps(method)
    def handle(view, *args, **kwargs):
        if getattr(view, '_closed', False):
            return None
        if args and _ui_claims(view, method.__name__, args[0]):
            return None
        remote = getattr(getattr(view, 'packed', None), 'model_translations', None) is not None
        if remote:
            view._sync_scene_camera()
        origin = getattr(getattr(view, "packed", None), "origin", ())
        prototypes = getattr(view, '_remote_meshes', ())
        if any(origin) or prototypes:
            with view._geometry_frame():
                result = method(view, *args, **kwargs)
        else:
            result = method(view, *args, **kwargs)
        if remote and getattr(view.packed, 'model_translations', None) is not None:
            view._sync_scene_camera()
        return result
    return handle


import ingetrazo_render


from views.view_navigation import ViewNavigation


class MetalView(ViewNavigation):
    frameReady = Signal(float, object, float)
    _ZOOM_OUT_STEP = 0.05

    @_tool_frame
    def mousePressEvent(self, event):
        self.setFocus()
        self.last = self.press = event.position()
        self._drag_nav = event.button() == Qt.MiddleButton or bool(
            getattr(self.source_viewport, 'nav_mode', None))
        mode = self.tools.sync()
        if not self._drag_nav and mode == 'select' and event.button() == Qt.LeftButton:
            # The Select machine owns the gesture: a press in place right
            # after a double click is the triple click, taken at once.
            tool = self.tools.select_tool()
            tool.press(self.tools, self.press.x(), self.press.y(), event.modifiers(),
                       float(event.timestamp()))
            if not tool.pressed():
                self.last = None        # the release that follows is no click
                if self.source_viewport is not None:
                    self.source_viewport.notify_scene_changed()
                self.sync_selection()
                self.update()
                return
        if (not self._drag_nav and callable(getattr(self.tools.tool, 'on_look', None))
                and event.button() in (Qt.LeftButton, Qt.RightButton)):
            self._begin_look(event)
            return
        if not self._drag_nav and event.button() == Qt.LeftButton and mode in ('tape', 'move', 'mesh'):
            self.hover(self.press.x(), self.press.y(), event.modifiers())
            self.tools.click(event)
        if self._drag_nav:
            self._pivot = self._orbit_pivot_at(self.press.x(), self.press.y())

    @_tool_frame
    def mouseMoveEvent(self, event):
        if getattr(self, '_look_drag', None) is not None:
            if event.buttons():
                self._look_move(event)
                return
            self._look_drag = None
            self.tools._apply_tool_cursor()
        self._cursor_pos = event.position()
        if not event.buttons():
            self.last = None
            self.hover(event.position().x(), event.position().y(), event.modifiers())
            self.tools.hover(event)
            return
        if self.last is None:
            return
        delta = event.position().toPoint()-self.last.toPoint()
        self.last = event.position()
        mode = getattr(self.source_viewport, 'nav_mode', None)
        if self._drag_nav and mode != 'zoom_window':
            ingetrazo_render.camera_drag(self, mode, delta, bool(event.modifiers() & Qt.ShiftModifier))
        elif self.tools.sync() in ('tape', 'move', 'mesh'):
            self.hover(event.position().x(), event.position().y(), event.modifiers())
            self.tools.hover(event)
        elif event.buttons() & Qt.LeftButton and mode == 'zoom_window':
            from views import wgpu_overlay
            self.overlay_primitives = [wgpu_overlay.selection_rectangle(
                (self.press.x(), self.press.y()), (self.last.x(), self.last.y()), zoom=True)]
        elif event.buttons() & Qt.LeftButton and self.tools.mode == 'select':
            from views import wgpu_overlay
            # The machine shows a box only past its drag threshold (a click
            # that jitters a pixel is no box).
            box = self.tools.select_tool().drag(self.tools, self.last.x(), self.last.y(),
                                                event.modifiers())
            self.overlay_primitives = [] if box is None else [wgpu_overlay.selection_rectangle(
                box[0][:2], box[0][2:], zoom=False)]
        self.update()

    @property
    def active_tool(self):
        return self.tools.tool

    @_tool_frame
    def focusOutEvent(self, event):
        # As GL's: the walking keys' releases will land elsewhere, but the
        # tool goes on. Cancelling it here reset a line whose first point
        # was down as soon as a tray, a field or a dialog took the focus.
        self._look_drag = None
        hook = getattr(self.tools.tool, 'on_focus_out', None)
        if hook is not None:
            hook(self.tools)
        super().focusOutEvent(event)

    def focusNextPrevChild(self, next):
        # Tab is a tool key here (reserved for the tools, views.shortcuts):
        # Qt's focus chain took it and left the keyboard on a tray button.
        return False

    def leaveEvent(self, event):
        from views import wgpu_ui
        wgpu_ui.pointer(self, 'leave')
        super().leaveEvent(event)

    @_tool_frame
    def mouseReleaseEvent(self, event):
        if getattr(self, '_look_drag', None) is not None:
            self._look_drag = None
            self.tools._apply_tool_cursor()
            self.last = None
            return
        if self.last is None:
            return
        mode = getattr(self.source_viewport, 'nav_mode', None)
        if self._drag_nav:
            if mode == 'zoom_window':
                self._zoom_to_box(self.press, event.position())
        elif event.button() == Qt.LeftButton and self.tools.sync() == 'select':
            # A release in place is a click, else the box (crossing when
            # dragged right to left): the machine decides.
            end = event.position()
            self.tools.select_tool().release(self.tools, end.x(), end.y(), event.modifiers())
            if self.source_viewport is not None:
                self.source_viewport.notify_scene_changed()
            self.sync_selection()
        if (self.tools.mode == 'mesh' and event.button() == Qt.LeftButton
                and not self._drag_nav):
            # GL's: a left release only. A wheel-button orbit's release
            # committed a Push/Pull in progress (a release far from its
            # press is a stroke) and set Rotate's axis along the orbit.
            self.tools.tool.on_release(self.tools)
            self.tools.update()
            self.overlay_primitives = []
            self.tools.mesh_feedback()
        self.last = None
        if self.tools.mode == 'select' or mode == 'zoom_window':
            self.overlay_primitives = []
        self.update()

    def contextMenuEvent(self, event):
        from views import wgpu_sheets
        if wgpu_sheets.context_menu(self, event):   # the sheet's own menus
            return
        point = event.pos()
        from ingetrazo_render import context_menu_available
        from plugins._context_menu import context_menu_open_at
        chrome = context_menu_available(self)
        if chrome and self.renderer.ui_over_chrome(float(point.x()), float(point.y())):
            return
        self.tools.sync()
        tool = self.tools.tool
        menu = getattr(tool, 'context_menu', None)
        if menu is not None and menu(self.tools, event.globalPos()):
            return
        source = self.source_viewport
        if source is None:
            return
        # With the chrome on (the native app, or the Qt host), the menu is the
        # chrome's: only the verbs for what was hit (docs/context-menu.md).
        # A right-click on the chrome itself is the chrome's own.
        if not chrome:
            source.flash_status("Built could not draw the context menu.")
            return
        context_menu_open_at(self, point.x(), point.y())

    @_tool_frame
    def mouseDoubleClickEvent(self, event):
        # A double click of the wheel recentres the view on the point under
        # the cursor (on sky: at the depth of the view's centre); the tool in
        # use never hears of it and no orbit starts, as in GL (#979, #1154).
        if event.button() == Qt.MiddleButton:
            self.last = None
            self._recentre_on(event.position().x(), event.position().y())
            return
        if event.button() == Qt.LeftButton and getattr(self.source_viewport, 'nav_mode', None):
            return self.mousePressEvent(event)
        mode = self.tools.sync()
        if mode in ('tape', 'move', 'mesh'):
            self.hover(event.position().x(), event.position().y(), event.modifiers())
            if mode == 'mesh':
                ctx = self.tools.context(event)
                if ctx is not None:
                    self.tools.tool.on_double_click(ctx)
                    self.tools.update()
            else:
                self.tools.click(event)
            return
        if mode != 'select':
            return
        self.tools.select_tool().double(self.tools, event.position().x(), event.position().y(),
                                        event.modifiers(), float(event.timestamp()))
        self.update()

    def event(self, event):
        if event.type() == QEvent.NativeGesture:
            from views import wgpu_sheets
            if wgpu_sheets.gesture(self, event):    # pinch on the sheet
                return True
            # Pinch, smart zoom: a trackpad's, or the touch screen's (#248).
            from ingetrazo_render import trackpad_nav
            if trackpad_nav.native_gesture(self, event):
                return True
        if event.type() == QEvent.ShortcutOverride:
            _note_ctrl_tap(self, event)
            from views import wgpu_ui
            if wgpu_ui.claims_key(self, event):
                event.accept()
                return True
        if event.type() == QEvent.ShortcutOverride and hasattr(self, 'tools'):
            mode = self.tools.sync()
            from views.document_interaction import DocumentInteraction
            target = self.tools if mode == 'mesh' else self.source_viewport
            if target is not None:
                buffer = self.tools.value_buffer if mode == 'mesh' else target._value_buffer
                if DocumentInteraction._measurement_claims_key(target, event, buffer):
                    event.accept()
                    return True
            claims = getattr(self.tools.tool, 'claims_key', None)
            if claims is not None and claims(event.key(), event.modifiers()):
                event.accept()
                return True
        return super().event(event)

    @_tool_frame
    def keyPressEvent(self, event):
        _note_ctrl_tap(self, event)
        mode = self.tools.sync()
        if event.key() == Qt.Key_Escape:
            from ingetrazo_render import model_escape
            from views.document_interaction import DocumentInteraction
            source = self.source_viewport
            scene = self.tools.scene
            busy = bool(getattr(self, "_look_drag", None)
                        or self.tools.start_point is not None or DocumentInteraction._tool_busy_for_delete(self.tools.tool))
            from ingetrazo_render.shell_views import raw_renderer
            chrome = getattr(self, "_shared", self)
            renderer = raw_renderer(chrome.renderer) if getattr(chrome, "_ui_attached", False) else None
            owner = model_escape(busy, bool(scene is not None and scene.selection),
                                 source is not None and scene.edit_group is not None, renderer,
                                 self.tools._array_pending(), bool(self.tools.value_buffer),
                                 getattr(source, "axis_lock", None) is not None or getattr(source, "reference_mode", None) is not None)
            self._look_drag = None
            from ingetrazo_render import leave_navigation
            win = self.window() if hasattr(self, "window") else None
            if (self.source_viewport is not None and getattr(win, "viewport", None) is self.source_viewport
                    and leave_navigation(win)):
                return
            self.tools._apply_tool_cursor()
            if owner == "value":
                self.tools._set_value_buffer("")
            elif owner == "constraint":
                source.release_constraints()
            elif owner == "tool":
                self.tools.cancel()
            elif owner == "array":
                self.tools.tool.close_array_window()
            elif owner == "edit":
                source.end_one_group_edit()
            elif owner == "selection":
                scene.clear_selection()
                if source is not None:
                    source.notify_scene_changed()
            if owner in ("edit", "selection"):
                self.last = None
                self.overlay_primitives = []
                self.sync_selection()
        elif self.tools.key(event):
            pass
        elif event.key() == Qt.Key_Down and mode in ('tape', 'mesh'):
            self._down_reference(event)
        elif mode == 'inert':
            event.ignore()
        elif self.source_viewport is not None:
            self.source_viewport.keyPressEvent(event)
        self.update()

    def _down_reference(self, event):
        """The Down arrow no tool took: GL's parallel → perpendicular →
        free cycle on the edge under the cursor, picked here (GL's hover
        edge is fed by GL's own picks only, so Down found nothing on this
        view), else on the last edge the line referenced."""
        source = self.source_viewport
        if source is None or event.isAutoRepeat():
            return
        pos = getattr(self, '_cursor_pos', None)
        edge = None
        if pos is not None and source.reference_mode is None:
            edge = self.tools.pick_edge_any(pos.x(), pos.y())
        # GL's Down press: what it locks outlives a held Shift's plane mark.
        source._shift_plane_mark = None
        source._cycle_reference_mode(edge)
        if pos is not None:
            # The lock shows at once, the hand still (GL's _refresh_snap).
            from ingetrazo_render import MouseEvent
            from ingetrazo_core import window_input
            held = window_input.modifiers(self)
            self.hover(pos.x(), pos.y(), held)
            self.tools.hover(MouseEvent("move", pos.x(), pos.y(),
                                       modifiers=int(getattr(held, "value", held))))

    @_tool_frame
    def keyReleaseEvent(self, event):
        mode = self.tools.sync()
        release = getattr(self.tools.tool, 'on_key_release', None)
        if mode == 'mesh' and event.isAutoRepeat() and callable(getattr(self.tools.tool, 'claims_key', None)):
            return
        if mode == 'mesh' and release is not None and release(self.tools, event.key()):
            self.tools.update()
            return
        if mode == 'inert':
            event.ignore()
            return
        if self.source_viewport is not None:
            self.source_viewport.keyReleaseEvent(event)

    def _depth_world_at(self, x, y):
        from views.wgpu_inference import surface, packed_surface
        row = self.pick_row(x, y)
        if row is None:
            return 'miss', None
        ray = self._pixel_to_ray(x, y)
        point = None
        if self.inference_index is not None:
            point = surface(self.inference_index, row, *ray)
        if point is None:
            # No snap index yet (worker still building it, or dropped by an
            # edit): the picked placement's triangles answer on their own.
            point = packed_surface(self.packed, row, *ray)
        if point is not None:
            return 'hit', point
        return 'miss', None

    def _model_point_at(self, x, y):
        """The model point under pixel ``(x, y)`` from the Metal pick, or
        ``None`` on sky or ground (GL's ``DocumentInteraction._model_point_at``)."""
        seen, point = self._depth_world_at(x, y)
        return point if seen == 'hit' else None

    def _orbit_pivot_at(self, x, y):
        return ingetrazo_render.metal_pivot(self, x, y)

    @_tool_frame
    def wheelEvent(self, event):
        from views.document_interaction import DocumentInteraction
        self._invert_wheel = getattr(self.source_viewport, '_invert_wheel', False)
        DocumentInteraction.wheelEvent(self, event)
        self.update()

    def set_highlight(self, selected_ids, hovered_id=None):
        if self.renderer is None:
            return
        indices = [i for uid in selected_ids for i in self.uid_rows.get(uid, ())]
        self.renderer.set_highlight(indices, list(self.uid_rows.get(hovered_id, ())))
        self.update()

    def rect_rows(self, x0, y0, x1, y1):
        if self.renderer is None:
            return []
        scale = self.devicePixelRatioF()
        x0, x1 = sorted((x0, x1))
        y0, y1 = sorted((y0, y1))
        self._prepare_pick_frame()
        return [row for row in self.renderer.pick_rect(*(max(0, int(v*scale)) for v in (x0, y0, x1, y1)))
                if self.resolve(row) is not None]

    def pick_rect(self, x0, y0, x1, y1):
        return [self.packed.uids[i] for i in self.rect_rows(x0, y0, x1, y1)]

    def closeEvent(self, event):
        self._closed = True
        for name in ('_refresh_timer', '_repaint_timer', '_resize_timer', '_settle_timer'):
            timer = getattr(self, name, None)
            if timer is not None:
                timer.stop()
        tracker = getattr(self, '_instance_updates', None)
        if tracker is not None and tracker.unsubscribe is not None:
            tracker.unsubscribe()
        self.renderer = None
        if hasattr(self, '_inference_worker'):
            self._inference_worker.shutdown(wait=False, cancel_futures=True)
        # A host's own close, when its base has one (a widget shim has none).
        close = getattr(super(), 'closeEvent', None)
        if close is not None:
            close(event)
        self._instance_updates = self._inference_future = self.inference_index = None
        self.source_viewport = self._tool_source = self.scene = self.packed = None
        self._billboards = self._reference_cache = None


# Transitional: the methods already native (metal_view.rs) go on the class.
ingetrazo_render.register_view_functions(globals())
ingetrazo_render.register_view_class(MetalView)
