"""Document-tool facade for native mesh editing; no GL picking or drawing."""
import numpy as np
from PySide6.QtGui import QVector3D, QMatrix4x4
from core.group import iter_placements

_VIEWPORT = None


def _viewport_class():
    """``views.viewport.Viewport``, imported once (no import cycle at load)."""
    global _VIEWPORT
    if _VIEWPORT is None:
        from views.viewport import Viewport
        _VIEWPORT = Viewport
    return _VIEWPORT


class MeshToolAccess:
    mesh_outline_preview = True
    # Do not borrow the GL viewport's cached loose-edge screen projection.
    _ledge_screen = None
    _group_chunk = None  # SelectTool uses its model-space box traversal.
    _OCCLUSION_STEP_PX = 12.
    _OCCLUSION_MAX_SAMPLES = 128
    _OCCLUSION_SPLIT = 8
    _OCCLUSION_ROUNDS = 2

    @staticmethod
    def _text_block_x(*args):
        from views.viewport import Viewport
        return Viewport._text_block_x(*args)

    @staticmethod
    def _format_dim_value(*args):
        from views.viewport import Viewport
        return Viewport._format_dim_value(*args)


    def inference_meshes(self, x, y):
        picked = self._picked_mesh(x, y)
        # Other placements are queried through their world-space face plane;
        # their local vertices must not be projected as world coordinates.
        return [self.scene.mesh]

    def work_point(self, x, y, *, picked_row=...):
        from core import axes
        tool = self.tool
        own = tool.drag_plane(self)
        plane = own or getattr(tool, 'work_plane', None)
        if own is None and plane is not None:
            # GL's _current_work_plane (#545): a PLANAR shape re-plans onto
            # the face the cursor is over, capture or not, and takes its
            # capture with it — or the preview stayed on the face it started
            # on while the point and the badge went to the other one.
            face_plane = self._face_replan(tool, x, y, plane, picked_row)
            if face_plane is not None:
                tool.work_plane = plane = face_plane
        if plane is None:
            lock = getattr(tool, 'locked_work_plane', None)
            anchor = self.start_point if self.start_point is not None else axes.origin()
            plane = lock(anchor) if lock else None
        if plane is None:
            plane = self._face_plane_under(x, y, picked_row)
        if plane is None:
            anchor = self.start_point if self.start_point is not None else axes.origin()
            normal = axes.axis('z')
            if abs(QVector3D.dotProduct(self.camera.forward(), normal)) < .2:
                normal = self.camera.forward()
            plane = anchor, normal
        self._last_work_plane = plane
        return self._ray_hit_plane((x, y), plane)

    def _face_replan(self, tool, x, y, captured, picked_row):
        """``Viewport._planar_face_plane`` with the face under the cursor
        from this view's picks (:meth:`_face_plane_under`). Only a planar
        shape pays for the ray."""
        from tools.base import PlaneLock
        if not isinstance(tool, PlaneLock):
            return None
        face_plane = self._face_plane_under(x, y, picked_row)
        if face_plane is None:
            return None
        return _viewport_class()._planar_face_plane(
            self, tool, (x, y), captured, face_plane=face_plane)

    def _face_plane_under(self, x, y, picked_row=...):
        """World ``(point, unit normal)`` of the face under the cursor, or
        ``None``: the placement under the cursor (the ID pass) and its
        triangle on the cursor ray, from the snap index — one ray over one
        placement, where the mesh walk (:meth:`pick_face_placement`) costs
        the mesh's size. The walk still answers before the index is built
        and while a tool moves faces (a moving face is no plane to land on,
        and only the walk knows which triangles are its)."""
        index = getattr(self.view, 'inference_index', None)
        moving = getattr(self.tool, 'moving_face_ids', None)
        if index is None or (callable(moving) and moving()):
            face, placement = self.pick_face_placement(x, y)
            if face is None:
                return None
            from core.snap import face_plane_world
            return face_plane_world(face, getattr(placement, 'xform', None))
        row = self.view.pick_row(x, y) if picked_row is ... else picked_row
        if row is None:
            return None
        origin, direction = self._pixel_to_ray(x, y)
        if origin is None or direction is None:
            return None
        from views.wgpu_inference import surface_hit
        hit = surface_hit(index, row, origin, direction)
        if hit is None or hit[1] is None:
            return None
        return hit

    def _segment_to_pixels(self, a, b):
        """GL's: cut where the segment passes behind the eye instead of
        dropping it (a guide preview is kilometres long)."""
        from views.viewport import Viewport
        return Viewport._segment_to_pixels(self, a, b)

    def _clip_w(self):
        from views.viewport import Viewport
        return Viewport._clip_w(self)

    _CLIP_W = 1e-3      # Viewport._CLIP_W (no import cycle at class level)

    def _pixel_to_ray(self, x, y):
        return self.view._pixel_to_ray(x, y)

    def _world_to_pixel(self, point, mvp=None):
        if mvp is None:
            return self.view._world_to_pixel(point)
        # Called once per vertex by the walks (box select's fallback): an
        # import statement each time went through shiboken's import hook,
        # a visible share of the box hang samples (2026-11-18).
        return _viewport_class()._world_to_pixel(self, point, mvp)

    def _project_to_lock_line(self, *args):
        return self.view._project_to_lock_line(*args)

    def _ray_hit_plane(self, cursor, plane):
        from views.viewport import Viewport
        return Viewport._ray_hit_plane(self, cursor, plane)

    def _picked_mesh(self, x, y):
        row = self._selection_row(x, y) if self.mode == 'select' else self.view.pick_row(x, y)
        resolved = self.view.resolve(row)
        if resolved is None:
            return None
        group, _ = resolved
        packed = self.view.packed
        world = packed.instances['world'][row]
        mesh = group.mesh if group is not None else self.scene.loose_mesh
        code = int(packed.instances['info'][row, 0])
        origins = getattr(packed, 'prototype_origins', ())
        if code < len(origins) and (any(origins[code]) or any(packed.origin)):
            # The tool's active mesh is in scene space; the GPU row expects
            # vertices relative to the prototype anchor.
            st = mesh.__dict__.get('_store')
            frame = st.frame_origin if st is not None else (0., 0., 0.)
            world = world.astype(float)
            world[3, :3] += np.subtract(frame, origins[code]) @ world[:3, :3]
        matrix = QMatrix4x4(world.T.flatten().tolist())
        if group is self.scene.edit_group and group is not None:
            # The editing mesh is in world coordinates even if the retained
            # GPU row still draws its local-space definition.
            matrix = QMatrix4x4()
        return mesh, group, matrix

    def pick_face_placement(self, x, y):
        """The face of the placement under the cursor that the cursor ray
        meets first, and that placement (``None`` for loose geometry).

        One vectorised ray test over the mesh's triangles, kept per mesh
        (:meth:`_face_triangles`); it used to triangulate every face of
        the mesh in Python on every call — twice a hover (the work plane
        and the face centre), ~4 ms over 600 faces and without bound on a
        big loose mesh. The same Möller–Trumbore thresholds, the same
        first-face-wins on a tie."""
        packed = getattr(self.view, 'packed', None)
        if any(getattr(packed, 'origin', ())) or getattr(self.view, '_remote_meshes', ()):
            with self.view._geometry_frame():
                return self._face_placement_at(x, y)
        return self._face_placement_at(x, y)

    def _face_placement_at(self, x, y):
        from types import SimpleNamespace
        from views.viewport import _moller_trumbore
        picked = self._picked_mesh(x, y)
        if picked is None:
            return None, None
        mesh, group, matrix = picked
        origin, direction = self._pixel_to_ray(x, y)
        inverse, ok = matrix.inverted()
        if not ok:
            return None, None
        origin = inverse.map(origin)
        direction = inverse.mapVector(direction)
        faces, owner, v0, e1, e2 = self._face_triangles(mesh)
        if not len(v0):
            return None, (SimpleNamespace(xform=matrix) if group is not None else None)
        t, hit = _moller_trumbore(np.array(origin.toTuple()),
                                  np.array(direction.toTuple()), v0, e1, e2)
        excluded = getattr(self.tool, 'moving_face_ids', lambda: set())()
        if excluded and hit.any():
            moving = np.fromiter((f.key in excluded for f in faces), bool, len(faces))
            hit &= ~moving[owner]
        best = None
        if hit.any():
            best = faces[int(owner[int(np.argmin(np.where(hit, t, np.inf)))])]
        return best, (SimpleNamespace(xform=matrix) if group is not None else None)

    #: Meshes whose triangles :meth:`_face_triangles` keeps at once (the
    #: cursor goes back and forth between a few).
    FACE_TRIANGLE_MEMO = 8

    def _face_triangles(self, mesh):
        """``(faces, owner, v0, e1, e2)``: the shown faces of ``mesh`` and
        their triangles in its own space, ``owner[i]`` the face of triangle
        ``i`` — kept until the mesh mutates or the scene's version moves
        (a hide, a vertex dragged)."""
        store = mesh.__dict__.get('_store')
        key = (getattr(mesh, '_mut_serial', None), getattr(self.scene, 'version', None),
               tuple(store.frame_origin) if store is not None else None)
        memo = self.__dict__.setdefault('_face_tri_memo', {})
        hit = memo.get(id(mesh))
        if hit is not None and hit[0] is mesh and hit[1] == key:
            return hit[2]
        faces, owner, pts = [], [], []
        for face in mesh.faces:
            if getattr(face, 'hidden', False):
                continue
            tris = face.triangulate()
            if not tris:
                continue
            k = len(faces)
            faces.append(face)
            for a, b, c in tris:
                owner.append(k)
                pts.append((a.toTuple(), b.toTuple(), c.toTuple()))
        arr = np.asarray(pts, dtype=np.float64).reshape(-1, 3, 3)
        v0 = np.ascontiguousarray(arr[:, 0])
        value = (faces, np.asarray(owner, np.int64), v0,
                 np.ascontiguousarray(arr[:, 1] - v0),
                 np.ascontiguousarray(arr[:, 2] - v0))
        memo.pop(id(mesh), None)
        while len(memo) >= self.FACE_TRIANGLE_MEMO:
            memo.pop(next(iter(memo)))
        memo[id(mesh)] = (mesh, key, value)
        return value

    def pick_face_any(self, x, y):
        face, _ = self.pick_face_placement(x, y)
        picked = self._picked_mesh(x, y)
        if picked is None:
            return None, None
        mesh, group, _ = picked
        if face is None or not self.scene.entity_selectable(face):
            return None, None
        if group is self.scene.edit_group:
            return face, None
        # The ray hit used for a drawing plane can also hit a locked
        # reference. Paint and the other editing tools need an editable
        # placement, including the container selected in this context.
        owner = self.view.context_placement(group)
        if (owner is None or not self.scene.entity_selectable(group)
                or not self.scene.entity_selectable(owner)):
            return None, None
        return face, group

    def pick_face(self, x, y):
        face, group = self.pick_face_any(x, y)
        return face if group is None and self.scene.entity_selectable(face) else None

    def _selection_row(self, x, y):
        row = self.view.pick_row(x, y)
        if row is not None and self.view.inference_index is not None \
                and self._linework_rows()[row]:
            # GL's pick_group: the face under the cursor before a line
            # drawn over it (the lines are SelectTool._linework's).
            row = self._face_row_under(x, y, row)
        context = self.scene.edit_group
        if context is not None:
            group = self.view.packed.groups[row] if row is not None else None
            if group is not context and self.view.context_placement(group) is None:
                # Fade retains depth. GL picks only the editable subtree,
                # even when another placement covers it in the ID pass.
                from views.wgpu_inference import context_hit
                row = (context_hit(self.view, context, x, y)
                       if self.view.inference_index is not None
                       else self._context_mesh_row(context, x, y))
        return row

    def _context_mesh_row(self, context, x, y):
        """The open group's row when the cursor ray meets its own faces —
        what :func:`context_hit` answers while the snap index is still
        being built on its worker (a second and more on a big model, after
        opening it and after each repack). Without it a face covered in
        the ID pass by the faded rest — a plate against a profile, a nut on
        its bolt — took no click at all until the index came back."""
        from views.viewport import _moller_trumbore
        rows = [r for r in self.view.uid_rows.get(context.uid, ())
                if 0 <= r < len(self.view.packed.instances)
                and self.view.packed.groups[r] is context
                and not self.view.packed.instances['info'][r, 2] & 129]
        if not rows:
            return None
        _faces, _owner, v0, e1, e2 = self._face_triangles(self.scene.mesh)
        if not len(v0):
            return None
        origin, direction = self._pixel_to_ray(x, y)
        _t, hit = _moller_trumbore(np.array(origin.toTuple()),
                                   np.array(direction.toTuple()), v0, e1, e2)
        return rows[0] if hit.any() else None

    def _face_row_under(self, x, y, default):
        """The row of the front face at ``(x, y)`` among the placements
        that have faces — the ID pass shows the line of a lines-only
        placement drawn over it — else ``default``."""
        from views.wgpu_inference import _front_at, eligible_rows
        v = self.view
        rows = eligible_rows(v.inference_index, v.packed)
        rows = rows[~self._linework_rows()[rows]]
        if not len(rows):
            return default
        mvp = v.camera.projection_matrix() * v.camera.view_matrix()
        hit = _front_at(v.inference_index, v, x, y, rows, mvp)
        return default if hit is None else hit[2]

    def pick_group(self, x, y):
        row = self._selection_row(x, y)
        self.view.hovered_row = row
        group = self.view.packed.groups[row] if row is not None else None
        target = self.view.context_placement(group)
        return target if target is not None and self.scene.entity_selectable(target) else None

    def pick_locked_group(self, x, y):
        picked = self._picked_mesh(x, y)
        target = self.view.context_placement(picked[1]) if picked else None
        if target is not None and getattr(target, 'locked', False):
            visible, layer_locked = self.scene._layer_state(target)
            if visible and not layer_locked:
                return target
        return None

    def pick_locked_image_plane(self, x, y):
        from views.viewport import Viewport
        return Viewport.pick_locked_image_plane(self, x, y)

    def _face_or_image_in_front(self, x, y):
        return self.pick_face(x, y)

    def _context_pick(self, x, y):
        from views.viewport import Viewport
        return Viewport._context_pick(self, x, y)

    #: How near (logical pixels) a click must come to a line to take it.
    EDGE_PICK_PX = 12.

    def _pick_loose_edge(self, x, y, hidden):
        """``(edge, pixels)`` of the nearest selectable loose edge within
        :attr:`EDGE_PICK_PX` that ``hidden`` does not hide, else ``(None,
        inf)``."""
        best, distance = None, self.EDGE_PICK_PX
        for edge in self.scene.mesh.edges:
            if not self.scene.entity_selectable(edge) or (
                    (getattr(edge, 'hidden', False) or getattr(edge, 'soft', False))
                    and not self.scene.show_hidden_geometry):
                continue
            a, b = self._world_to_pixel(edge.a), self._world_to_pixel(edge.b)
            if a is None or b is None:
                continue
            a, b = np.asarray(a), np.asarray(b)
            d = b-a
            t = np.clip(np.dot(np.array([x, y])-a, d)/max(np.dot(d, d), 1e-12), 0, 1)
            dist = np.linalg.norm(a+t*d-[x, y])
            if dist < distance and not hidden(edge.a + (edge.b-edge.a)*float(t)):
                best, distance = edge, dist
        return best, (float(distance) if best is not None else float('inf'))

    def pick_edge(self, x, y):
        return self._pick_loose_edge(x, y, self._is_occluded)[0]

    pick_visible_edge = pick_edge

    def linework_hidden(self, point):
        """``Viewport.linework_hidden``: occlusion with the depth slack of
        a line lying on (or a hair off) a face."""
        from views.viewport import linework_slack_point
        return self._is_occluded(linework_slack_point(self.view.camera, point))

    def pick_linework_edge(self, x, y):
        """``(edge, pixels)``: the loose edge the Select tool takes before
        the face under the cursor (``Viewport.pick_linework_edge``)."""
        return self._pick_loose_edge(x, y, self.linework_hidden)

    def _linework_rows(self):
        """Mask over the resident rows: placements of lines only (their
        prototype draws no face, shown or hidden)."""
        packed = self.view.packed
        key = id(packed)
        cached = self.__dict__.get('_linework_rows_memo')
        if cached is not None and cached[0] == key and cached[1] is packed:
            return cached[2]
        faces = packed.commands['count'].astype(np.int64)
        hidden = getattr(packed, 'hidden_commands', None)
        if hidden is not None:
            faces = faces + hidden['count']
        mask = faces[packed.instances['info'][:, 0]] == 0
        self._linework_rows_memo = (key, packed, mask)
        return mask

    def pick_linework_groups(self, x, y):
        """``[(group, pixels)]``, nearest first: the objects made of lines
        only whose visible lines pass within :attr:`EDGE_PICK_PX` of the
        cursor (``Viewport.pick_linework_groups``). Their 1-px lines in
        the ID pass were all the hitbox they had: off by a pixel, at a
        grazing angle or z-fighting the face below, the face's group
        took the click."""
        v = self.view
        if v.inference_index is None or v.packed is None or not len(v.packed.instances):
            return []
        wire = self._linework_rows()
        if not wire.any():
            return []
        from views.wgpu_inference import nearby_edges
        rows = np.flatnonzero(wire)
        edges = nearby_edges(v.inference_index, v.packed, v.camera, v.width(),
                             v.height(), x, y, radius_px=self.EDGE_PICK_PX,
                             cap=48, rows=rows, edit_group=self.scene.edit_group)
        from views.viewport import Viewport
        out, seen = [], set()
        for e in edges:
            if not e.in_group or not e.distance < self.EDGE_PICK_PX:
                continue
            target = v.context_placement(e.group)
            if target is None or id(target) in seen or not self.scene.entity_selectable(target):
                continue
            p = Viewport.edge_point_under_cursor(self, e, x, y)
            if p is None or self.linework_hidden(p):
                continue
            seen.add(id(target))
            out.append((target, float(e.distance)))
        return out

    def linework_groups_around(self, x, y):
        """The objects made of lines only whose screen box holds the
        cursor, smallest first (``Viewport.linework_groups_around``)."""
        v = self.view
        if v.inference_index is None or v.packed is None or not len(v.packed.instances):
            return []
        wire = self._linework_rows()
        if not wire.any():
            return []
        from views.wgpu_inference import eligible_rows, screen_boxes
        from views.viewport import _boxes_around
        mvp = v.camera.projection_matrix() * v.camera.view_matrix()
        lo, hi, _grid, _moved = screen_boxes(v.inference_index, v.packed, mvp,
                                             v.width(), v.height())
        rows = eligible_rows(v.inference_index, v.packed)
        rows = rows[wire[rows] & np.isfinite(lo[rows]).all(axis=1)
                    & np.isfinite(hi[rows]).all(axis=1)]
        edit = self.scene.edit_group

        def owner(row):
            group = v.packed.groups[row]
            return None if group is edit else v.context_placement(group)
        return _boxes_around(rows, lo[rows, 0], lo[rows, 1], hi[rows, 0],
                             hi[rows, 1], x, y, owner, self.scene)

    def _project_px(self, pts):
        matrix = np.asarray((self.view.camera.projection_matrix() *
                             self.view.camera.view_matrix()).data()).reshape(4, 4).T
        clip = pts @ matrix[:, :3].T + matrix[:, 3]
        ok = clip[:, 3] > 0
        w = np.where(ok, clip[:, 3], 1.)
        return ((clip[:, 0]/w*.5+.5)*self.view.width(),
                (1.-(clip[:, 1]/w*.5+.5))*self.view.height(), ok)

    def _clear_annotation_segment(self, a, b):
        """Whether every viewing ray to this segment ends before resident geometry."""
        from views.wgpu_inference import _screen_rows
        view = self.view
        index = getattr(view, "inference_index", None)
        if index is None:
            return True
        pa, pb = self._world_to_pixel(a), self._world_to_pixel(b)
        if pa is None or pb is None:
            return False
        x, y = (pa[0]+pb[0])*.5, (pa[1]+pb[1])*.5
        radius = .5*np.hypot(pa[0]-pb[0], pa[1]-pb[1]) + 2.
        if radius > 128 or not (0 <= x <= view.width() and 0 <= y <= view.height()):
            return False
        mvp = view.camera.projection_matrix() * view.camera.view_matrix()
        rows = _screen_rows(index, view.packed, mvp, view.width(), view.height(),
                            x, y, radius, None)
        if not len(rows):
            return True
        # The near-plane origins and the segment endpoints span a convex
        # quadrilateral. Reject a box only if a whole axis separates it
        # from that quadrilateral (touching at the measured surface is OK).
        oa, _ = self._pixel_to_ray(*pa)
        ob, _ = self._pixel_to_ray(*pb)
        if oa is None or ob is None:
            return False
        origins = np.array([oa.toTuple(), ob.toTuple()])
        ends = np.array([a.toTuple(), b.toTuple()])
        centers, extents = index.world_box_cache
        low, high = centers[rows]-extents[rows], centers[rows]+extents[rows]
        before = (origins.max(axis=0) < low) & (ends.max(axis=0) <= low)
        after = (origins.min(axis=0) > high) & (ends.min(axis=0) >= high)
        return bool((before | after).any(axis=1).all())

    def _is_occluded(self, point):
        from views.wgpu_inference import surface
        pixel = self._world_to_pixel(point)
        if pixel is None:
            return True
        row = self.view.pick_row(*pixel)
        if row is None or self.view.inference_index is None:
            return False
        origin, direction = self._pixel_to_ray(*pixel)
        hit = surface(self.view.inference_index, row, origin, direction)
        return hit is not None and (hit-origin).length() + 1e-4 < (point-origin).length()

    def set_hover(self, entity, placement=None):
        self._mesh_hover = self._hover_entity = entity
        # The component placement a face was picked through: its dots are
        # drawn through that matrix (GL's _hover_placement).
        self._mesh_hover_placement = placement

    def set_suppressed_faces(self, faces):
        if type(getattr(self, 'tool', None)).__name__ == 'TexturePositionTool':
            # TexturePreview replaces the resident material stream itself;
            # suppressing the base would also suppress its mapped preview.
            return
        faces = tuple(faces or ())
        if {f.key for f in faces} == {f.key for f in getattr(self.view, 'suppressed_faces', ())}:
            return
        self.view.suppressed_faces = faces
        renderer = getattr(self.view, 'renderer', None)
        if renderer is not None and hasattr(renderer, 'set_suppressed_faces'):
            from views.wgpu_parity import face_ranges
            renderer.set_suppressed_faces(face_ranges(self.view, faces))
        self.view.update()

    def mesh_feedback(self):
        """Push the tool's depth-tested preview to the renderer and report
        its measurement. Everything that floats over the model (rubber
        band, guide preview, grips, labels, snap) is painted each frame by
        draw_annotations with GL's own painters, so this adds no overlay
        primitives. The upload is skipped when the preview is unchanged:
        the frame loop calls this whenever no primitive is pending."""
        faces = list(getattr(self.tool, 'preview_faces', lambda: [])())
        self.view.preview_faces = faces
        renderer = getattr(self.view, 'renderer', None)
        if renderer is not None and hasattr(renderer, 'set_preview_mesh'):
            from views.wgpu_parity import preview_mesh
            from views.wgpu_annotations import rubber_band
            band = rubber_band(self) if callable(getattr(self.tool, 'rubber_band_lines', None)) else None
            # GL draws a depth-tested band (Push/Pull's forming box) in
            # the scene pass so the faces hide its back edges; the preview
            # faces themselves carry no outline of their own.
            edges = ([(a, b, band[1]) for a, b in band[0]]
                     if band is not None and band[3] else [])
            mesh = preview_mesh(faces, edges)
            key = tuple(bytes(part) for part in mesh)
            # The selected groups' boxes, depth-tested like GL's: the
            # preview is this tool's while it is active. Cached vertices,
            # compared by their key, not their bytes.
            boxes = getattr(self.view, '_selection_box_vertices', None)
            box_key, box = boxes() if boxes is not None else (None, None)
            if (key, box_key) != getattr(self.view, '_preview_key', None):
                if box is not None and len(box):
                    from views.wgpu_parity import raw
                    mesh = (mesh[0], np.concatenate((mesh[1], raw(box))))
                renderer.set_preview_mesh(*mesh)
                self.view._preview_key = (key, box_key)
        label = self.tool.value_label() if hasattr(self.tool, 'value_label') else None
        if label is not None:
            self.measurementChanged.emit(label[0])

    def draw_annotations(self, painter):
        from views.wgpu_annotations import (draw_annotations, draw_tool_feedback,
                                            draw_tool_overlays)
        from views.wgpu_overlay import draw_snap_indicator
        self._hover_entity = getattr(self, '_mesh_hover', None)
        marks = getattr(self.view, '_draw_selection_marks', None)
        if marks is not None:
            marks(painter)
        layout = draw_annotations(painter, self.scene, self.camera,
            (painter.device().width()/painter.device().devicePixelRatioF(),
             painter.device().height()/painter.device().devicePixelRatioF()), occluded=self._is_occluded,
            hover=self._hover_entity, clear_segment=self._clear_annotation_segment,
            guide_lines=not hasattr(getattr(self.view, 'renderer', None), 'set_lines'))
        draw_tool_feedback(painter, self, layout)
        snap = self.last_snap
        if snap is not None and getattr(snap, 'kind', None) and hasattr(snap, 'color'):
            # With its tooltip ("Endpoint", "On Edge in group"), as GL.
            draw_snap_indicator(painter, snap, layout._world_to_pixel,
                                layout._clip_pixel_line,
                                scene=getattr(self, 'scene', None))
        draw_tool_overlays(painter, self)

    def world_to_pixel(self, world):
        """Extension API (``ExtensionApp.add_overlay``) on the Metal view:
        the hidden GL widget's own projection uses its own size."""
        return self._world_to_pixel(world)

    def world_to_pixels(self, pts):
        return self._project_px(np.asarray(pts, dtype=float).reshape(-1, 3))

    def _draw_occluded_segment(self, painter, a, b):
        from views.viewport import Viewport
        Viewport._draw_occluded_segment(self, painter, a, b)

    def _project_samples(self, points):
        return [(self._world_to_pixel(p), not self._is_occluded(p)) for p in points]

    def _occluded_on_screen(self, point):
        return self._is_occluded(point)

    def pick_dimension(self, *args, **kwargs):
        from views.viewport import Viewport
        return Viewport.pick_dimension(self, *args, **kwargs)

    def pick_text_label(self, *args, **kwargs):
        from views.viewport import Viewport
        return Viewport.pick_text_label(self, *args, **kwargs)

    def _unsupported_pick(self, *args, **kwargs):
        return None

    def pick_geopath(self, *args):
        from views.viewport import Viewport
        return Viewport.pick_geopath(self, *args)

    def _draw_geo_path_label(self, *args):
        from views.viewport import Viewport
        return Viewport._draw_geo_path_label(self, *args)

    def section_plane_frame(self, plane):
        from views.viewport import Viewport
        return Viewport.section_plane_frame(self, plane)

    def _clip_segment_front(self, a, b):
        from views.viewport import Viewport
        return Viewport._clip_segment_front(self, a, b)

    def pick_section_plane(self, x, y):
        from views.viewport import Viewport
        return Viewport.pick_section_plane(self, x, y)

    pick_image = _unsupported_pick
    def pick_image_plane(self, x, y):
        from views.viewport import Viewport
        return Viewport.image_plane_at(self, x, y, selectable_only=True)

    def image_in_front_of(self, face, x, y):
        from views.viewport import Viewport
        return Viewport.image_in_front_of(self, face, x, y)

    def edge_point_under_cursor(self, edge, x, y):
        from views.viewport import Viewport
        return Viewport.edge_point_under_cursor(self, edge, x, y)

    def pick_extension_item(self, x, y):
        from views.viewport import Viewport
        return Viewport.pick_extension_item(self, x, y)

    _group_obb = _unsupported_pick
    def begin_groups_preview(self, groups):
        # Their selection boxes stay behind until the commit (GL skips them).
        self._moving_groups = tuple(groups)
        selected = {g.uid for root in groups for g, _ in iter_placements(root)}
        self.ids = np.asarray([row for uid in selected
                               for row in self.view.uid_rows.get(uid, ())], np.int64)
        billboards = getattr(self.view, '_billboards', None)
        if billboards is not None:
            billboards.restore()
            billboards.key = None
        self.original = self.view.packed.instances[self.ids].copy()

    def set_groups_preview_offset(self, delta):
        matrix = QMatrix4x4()
        matrix.translate(delta)
        self.set_groups_preview_matrix(matrix)

    def set_groups_preview_matrix(self, matrix):
        if self.original is None:
            return
        transform = np.asarray(matrix.data(), np.float32).reshape(4, 4).T
        rows = self.original.copy()
        rows['world'] = (transform @ rows['world'].transpose(0, 2, 1)).transpose(0, 2, 1)
        self.write_rows(rows)

    def end_groups_preview(self):
        self._moving_groups = ()
        self.restore()
        self.original = None

    image_plane_at = _unsupported_pick

    def _resident_nearby_edges(self, px, py, radius_px=48., cap=48):
        from views.wgpu_inference import nearby_edges
        v = self.view
        if v.inference_index is None:
            return []
        return nearby_edges(v.inference_index, v.packed, v.camera, v.width(),
                            v.height(), px, py, radius_px=radius_px, cap=cap,
                            edit_group=self.scene.edit_group)

    def _nearby_loose_edges(self, px, py, radius_px=64., cap=400):
        return [e for e in self._resident_nearby_edges(px, py, radius_px, cap) if not e.in_group]

    def _nearby_group_edges(self, px, py, radius_px=48., cap=48):
        return [e for e in self._resident_nearby_edges(px, py, radius_px, cap) if e.in_group]

    def reference_tangent(self, point):
        """Arc continuation from bounded resident curve features, not GL scans."""
        from views.wgpu_inference import tangent_direction
        v = self.view
        if getattr(v, 'inference_index', None) is None:
            return None
        pixel = v._world_to_pixel(point)
        if pixel is None:
            return None
        edges = self._resident_nearby_edges(*pixel, radius_px=12., cap=48)
        for row in dict.fromkeys(edge.row for edge in edges):
            direction = tangent_direction(v.inference_index, v.packed, row, point)
            if direction is not None:
                return direction
        return None
