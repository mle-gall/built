# SPDX-License-Identifier: GPL-3.0-or-later
"""Placement bounds, projected mesh previews and host selection handoff."""
from ingetrazo_core import Vec3 as QVector3D, ToolMachine

_MAX_PREVIEW_EDGES = 2000

def _edge_count(mesh):
    """How many edges ``mesh`` has, or None for a deferred mesh (it is
    previewed by its box, and counting would build it)."""
    from core.mesh import DeferredMesh
    if type(mesh) is DeferredMesh:
        return None
    return len(mesh.edges)


class PlaceGroupHost:
    name = "Place component"
    uses_snap = True
    wireframe_color = (0.13, 0.17, 0.23, 1.0)
    wireframe_depth_tested = False      # the pending component floats on top

    def __init__(self, group, align_to_face: bool = False,
                 anchor: QVector3D | None = None) -> None:
        super().__init__()
        self._group = group
        self._anchor = (QVector3D(anchor) if anchor is not None
                        else (self._back_center(group) if align_to_face
                              else self._base_center(group)))
        self._align = align_to_face
        # Local preview segments, relative to the anchor (computed once).
        # A container's own mesh is usually empty and its children can be
        # anything: a SMALL placement tree (a 3D text, one group per
        # letter) previews its real outlines; a big one (an imported
        # document) gets the box of the whole placement, the honest
        # preview that costs nothing.
        if getattr(group, "children", None):
            self._segments = self._placement_segments(group)
        elif _edge_count(group.mesh) is None:
            self._segments = []         # deferred: the box, nothing built
        else:
            self._segments = [
                (QVector3D(e.a) - self._anchor, QVector3D(e.b) - self._anchor)
                for e in group.mesh.edges[:_MAX_PREVIEW_EDGES]]
        if not self._segments:
            self._segments = self._bbox_segments(group)
        self._attachment = None
        self._machine.set("load", (group.uid, self._anchor.toTuple(), self._align,
                                   [(a.toTuple(), b.toTuple()) for a, b in self._segments]))

    def _placement_segments(self, group) -> list:
        """Edges of the group and its nested placements in world space,
        relative to the anchor — empty when the tree holds more edges than
        the preview cap, so the caller falls back to the box."""
        from core.group import iter_placements
        placements = list(iter_placements(group))
        # Counted without building a deferred mesh (core.mesh.DeferredMesh):
        # ``len(mesh.edges)`` builds it, and a linked or imported model of
        # millions of faces sat 60 s in the preview's head count before the
        # box fallback was chosen anyway. A deferred one is drawn as the box.
        total = 0
        for g, _m in placements:
            n = _edge_count(g.mesh)
            if n is None:
                return []
            total += n
            if total > _MAX_PREVIEW_EDGES:
                return []
        out = []
        for g, m in placements:
            for e in g.mesh.edges:
                a, b = QVector3D(e.a), QVector3D(e.b)
                if m is not None:
                    a, b = m.map(a), m.map(b)
                out.append((a - self._anchor, b - self._anchor))
        return out

    @staticmethod
    def _world_points(group):
        """Every vertex of the group AND its nested placements, in world
        space — what a container's extent is made of."""
        from core.group import placement_points
        return placement_points(group)

    def _bbox_segments(self, group):
        """Wireframe box fallback: meshes without edges (billboards) and
        containers (their geometry lives in the children)."""
        pts = self._world_points(group)
        if not len(pts):
            return []
        lo, hi = pts.min(axis=0), pts.max(axis=0)
        a = QVector3D(float(lo[0]), float(lo[1]), float(lo[2])) - self._anchor
        b = QVector3D(float(hi[0]), float(hi[1]), float(hi[2])) - self._anchor
        c = [QVector3D(x, y, z)
             for z in (a.z(), b.z()) for y in (a.y(), b.y())
             for x in (a.x(), b.x())]
        idx = [(0, 1), (1, 3), (3, 2), (2, 0), (4, 5), (5, 7), (7, 6),
               (6, 4), (0, 4), (1, 5), (2, 6), (3, 7)]
        return [(c[i], c[j]) for i, j in idx]

    @classmethod
    def _base_center(cls, group) -> QVector3D:
        pts = cls._world_points(group)
        if not len(pts):
            return QVector3D(0.0, 0.0, 0.0)
        lo, hi = pts.min(axis=0), pts.max(axis=0)
        return QVector3D(float(lo[0] + hi[0]) / 2.0,
                         float(lo[1] + hi[1]) / 2.0,
                         float(lo[2]))

    @classmethod
    def _back_center(cls, group) -> QVector3D:
        """The back-bottom-centre a sign hangs from: centred across the width,
        on the BACK face (max +Y) and the base (min Z) — the one point that
        stays put when the thickness is re-edited, so the sign is planted and
        glued by the same face."""
        pts = cls._world_points(group)
        if not len(pts):
            return QVector3D(0.0, 0.0, 0.0)
        lo, hi = pts.min(axis=0), pts.max(axis=0)
        return QVector3D(float(lo[0] + hi[0]) / 2.0,
                         float(hi[1]),
                         float(lo[2]))


    @staticmethod
    def _face_frame(normal):
        return tuple(QVector3D(*p) for p in ToolMachine("place_group").query("basis", normal.toTuple()))

    def host_placement_normal(self, vp, px):
        pick = getattr(vp, "pick_face_placement", None) or getattr(vp, "pick_face_any", None)
        hit = pick(*px) if pick else None
        face, owner = hit if isinstance(hit, tuple) else (hit, None)
        if face is not None:
            from core.snap import face_plane_world
            return face_plane_world(face, getattr(owner, "xform", None))[1].toTuple()

    def on_click(self, ctx):
        if self._group is not None and self._align and self._group.text3d:
            from core.annotation_anchor import picked_attachment
            self._attachment = picked_attachment(ctx)
        self._drop(ctx.viewport, self._pointer("click", ctx))

    def place_at(self, viewport, world, *, attachment=None):
        self._attachment = attachment
        self._drop(viewport, {"type": "paste_at", "offset": world.toTuple()})

    def _drop(self, vp, event):
        from core.i18n import tr
        if self._send(vp, event)["committed"]:
            self._group = None
            vp.flash_status(tr("Component placed — Move (M) adjusts it"))
            window = vp.window()
            if hasattr(window, "_activate_tool"):
                window._activate_tool("select")
            vp.update()

    def on_cancel(self, vp):
        self._group = None
        super().on_cancel(vp)

    def on_deactivate(self, vp):
        self._group = None
        super().on_deactivate(vp)
