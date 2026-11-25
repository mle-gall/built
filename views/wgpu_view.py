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

    def render_image(self, width_px, height_px=None, overlays=False,
                     sharp_shadows=False, *, transparent=False, annotations_only=False,
                     page_key=None, scene=None, style=None, camera=None, rows=None,
                     edge_px=None, profile_px=None):
        """R3c-2a: render sheet targets using this view's resident scene.

        A sheet frame (views/wgpu_sheet_frames.py) passes what it shows
        instead of the live state: ``scene`` (a read-only facade), ``style``,
        ``camera(origin)`` and ``rows()`` (resident rows with the frame's
        visibility), and ``page_key``, the page texture the picture goes to:
        then nothing is read back and None is returned."""
        frame = self.render_pixels(width_px, height_px, overlays=overlays, annotations_only=annotations_only,
                                   sharp_shadows=sharp_shadows, transparent=transparent, page_key=page_key,
                                   scene=scene, style=style, camera=camera, rows=rows,
                                   edge_px=edge_px, profile_px=profile_px)
        if frame is None:
            return None
        w, h, pixels = frame
        from ingetrazo_render import SurveyRaster
        return SurveyRaster(w, h, bytes(pixels))

    def render_pixels(self, width_px, height_px=None, *, overlays=False, annotations_only=False, **frame_options):
        """RGBA of the requested frame, including any document marks for that camera."""
        frame = self._render_resident(width_px, height_px, **frame_options)
        if frame is None:
            return None
        w, h, pixels, camera = frame
        if overlays:
            from views.wgpu_annotations import composite_pixels
            pixels = composite_pixels(pixels, w, h, self.source_viewport.scene, camera,
                                      logical_size=(self.width(), self.height()), annotations_only=annotations_only)
        return w, h, pixels

    def _render_resident(self, width_px, height_px, sharp_shadows=False, transparent=False,
                         page_key=None, scene=None, style=None, camera=None, rows=None,
                         edge_px=None, profile_px=None):
        from copy import copy
        from views.wgpu_style import style_payload

        source = self.source_viewport
        self.tools.sync()
        if height_px is None:
            height_px = round(width_px * self.height() / max(1, self.width()))
        width_px, height_px = int(width_px), int(height_px)
        if width_px <= 0 or height_px <= 0:
            raise ValueError("Image dimensions must be positive")
        if hasattr(self._shared, '_content_token'):
            self._shared.set_scene(source.scene)
        if self.packed is None:
            self.packed = pack_scene(source.scene, getattr(source, 'view_tint', None),
                                     focus=source.camera.target.toTuple())
        if self.packed.model_translations is not None:
            self._sync_scene_camera()
        if self.renderer is None or self._scene_dirty:
            self._upload(force=True)
        translated = self.packed.model_translations is not None
        if camera is not None:
            camera = camera(self.packed.origin if translated else None)
        else:
            camera = copy(self.camera if translated else source.camera)
            camera.aspect = width_px / height_px
        view, projection = camera.view_matrix(), camera.projection_matrix()
        shown = source.scene if scene is None else scene
        look = source._effective_style() if style is None else style
        payload = style_payload(shown, look,
                                horizon=self._horizon(projection * view, camera),
                                edit_rest_fade=getattr(source, 'edit_rest_fade', .75),
                                soft_shadows=getattr(source, '_soft_shadows', 0),
                                camera=camera)
        from views.wgpu_references import sync as sync_references
        payload['transparent_background'] = transparent
        payload['edge_width'] = float(getattr(source, '_export_edge_px', 1) if edge_px is None else edge_px)
        payload['profile_width'] = float(getattr(source, '_export_profile_px', 1)
                                         if profile_px is None else profile_px)
        from views.wgpu_sections import capture_rows, geometry
        if rows is not None:
            rows = rows()
        else:
            rows = capture_rows(self.packed, source.scene, getattr(source, 'view_tint', None))
        from views.wgpu_edit_context import rest_rows
        mode = getattr(source, 'edit_rest_mode', 'fade')
        rows['info'][:, 2] &= np.uint32(~192 & 0xffffffff)
        if mode in ('fade', 'hide') and shown.edit_group is not None:
            rows['info'][rest_rows(self.packed.groups, shown.edit_group), 2] |= (
                64 | (128 if mode == 'hide' else 0))
        from views.wgpu_billboards import Billboards
        from views.wgpu_axes import axes_state
        capture = copy(self.packed)
        capture.instances = rows
        billboards = getattr(self, '_billboards', None)
        if billboards is not None:
            rows['world'][billboards.ids] = billboards.base
        if billboards is not None:
            Billboards(capture, billboards.ids).update(camera)
        # Capture rows may change tint/visibility only. Install the camera-facing
        # matrices through the resident API, restoring them even on failure.
        billboard_ids = [] if billboards is None else billboards.ids.tolist()
        resident_world = self.packed.instances['world'][billboard_ids].copy()
        old_axes = getattr(self, '_axes_state', None) or axes_state(source.scene, self.camera, source, self.packed.origin)
        previous = getattr(self, '_section_uploaded', np.empty(0, np.uint8))
        data = geometry(self, shown, look, camera, height_px, rows)
        try:
            sync_references(self, payload, scene=scene)
            if billboard_ids:
                self.renderer.set_instance_world(billboard_ids, rows['world'][billboard_ids].reshape(-1))
            self.renderer.set_axes(*axes_state(shown, camera, source, self.packed.origin))
            self.renderer.set_section_geometry(data)
            args = (width_px, height_px, tuple(view.data()), tuple(projection.data()), payload,
                    rows.view(np.uint8).reshape(-1) if len(rows) else None)
            options = {**({'sharp_shadows': True} if sharp_shadows and payload['shadows'] else {}),
                       # A far clip set by hand; otherwise reverse-Z reaches to
                       # infinity (stand-in renderers need not know the flag).
                       **({'far_clip': True} if getattr(camera, 'clip_far', None) is not None else {})}
            if page_key is not None:
                self.renderer.page_frame_render(page_key, *args, **options)
                return None
            pixels = bytes(self.renderer.render_image(*args, **options))
        finally:
            if billboard_ids:
                self.renderer.set_instance_world(billboard_ids, resident_world.reshape(-1))
            self.renderer.set_axes(*old_axes)
            self.renderer.set_section_geometry(previous)
            if scene is not None:
                sync_references(self, getattr(self, "_frame_style", None) or {})
        return width_px, height_px, pixels, camera

    def paintEvent(self, event):
        # During a splitter/window drag keep the last Metal frame: the
        # surface is configured once the resize settles (_finish_resize).
        if self._resize_timer.isActive():
            return
        self.draw_frame()

    def resolve(self, row):
        """The resident row identifies both the placement and shared prototype."""
        if self.packed is None or row is None or not 0 <= row < len(self.packed.uids):
            return None
        if self.packed.instances['info'][row, 2] & 129:
            return None
        return self.packed.groups[row], int(self.packed.instances['info'][row, 0])

    def _prepare_pick_frame(self):
        if getattr(self, '_drawing_frame', False):
            return  # overlay occlusion reads IDs during this very presentation
        if getattr(self.packed, 'model_translations', None) is not None:
            self._sync_scene_camera()
        # A click immediately after release must use the new pixel mapping.
        if self._resize_timer.isActive():
            self._resize_timer.stop()
            self._finish_resize()
        if self._scene_dirty and self.isVisible():
            self._upload()
        timer = getattr(self, '_refresh_timer', None)
        if timer is not None and timer.isActive():
            self.set_scene(self.scene)
        self.sync_edit_context()
        if getattr(self, '_pick_frame_dirty', False) and hasattr(self.renderer, 'render'):
            self._drawing_frame = True
            try:
                self._present_frame(pick_only=True)
            finally:
                self._drawing_frame = False

    def pick_row(self, x, y):
        if self.renderer is None or x < 0 or y < 0:
            return None
        self._prepare_pick_frame()
        scale = self.devicePixelRatioF()
        row = self.renderer.pick(int(x*scale), int(y*scale))
        return row if self.resolve(row) is not None else None

    @_tool_frame
    def hover(self, x, y, modifiers=Qt.NoModifier):
        from views.wgpu_inference import snap
        from views.wgpu_overlay import snap_glyph, Primitive
        start = time.perf_counter()
        self.tools.sync()
        # Select's priority walk already queries the row for its group pick.
        self._cue_cursor = (x, y)
        self.hovered_row = None
        selecting = self.tools.mode == 'select' and not getattr(self.source_viewport, 'nav_mode', None)
        if not selecting or self.source_viewport is None:
            self.hovered_row = self.pick_row(x, y)
        self.hover_rows = ()  # GL paints hovered edges and annotations only.
        self.last_snap = None
        self.overlay_primitives = []
        source = self.source_viewport
        if selecting:
            if source is not None:
                self.tools.select_tool().hover_at(self.tools, x, y, modifiers)
            else:
                self.tools.set_hover(None)
            self.hover_ms = (time.perf_counter()-start)*1000
            self.update()
            return None
        work_point = None
        mesh_mode = self.tools.sync() == 'mesh'
        if mesh_mode:
            work_point = self.tools.work_point(x, y, picked_row=self.hovered_row)
        tool = getattr(source, 'active_tool', None)
        if tool is not None and not tool.uses_snap and self.tools.mode == 'mesh':
            from types import SimpleNamespace
            point = work_point
            if point is not None:
                self.last_snap = SimpleNamespace(point=point, kind=None, context=None)
            self.update()
            return self.last_snap
        inference = getattr(source, 'inference', None)
        project = (lambda start, direction:
                   self._project_to_lock_line(start, direction, x, y))
        if inference is not None and hasattr(inference, 'snap_options'):
            # The same tool/state keywords as GL's InferenceEngine.snap: the
            # short list this used to build left out the acquired edge and
            # face, the Shift lock, the plane rules and the rectangle's
            # diagonal (see InferenceEngine.snap_options).
            options = inference.snap_options(tool, modifiers)
            options['start_point'] = self.tools.start_point
            options['project_onto_line'] = project
        else:
            options = dict(start_point=self.tools.start_point,
                           chain_first_point=getattr(tool, 'chain_first_point', None),
                           acquired_point=getattr(inference, '_acquired_point', None),
                           acquired_points=getattr(inference, '_encouraged', None),
                           axis_lock=getattr(inference, 'axis_lock', None),
                           shift_held=bool(modifiers & Qt.ShiftModifier),
                           reference_edge=getattr(inference, 'reference_edge', None),
                           reference_mode=getattr(inference, 'reference_mode', None),
                           magnetic_axis_deg=getattr(tool, 'magnetic_axis_deg', None),
                           screen_axis_px=getattr(tool, 'screen_axis_px', None),
                           linear_mode=getattr(source, 'linear_inference_mode', 'all'),
                           project_onto_line=project)
        if self.inference_index is not None:
            if hasattr(inference, 'note_drawing_refs'):
                options.update(inference=inference, modifiers=modifiers)
            if mesh_mode:
                options['work_point'] = work_point
            self.last_snap = snap(self.inference_index, self.hovered_row, self, x, y, **options)
            if hasattr(inference, 'finish_snap') and self.last_snap is not None:
                # The axis-source cue picks through this view's tools (the
                # snap index answers its occlusion); the extension
                # providers get the document's viewport, as on GL.
                self.last_snap = inference.finish_snap(self.last_snap, x, y, tool,
                                                       picks=self.tools)
                if self.last_snap.kind == 'endpoint':
                    # GL's mid-segment corner pick: the corner under the
                    # cursor is a soft, instant reference (through point,
                    # from point) while a segment is drawn.
                    inference.note_drawing_refs(tool, corner=self.last_snap.point)
        elif self.tools.sync() in ('tape', 'move', 'mesh'):
            from core.snap import compute_snap
            from types import SimpleNamespace
            origin, direction = self._pixel_to_ray(x, y)
            plane = self.tools.start_point if self.tools.start_point is not None else self.camera.target
            normal = self.camera.forward()
            denominator = QVector3D.dotProduct(direction, normal)
            if abs(denominator) > 1e-9:
                point = origin + direction * (QVector3D.dotProduct(plane-origin, normal)/denominator)
                if self.tools.mode == 'mesh':
                    point = work_point or point
                self.last_snap = compute_snap(point, (x, y), SimpleNamespace(edges=[]),
                                              self._world_to_pixel, 12., **options)
        if inference is not None and self.last_snap is not None:
            source._note_snap(self.last_snap)
            point = (self.last_snap.point if self.last_snap.kind in source._ENCOURAGING_KINDS else None)
            inference._dwell_on(point)
        self.sync_selection()
        self.hover_ms = (time.perf_counter()-start)*1000
        from views.document_interaction import _PERF, _plog
        if _PERF:
            _plog("hover", self.hover_ms, extra="renderer=wgpu id-local", floor=0.)
        self.update()
        return self.last_snap

    def sync_edit_context(self):
        if self.renderer is None or self.packed is None:
            return
        mode = getattr(self.source_viewport, 'edit_rest_mode', 'fade')
        signature = (id(self.packed), id(self.scene.edit_group), mode)
        if signature == getattr(self, '_rest_signature', None):
            return
        from views.wgpu_edit_context import rest_rows
        rows = rest_rows(self.packed.groups, self.scene.edit_group)
        before = time.perf_counter()
        # Always: a reset signature (an incremental update clears it) does
        # not mean the renderer holds no rest bits. Skipping the call when
        # there were no rows left the whole model hidden after leaving a
        # group with "hide" (and faded with "fade"). The native call is one
        # pass over the flags and writes nothing when they are unchanged.
        self.renderer.set_rest(rows, mode)
        self.rest_write_ms = (time.perf_counter() - before) * 1000
        flags = self.packed.instances['info'][:, 2]
        flags &= np.uint32(~(64 | 128) & 0xffffffff)
        if mode in ('fade', 'hide'):
            flags[rows] |= np.uint32(64 | (128 if mode == 'hide' else 0))
        self._pick_frame_dirty = True
        self._rest_signature = signature
        from views.wgpu_parity import hidden_geometry
        hidden_geometry(self)

    def _group_obb(self, group):
        from views.document_interaction import DocumentInteraction
        shared = self._shared.__dict__
        boxes = shared.setdefault('_container_boxes', {})
        if group not in boxes:
            boxes[group] = DocumentInteraction._compute_obb(group)
            # Which rows it measured: an edit elsewhere keeps the box.
            if getattr(self, 'uid_rows', None) is not None:
                shared.setdefault('_container_box_rows', {})[group] = np.asarray(
                    self._rows_for_group(group), np.int64)
        return boxes[group]

    def _keep_container_boxes(self, tracker, ids, boxes, box_rows):
        """Keep the container boxes an incremental edit did not touch.

        Clearing them all on every edit threw away the big containers'
        boxes measured after the open (0.9 s each on the ride, measured
        again at the next double-click). A box stays when none of its rows
        moved or changed prototype; rows added (or a diff that walked every
        placement) may land inside any container, so then none stays."""
        if not boxes or not tracker.last_local or tracker.added:
            return
        touched = [np.asarray(ids, np.int64), np.asarray(tracker.removed, np.int64)]
        if tracker.changed:
            touched.extend(tracker.changed_code_rows.values())
        touched = np.concatenate(touched)
        for group, box in boxes.items():
            rows = box_rows.get(group)
            if rows is not None and not np.isin(rows, touched).any():
                self._container_boxes[group] = box
                self._container_box_rows[group] = rows

    def _edit_group_box_corners(self):
        from views.document_interaction import DocumentInteraction
        return DocumentInteraction._edit_group_box_corners(self)

    def edit_group_box_contains_pixel(self, x, y):
        from views.document_interaction import DocumentInteraction
        return DocumentInteraction.edit_group_box_contains_pixel(self, x, y)

    def _leave_edit_on_empty_click(self, x, y, modifiers):
        if (self.scene.edit_group is not None and not self.scene.selection
                and not modifiers & (Qt.ControlModifier | Qt.MetaModifier | Qt.ShiftModifier)
                and not self.edit_group_box_contains_pixel(x, y)):
            self.source_viewport.end_one_group_edit()
            self.update()
            return True
        return False

    def _selection_box_primitives(self):
        from views.wgpu_edit_context import selection_box_primitives
        return selection_box_primitives(self)

    def _selection_box_vertices(self):
        from views.wgpu_edit_context import selection_box_vertices
        return selection_box_vertices(self)

    def _edit_box_primitives(self):
        from views.wgpu_edit_context import box_primitives
        return box_primitives(self)

    def sync_selection(self):
        if self.renderer is None:
            return
        from core.mesh import Face, Edge, element_identity
        from core.group import Group
        from views.wgpu_parity import face_marks
        selected = self.scene.selection
        hover = tuple(getattr(self, 'hover_rows', ()) or ())
        faces = [f for f in selected if isinstance(f, Face)]
        hovered = getattr(self.tools, '_mesh_hover', getattr(self.source_viewport, '_hover_entity', None))
        # A tool's face under the cursor wears the selection dots (GL's
        # _paint_hover); Select never hovers a face, and a selected face
        # already shows them.
        hover_face = (hovered if isinstance(hovered, Face) and self.tools.mode != 'select'
                      and hovered not in selected else None)
        placement = getattr(self.tools, '_mesh_hover_placement', None) if hover_face else None
        # Outside the mesh tools (whose feedback owns the preview), the
        # native preview edges carry the selected and hovered loose edges
        # and the selected groups' boxes, all depth-tested as in GL.
        edges_here = self.tools.mode != 'mesh' and hasattr(self.renderer, 'set_preview_mesh')
        box_key, boxes = self._selection_box_vertices() if edges_here else (None, ())
        signature = (frozenset(map(element_identity, selected)), hover, element_identity(hovered), id(self.packed),
                     id(placement), tuple(self.camera.eye().toTuple()) if faces or hover_face else (),
                     self.tools.mode, box_key)
        if signature != self._selection_signature:
            indices = set()
            for root in selected:
                if isinstance(root, Group):
                    indices.update(self._rows_for_group(root))
            # GL never tints a face-me figure (its mesh quad is not what is
            # on screen): its cue is the outline of the quad as drawn.
            groups = self.packed.groups
            indices = [r for r in indices if not getattr(groups[r], 'billboard', False)]
            self.renderer.set_highlight(indices, list(hover))
            if hasattr(self.renderer, 'set_face_marks'):
                marks = face_marks(self, faces)
                if hover_face is not None:
                    from views.wgpu_parity import hover_face_marks
                    marks = np.concatenate((marks, hover_face_marks(self, hover_face, placement)))
                self.renderer.set_face_marks(marks)
            if edges_here:
                from ingetrazo_core import VERTEX
                from views.wgpu_parity import raw
                edges = [(e, (.95, .45, .16)) for e in selected if isinstance(e, Edge)]
                if isinstance(hovered, Edge) and hovered not in selected:
                    edges.append((hovered, (.30, .55, .95)))
                vertices = [(p.toTuple(), (0,0,1), (*np.asarray(np.array(c)*255, np.uint8),255))
                            for e,c in edges for p in (e.a,e.b)]
                lines = np.concatenate((np.array(vertices, VERTEX), boxes))
                self.renderer.set_preview_mesh(np.empty(0,np.uint8), raw(lines))
            self._selection_signature = signature

    def _draw_selection_marks(self, painter):
        """Loose edge feedback; faces/groups are depth-tested native passes."""
        if self.tools.mode != 'mesh' and hasattr(self.renderer, 'set_preview_mesh'):
            return
        from core.mesh import Edge
        from ingetrazo_core import ScreenPoint as QPointF
        from ingetrazo_render import PaintPen as QPen
        entities = [(e, False) for e in self.scene.selection if isinstance(e, Edge)]
        hovered = getattr(self.tools, '_mesh_hover', getattr(self.source_viewport, '_hover_entity', None))
        if isinstance(hovered, Edge) and hovered not in self.scene.selection:
            entities.append((hovered, True))
        painter.save()
        try:
            for edge, hover in entities:
                painter.setPen(QPen(QColor.fromRgbF(*((.30, .55, .95) if hover else (.95, .45, .16))), 1.5))
                a, b = self._world_to_pixel(edge.a), self._world_to_pixel(edge.b)
                if a is not None and b is not None:
                    painter.drawLine(QPointF(*a), QPointF(*b))
        finally:
            painter.restore()

    def _mirror_cursor(self):
        """Wear the cursor the tools set on the (hidden) GL viewport: the
        tool icons, Orbit's and Pan's hands, a tool's own changes while it
        works. Copying it once per tool change missed every other change
        (Orbit chosen over Select kept the arrow, Maxence 2026-11-09)."""
        source = self.source_viewport
        if source is None:
            return
        if not source.testAttribute(Qt.WA_SetCursor):
            key = None
        else:
            cur = source.cursor()
            shape = cur.shape() if hasattr(cur, "shape") else cur
            key = (shape, cur.pixmap().cacheKey()) if shape == Qt.BitmapCursor and hasattr(cur, "pixmap") else shape
        if key == getattr(self, '_cursor_key', object()):
            return
        self._cursor_key = key
        if key is None:
            self.unsetCursor()
        else:
            self.setCursor(source.cursor())

    def _parents(self):
        """``{id(child): parent}`` over the scene's groups, per pack."""
        token = getattr(self, '_content_token', None)
        cached = getattr(self, '_parent_map', None)
        if cached is not None and cached[0] == token:
            return cached[1]
        parents = {}
        todo = list(self.scene.groups)
        while todo:
            g = todo.pop()
            for child in getattr(g, 'children', None) or ():
                parents[id(child)] = g
                todo.append(child)
        self._parent_map = (token, parents)
        return parents

    def context_placement(self, group):
        """The placement a click selects for a hit on ``group``: the
        top-level object at the root, the child directly under the group
        being edited inside one, ``None`` for a hit outside the context —
        what the GL viewport's ``pick_group`` returns."""
        if group is None:
            return None
        parents = self._parents()
        chain = [group]
        while (parent := parents.get(id(chain[-1]))) is not None:
            chain.append(parent)
        ctx = self.scene.edit_group
        if ctx is None:
            return chain[-1]
        for i, g in enumerate(chain):
            if g is ctx:
                return chain[i-1] if i > 0 else None
        return None

    def _rows_for_group(self, group):
        cache = self._shared.__dict__.setdefault('_selection_rows', {})
        if group not in cache:
            rows = []
            pending = [group]
            while pending:
                child = pending.pop()
                rows.extend(self.uid_rows.get(self.packed.row_key(child), ()))
                pending.extend(child.children or ())
            cache[group] = rows
        return cache[group]

    def select_rows(self, rows, modifiers=Qt.NoModifier):
        start = time.perf_counter()
        groups = {self.context_placement(self.packed.groups[i])
                  for i in rows if self.resolve(i) is not None}
        groups = [g for g in groups if g is not None and self.scene.entity_selectable(g)]
        mode = selection_mode(modifiers)
        self.scene.select(groups, mode=mode)
        self.sync_selection()
        if self.source_viewport is not None:
            self.source_viewport.notify_scene_changed()
        self.update()
        self.selection_ms = (time.perf_counter()-start)*1000

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
