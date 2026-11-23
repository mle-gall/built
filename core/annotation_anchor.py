# SPDX-License-Identifier: GPL-3.0-or-later
"""Bind native annotation coordinates to the host's picks and archive geometry."""
import weakref

from ingetrazo_core import Matrix4, Vec3

from core.doc_model import Record, record_fields

from core.outliner import world_matrix
from core.protocol.keys import element, kind_of


def _matrix(scene, owner):
    if owner is None:
        return None
    root = getattr(scene, "_root", None)
    return root._anchor_matrix(owner) if root is not None else world_matrix(scene.groups, owner)


def _vertices(entity, kind):
    if kind == "vertex":
        return [entity]
    if kind == "edge":
        return [entity.v0, entity.v1]
    return entity.loop


def _coordinates(vertices):
    return [(v.key, v.position.toTuple()) for v in vertices]


class Attachment:
    def __init__(self, scene, binding):
        self.scene = weakref.ref(scene)
        self.binding = binding

    def __deepcopy__(self, memo):
        # An exporter can clone the complete scene, or just a label.
        scene = self.scene()
        if scene is None:
            return self
        target = memo.get(id(scene), scene)
        if target is scene:
            return type(self)(scene, self.binding)
        # Cloned stores can assign fresh element keys. The archive binding
        # identifies the same occurrence and local vertices in the copy.
        return self.restore(target, self.archive(), self.position())

    @classmethod
    def capture(cls, scene, entity, owner, world):
        from ingetrazo_core import ElementAnchor
        kind = kind_of(entity)
        if kind is None:
            return None
        mesh = scene.loose_mesh if owner is None else owner.mesh
        entity = element(mesh, kind, entity.key)
        if entity is None:
            return None
        matrix = _matrix(scene, owner)
        path = scene.occurrence_path(owner) if owner is not None else ()
        if owner is not None and not path:
            return None
        local = world
        if matrix is not None:
            inverse, ok = matrix.inverted()
            if not ok:
                return None
            local = inverse.map(world)
        vertices = _coordinates(_vertices(entity, kind))
        binding = ElementAnchor.capture(kind, entity.key, path, vertices, local.toTuple())
        return cls(scene, binding) if binding is not None else None

    def _resolve(self):
        scene = self.scene()
        if scene is None:
            return None, None
        b = self.binding
        owner = scene.resolve_occurrence(b.path) if b.path else None
        if b.path and owner is None:
            return None, None
        mesh = scene.loose_mesh if owner is None else owner.mesh
        return element(mesh, b.kind, b.key), owner

    def position(self):
        entity, owner = self._resolve()
        if entity is None:
            return None
        coords = self.binding.position(_coordinates(_vertices(entity, self.binding.kind)))
        if coords is None:
            return None
        point = Vec3(*coords)
        matrix = _matrix(self.scene(), owner)
        return matrix.map(point) if matrix is not None else point

    def placement_frame(self, parent_matrix):
        entity, owner = self._resolve()
        if entity is None:
            return None
        matrix = _matrix(self.scene(), owner) or Matrix4()
        if parent_matrix is not None:
            inverse, ok = parent_matrix.inverted()
            if not ok:
                return None
            matrix = inverse * matrix
        return self.binding.placement_frame(_coordinates(_vertices(entity, self.binding.kind)),
                                            matrix.data())

    def before_mesh_swap(self):
        entity, owner = self._resolve()
        if entity is None:
            return None
        matrix = _matrix(self.scene(), owner)
        points = [v.position for v in _vertices(entity, self.binding.kind)]
        world = [matrix.map(p) if matrix is not None else p for p in points]
        return self.position(), world

    def after_mesh_swap(self, snapshot):
        """The reference editor rebuilds a mesh with fresh keys on enter/exit."""
        if snapshot is None or self.position() is not None:
            return self
        scene = self.scene()
        owner = scene.resolve_occurrence(self.binding.path) if self.binding.path else None
        matrix = _matrix(scene, owner)
        point, vertices = snapshot
        if matrix is not None:
            inverse, ok = matrix.inverted()
            if not ok:
                return self
            vertices = [inverse.map(v) for v in vertices]
        raw = {"kind": self.binding.kind, "path": self.binding.path,
               "vertices": [v.toTuple() for v in vertices]}
        return self.restore(scene, raw, point) or self

    def archive(self):
        entity, _owner = self._resolve()
        if entity is None:
            return None
        vertices = _vertices(entity, self.binding.kind)
        return {"path": self.binding.path, "kind": self.binding.kind,
                "vertices": [v.position.toTuple() for v in vertices],
                "weights": self.binding.indexed_weights([v.key for v in vertices])}

    @classmethod
    def restore(cls, scene, raw, world):
        if not isinstance(raw, dict):
            return None
        try:
            kind, path = raw["kind"], raw["path"]
            owner = scene.resolve_occurrence(path) if path else None
            if path and owner is None:
                return None
            mesh = scene.loose_mesh if owner is None else owner.mesh
            vertices = [mesh.vertex_at(Vec3(*xyz)) for xyz in raw["vertices"]]
            if not vertices or any(v is None for v in vertices):
                return None
            if kind == "vertex" and len(vertices) == 1:
                entity = vertices[0]
            elif kind == "edge" and len(vertices) == 2:
                entity = mesh.find_edge(*vertices)
            elif kind == "face":
                keys = {v.key for v in vertices}
                entity = next((f for f in mesh.faces_at(vertices[0])
                               if {v.key for v in f.loop} == keys), None)
            else:
                return None
            if entity is None:
                return None
            if raw.get("weights") is not None:
                from ingetrazo_core import ElementAnchor
                binding = ElementAnchor.restore(kind, entity.key, path, [v.key for v in vertices],
                                                [tuple(pair) for pair in raw["weights"]])
                return cls(scene, binding) if binding is not None else None
            return cls.capture(scene, entity, owner, world)
        except (ValueError, TypeError, KeyError, OverflowError):
            return None


@record_fields("path", "attachment", "frame")
class TextPlacementAnchor(Record):
    """Document link from a placed 3D sign to the geometry it was hung on."""

    @classmethod
    def create(cls, scene, path, attachment):
        hold = cls()
        hold.path = list(path)
        hold.attachment = attachment
        hold.frame = hold.current_frame(scene)
        return hold if hold.frame is not None else None

    def current_frame(self, scene):
        parent = scene.resolve_occurrence(self.path[:-1]) if len(self.path) > 1 else None
        return self.attachment.placement_frame(_matrix(scene, parent))

    def follow(self, scene):
        sign = scene.resolve_occurrence(self.path)
        if sign is None:
            return
        frame = self.current_frame(scene)
        if frame is not None:
            pose = self.attachment.binding.follow_placement(self.frame, frame,
                                                            (sign.xform or Matrix4()).data())
            if pose is not None:
                sign.xform = pose
                self.frame = frame
                return True
        return False

    def archive(self, scene):
        if scene.resolve_occurrence(self.path) is None:
            return None
        source = self.attachment.archive()
        point = self.attachment.position()
        if source is not None and point is not None:
            return {"sign": list(self.path), "source": source, "point": point.toTuple()}
        return None

    @classmethod
    def restore(cls, scene, raw):
        try:
            path = raw["sign"]
            if not path or scene.resolve_occurrence(path) is None:
                return None
            source = Attachment.restore(scene, raw["source"], Vec3(*raw["point"]))
            return cls.create(scene, path, source) if source is not None else None
        except (KeyError, TypeError, ValueError):
            return None


def edge_by_endpoints(scene, start, end, *, include_loose=True, owner=None):
    """Resolve the two world endpoints reported by a viewport or sheet snap."""
    from core.dimension import _anchor_candidates

    def find(mesh, a, b):
        va, vb = mesh.vertex_at(a), mesh.vertex_at(b)
        return mesh.find_edge(va, vb) if va is not None and vb is not None else None

    if include_loose:
        hit = find(scene.loose_mesh, start, end)
        if hit is not None:
            return hit, None
    for root, placement, inverse in _anchor_candidates(scene, start, 1e-4):
        if owner is not None and owner not in (root, placement):
            continue
        a, b = (inverse.map(p) if inverse is not None else p for p in (start, end))
        hit = find(placement.mesh, a, b)
        if hit is not None:
            return hit, placement
    return None, None


def picked_edge(scene, edge):
    """Map a viewport segment back to its live edge and exact placement."""
    if edge is None:
        return None, None
    if getattr(edge, "in_group", False):
        return edge_by_endpoints(scene, edge.a, edge.b, include_loose=False,
                                 owner=getattr(edge, "group", None))
    owner = getattr(scene, "edit_group", None)
    if kind_of(edge) == "edge":
        return edge, owner
    if not hasattr(edge, "a") or not hasattr(edge, "b") or getattr(edge, "guide", False):
        return None, None
    a, b = (scene.mesh.vertex_at(p) for p in (edge.a, edge.b))
    return (scene.mesh.find_edge(a, b), owner) if a is not None and b is not None else (None, None)


def _picked_face(ctx):
    viewport = ctx.viewport
    pick = (getattr(viewport, "pick_face_inference", None)
            or getattr(viewport, "pick_face_placement", None)
            or getattr(viewport, "pick_face_any", None))
    if pick is None or getattr(ctx, "screen", None) is None:
        return None, None
    try:
        hit = pick(ctx.screen.x(), ctx.screen.y())
    except (AttributeError, RuntimeError):
        # Scripted tool hosts and a renderer between frames can supply the
        # snap point before a face pick index is available.
        return None, None
    face, owner = hit if isinstance(hit, tuple) else (hit, None)
    owner = getattr(owner, "source_occurrence", owner)
    if face is not None and owner is None:
        group = getattr(viewport.scene, "edit_group", None)
        if group is not None and element(group.mesh, "face", face.key) is not None:
            owner = group
    return face, owner


def picked_attachment(ctx):
    scene = getattr(ctx.viewport, "scene", None)
    if scene is None:
        return None
    kind = getattr(getattr(ctx, "snap", None), "kind", None)
    if kind == "endpoint":
        context = getattr(ctx.snap, "context", None)
        face, owner = _picked_face(ctx) if context else (None, None)
        if face is not None:
            mesh = scene.loose_mesh if owner is None else owner.mesh
            face = element(mesh, "face", face.key)
            matrix = _matrix(scene, owner)
            if face is not None:
                for vertex in face.loop:
                    point = matrix.map(vertex.position) if matrix is not None else vertex.position
                    if (point - ctx.world).length() < 1e-4:
                        return Attachment.capture(scene, vertex, owner, ctx.world)
        from core.dimension import resolve_vertex_anchor
        hold = resolve_vertex_anchor(scene, ctx.world)
        if hold is not None:
            return Attachment.capture(scene, hold.vertex, hold.chain[-1] if hold.chain else None, ctx.world)
    if kind in ("on_edge", "midpoint"):
        edge = getattr(ctx.snap, "edge", None)
        hit, owner = picked_edge(scene, edge)
        if hit is not None:
            return Attachment.capture(scene, hit, owner, ctx.world)
    if kind in ("on_face", "face_center"):
        face, owner = _picked_face(ctx)
        if face is not None:
            return Attachment.capture(scene, face, owner, ctx.world)
    return None
