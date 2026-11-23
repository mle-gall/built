"""Native tool dispatch shares measurement and History semantics with GL.

Live placement moves alter resident matrices only; the document is touched
once, by the same commands used by MoveTool, after the user commits.
"""
import numpy as np
from PySide6.QtCore import Qt
from core.history import MoveGroupCommand, CompoundCommand
from tools.base import ToolContext
from tools.tape import TapeMeasureTool

TOOL_DISPATCH = {'SelectTool': 'select',
 'TapeMeasureTool': 'tape',
 'MoveTool': 'move',
 'SectionPlaneTool': 'mesh',
 'LineTool': 'mesh',
 'RectangleTool': 'mesh',
 'PushPullTool': 'mesh',
 'EraserTool': 'mesh',
 'RotateTool': 'mesh',
 'ScaleTool': 'mesh',
 'OffsetTool': 'mesh',
 'CircleTool': 'mesh',
 'PolygonTool': 'mesh',
 'ArcTool': 'mesh',
 'PaintTool': 'mesh',
 'TexturePositionTool': 'mesh',
 'DimensionTool': 'mesh',
 'TextTool': 'mesh',
 'FollowMeTool': 'mesh',
 'FreehandTool': 'mesh',
 'RotatedRectangleTool': 'mesh',
 'ThreePointArcTool': 'mesh',
 'CenterArcTool': 'mesh',
 'PieTool': 'mesh',
 'FlipTool': 'mesh',
 'PlaceGroupTool': 'mesh',
 'PasteTool': 'mesh',
 'FilletTool': 'mesh',
 'ProtractorTool': 'mesh',
 'GeoPathTool': 'mesh',
 'ChangeAxesTool': 'mesh',
 'ScaleManagerTool': 'mesh',
 'PositionCameraTool': 'mesh',
 'WalkTool': 'mesh',
 'LookAroundTool': 'mesh',
 'FirstPersonTool': 'mesh',
 'OuterShellTool': 'mesh',
 'UnionTool': 'mesh',
 'SubtractTool': 'mesh',
 'TrimTool': 'mesh',
 'IntersectTool': 'mesh',
 'SplitTool': 'mesh',
 'NorthTool': 'mesh'}


def move_command(groups, delta):
    commands = [MoveGroupCommand(group, delta) for group in groups]
    return commands[0] if len(commands) == 1 else CompoundCommand(commands)


from views.wgpu_mesh_tools import MeshToolAccess


class NativeTools(MeshToolAccess):
    def __init__(self, view):
        self.view = view
        self.tool = None
        self.mode = 'select'
        self.start_point = None
        self.hover_point = None
        self.ids = np.empty(0, np.int64)
        self.original = None
        self.groups = []
        self.value_buffer = ""

    @property
    def scene(self):
        return getattr(self.view, 'scene', None) or getattr(self.view.source_viewport, 'scene', None)

    @property
    def last_snap(self):
        return getattr(self.view, 'last_snap', None)

    @property
    def active_tool(self):
        return self.tool

    def sync(self):
        source = self.view.source_viewport
        tool = getattr(source, 'active_tool', None)
        if tool is self.tool:
            if self.mode in ('tape', 'move') and self.start_point is not None and getattr(tool, 'start_point', None) is None:
                self.cancel()
            return self.mode
        if (tool is None and self.tool is not None
                and getattr(source, 'suspended_tool', None) is self.tool):
            return self._suspend()
        parked = self.__dict__.pop('_parked', None)
        if parked is not None:
            if tool is parked['tool']:
                return self._resume(parked)
            # Another tool after the camera mode: the parked one is
            # cancelled here as any switch cancels it.
            self.__dict__.update(parked)
        preview = getattr(self.view, '_texture_preview', None)
        if preview is not None:
            preview.restore()
        if type(self.tool).__name__ == 'TexturePositionTool':
            self.tool._leave(self)
        self.cancel()
        self.tool = tool
        self.mode = TOOL_DISPATCH.get(type(tool).__name__, 'select' if tool is None else 'inert')
        if self.mode == 'move':
            # Share MoveTool's copy toggle, numeric values and array window.
            self.mode = 'mesh'
        if self.mode == 'tape':
            self.tool.on_cancel(self)
        if hasattr(tool, '_viewport'):
            tool._viewport = self
        self._notified_revision = getattr(getattr(source, 'history', None), 'revision', 0)
        if hasattr(self.view, 'setCursor'):
            self._apply_tool_cursor()
        if self.mode == 'inert':
            source.flash_status('not yet available on the Metal renderer')
        return self.mode

    #: What a camera mode parks of the tool it suspends (GL's
    #: ``DocumentInteraction.suspended_tool``) and gives back when that tool resumes:
    #: the first point, a Move's preview rows, the typed value.
    _PARKED = ('tool', 'mode', 'start_point', 'original', 'ids', 'value_buffer')

    def _suspend(self):
        """Orbit / Pan / Zoom took the view from the tool: park it as it is.
        No ``on_cancel``, and a Move's rows stay where the preview put them
        (the preview mesh too), so the model shows what was being done
        while the camera turns."""
        self._parked = {name: getattr(self, name) for name in self._PARKED}
        self.tool = None
        self.mode = 'select'
        self.start_point = None
        self.original = None
        self._set_value_buffer("")
        self._mesh_hover = None
        self.view.overlay_primitives = []
        if hasattr(self.view, 'setCursor'):
            self._apply_tool_cursor()
        return self.mode

    def _resume(self, parked):
        self.__dict__.update(parked)
        self._set_value_buffer(self.value_buffer)
        if hasattr(self.tool, '_viewport'):
            self.tool._viewport = self
        if hasattr(self.view, 'setCursor'):
            self._apply_tool_cursor()
        return self.mode

    def claim(self):
        """The pointer came into this view (one of several, sharing the
        window's tool): the tool's viewport is this one again, so what it
        draws and picks is here."""
        if self.tool is not None and hasattr(self.tool, '_viewport'):
            self.tool._viewport = self

    def context(self, event):
        view = self.view
        point = (view.last_snap.point if getattr(view, 'last_snap', None) is not None
                 else view._world_under_cursor(event.position().x(), event.position().y()))
        if point is None:
            return None
        if type(self.tool).__name__ == 'FilletTool':
            self._hover_edge = self.pick_visible_edge(event.position().x(), event.position().y())
        if self.tool is not None:
            self.tool.snap_kind = getattr(view.last_snap, "kind", None)
        return ToolContext(self, point, event.position(), event.modifiers(), view.last_snap)

    def click(self, event):
        mode = self.sync()
        ctx = self.context(event)
        if ctx is None:
            return
        if mode == 'mesh':
            if (type(self.tool).__name__ == 'MoveTool'
                    and self.tool.start_point is None
                    and ctx.modifiers & Qt.ControlModifier and not self.tool._copy):
                self.tool._toggle_copy(self, say=False)
            had_start = getattr(self.tool, 'start_point', None) is not None
            plane = None
            if (not had_start and hasattr(self.tool, 'work_plane')
                    and self.tool.work_plane is None
                    and callable(getattr(self.view, 'pick_row', None))):
                # GL's _dispatch_tool_click: the first click on a face
                # captures its plane for the rest of the operation, so a
                # rectangle started on a sloped face and dragged past its
                # border stays on that face's plane and splits it. Asked
                # at the click only.
                face, placement = self.pick_face_placement(
                    event.position().x(), event.position().y())
                if face is not None:
                    from core.snap import face_plane_world
                    plane = face_plane_world(face, getattr(placement, 'xform', None))
            self.tool.on_hover(ctx)
            self.tool.on_click(ctx)
            self.start_point = getattr(self.tool, 'start_point', None)
            if (plane is not None and self.start_point is not None
                    and hasattr(self.tool, 'work_plane')
                    and self.tool.work_plane is None):
                self.tool.work_plane = plane
            self.update()
            self.view.overlay_primitives = []
            self.mesh_feedback()
        elif mode == 'tape':
            self.tool.on_click(ctx)
            self.start_point = getattr(self.tool, 'start_point', None)
            self.measurements()
        self.view.update()

    def hover(self, event):
        mode = self.sync()
        ctx = self.context(event)
        if ctx is None:
            return
        self.hover_point = ctx.world
        if mode == 'mesh':
            self.tool.on_hover(ctx)
            self.start_point = getattr(self.tool, 'start_point', None)
            self.mesh_feedback()
            return
        if mode == 'tape':
            # The band, the guide preview and the length label are painted
            # each frame by draw_annotations, with GL's painters.
            self.tool.on_hover(ctx)
            self.measurements()

    def key(self, event):
        if self.mode == 'mesh':
            from views.viewport import DocumentInteraction
            self._value_buffer = self.value_buffer
            if DocumentInteraction._handle_value_key(self, event):
                self.value_buffer = self._value_buffer
                self.start_point = getattr(self.tool, 'start_point', None)
                self.view.overlay_primitives = []
                self.update()
                self.mesh_feedback()
                return True
            return self.tool.on_key(self, event.key(), event.modifiers())
        return False

    def _current_token_tail(self):
        from views.viewport import DocumentInteraction
        return DocumentInteraction._current_token_tail(self)

    def _array_pending(self):
        return getattr(self.tool, 'array_pending', lambda vp: False)(self)

    def _release_axis_lock_after_operation(self, committed=False):
        release = getattr(self.view.source_viewport, '_release_axis_lock_after_operation', None)
        if release is not None:
            release(committed=committed)

    def _set_value_buffer(self, value):
        self._value_buffer = self.value_buffer = value
        signal = getattr(self.view.source_viewport, "valueBufferChanged", None)
        if signal is not None:
            signal.emit(value)

    def _parse_value_buffer(self, *args, **kwargs):
        from views.viewport import DocumentInteraction
        return DocumentInteraction._parse_value_buffer(*args, **kwargs)

    def write_rows(self, rows):
        if self.view.renderer is not None:
            self.view.renderer.update_instances(self.ids.tolist(), rows.view(np.uint8).reshape(-1))
        billboards = getattr(self.view, '_billboards', None)
        if billboards is not None:
            billboards.restore()
        self.view.packed.instances[self.ids] = rows
        from views.wgpu_parity import hidden_geometry
        hidden_geometry(self.view)
        if billboards is not None and len(billboards.ids):
            from views.wgpu_billboards import Billboards
            self.view._billboards = Billboards(self.view.packed)
            self.view._billboards.update(self.view.camera, self.view.renderer, self.view.inference_index)

    def restore(self):
        if self.original is not None:
            self.write_rows(self.original)

    def cancel(self):
        renderer = getattr(self.view, "renderer", None)
        if renderer is not None and hasattr(renderer, "set_preview_mesh"):
            empty = np.empty(0, np.uint8)
            renderer.set_preview_mesh(empty, empty)
            # mesh_feedback skips an unchanged preview: this one is gone.
            self.view._preview_key = None
        self.restore()
        if self.mode == 'mesh' and self.tool is not None:
            self.tool.on_cancel(self)
            self._mesh_hover = None
            self.update()
        if isinstance(self.tool, TapeMeasureTool):
            self.tool.on_cancel(self)
        self.original = None
        self.start_point = None
        if self.mode == "move" and self.tool is not None:
            self.tool.start_point = None
        if self.value_buffer:
            self._set_value_buffer("")
        self.view.overlay_primitives = []

    def measurements(self):
        source = self.view.source_viewport
        measured = getattr(self.tool, '_measured', None)
        if self.mode == 'tape' and self.start_point is None and measured is not None:
            from core.units import fmt_len_fine
            source.measurementChanged.emit(fmt_len_fine(measured))
        else:
            source.measurementChanged.emit(source._measurement_text())

    def flash_status(self, *args):
        self.view.source_viewport.flash_status(*args)

    def mesh_feedback(self):
        if type(self.tool).__name__ == 'TexturePositionTool':
            self.measurements()
            return
        super().mesh_feedback()

    def update(self):
        if self.mode == 'mesh':
            preview = getattr(self.view, '_texture_preview', None)
            if preview is not None:
                if not getattr(self.tool, 'active', False):
                    preview.restore()
            self.view.set_scene(self.scene)
            if type(self.tool).__name__ == 'TexturePositionTool':
                if preview is None:
                    from tools.texture_position_metal import TexturePreview
                    preview = self.view._texture_preview = TexturePreview(self.view)
                preview.update(self.tool)
            revision = self.history.revision
            if revision != getattr(self, '_notified_revision', revision):
                notify = getattr(self.view.source_viewport, 'notify_scene_changed', None)
                if notify is not None:
                    from core.coordinate_frame import model_frame
                    with model_frame():
                        notify()
            self._notified_revision = revision
        self.view.update()

    def pick_edge_any(self, x, y):
        from views.wgpu_inference import reference_edge
        if getattr(self.view, 'inference_index', None) is None:
            return None
        return reference_edge(self.view.inference_index, self.view, x, y,
                              radius_px=self.pick_threshold_px)

    def pick_guide(self, x, y):
        # These shared projection-only pickers never read GL caches.
        from views.viewport import DocumentInteraction
        return DocumentInteraction.pick_guide(self, x, y) if self.scene is not None else None

    def pick_axis(self, x, y):
        from views.viewport import DocumentInteraction
        return DocumentInteraction.pick_axis(self, x, y) if self.scene is not None else None

    def width(self):
        return self.view.width()

    def height(self):
        return self.view.height()

    def _apply_tool_cursor(self):
        source = self.view.source_viewport
        # A nav mode (Orbit, Pan, Zoom) has no active tool: asking for the
        # tool cursor then unset the hand the mode had just put on.
        if getattr(source, 'nav_mode', None):
            apply = getattr(source, '_apply_nav_cursor', None)
        else:
            apply = getattr(source, '_apply_tool_cursor', None)
        if apply is not None:
            apply()
            if hasattr(self.view, 'setCursor'):
                self.view.setCursor(source.cursor())

    def begin_group_edit(self, group):
        # The document controller owns undo, links, locks and edit sharing.
        self.view.source_viewport.begin_group_edit(group)

    def end_one_group_edit(self):
        self.view.source_viewport.end_one_group_edit()

    def set_extension_pick(self, hit):
        self.view.source_viewport.set_extension_pick(hit)

    def clear_extension_pick(self, notify=True):
        self.view.source_viewport.clear_extension_pick(notify=notify)

    def edit_group_box_contains_pixel(self, x, y):
        return self.view.edit_group_box_contains_pixel(x, y)

    def __getattr__(self, name):
        if name == "camera" and getattr(self.view, "camera", None) is not None:
            return self.view.camera
        return getattr(self.view.source_viewport, name)
