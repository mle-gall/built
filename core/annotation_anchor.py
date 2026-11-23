# SPDX-License-Identifier: GPL-3.0-or-later
"""Bind native annotation coordinates to the host's picks and archive geometry."""
import weakref

from ingetrazo_core import Vec3

from core.outliner import world_matrix
from core.protocol.keys import element, kind_of


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
        matrix = world_matrix(scene.groups, owner)
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
        matrix = world_matrix(self.scene().groups, owner)
        return matrix.map(point) if matrix is not None else point

    def archive(self):
        entity, _owner = self._resolve()
        if entity is None:
            return None
        return {"path": self.binding.path, "kind": self.binding.kind,
                "vertices": [v.position.toTuple() for v in _vertices(entity, self.binding.kind)]}

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
            return cls.capture(scene, entity, owner, world)
        except (ValueError, TypeError, KeyError):
            return None


def picked_attachment(ctx):
    scene = getattr(ctx.viewport, "scene", None)
    if scene is None:
        return None
    kind = getattr(getattr(ctx, "snap", None), "kind", None)
    if kind == "endpoint":
        from core.dimension import resolve_vertex_anchor
        hold = resolve_vertex_anchor(scene, ctx.world)
        if hold is not None:
            return Attachment.capture(scene, hold.vertex, hold.chain[-1] if hold.chain else None, ctx.world)
    if kind in ("on_edge", "midpoint"):
        edge = getattr(ctx.snap, "edge", None)
        if getattr(edge, "in_group", False):
            from core.dimension import _anchor_candidates
            for root, placement, inverse in _anchor_candidates(scene, edge.a, 1e-4):
                owner = getattr(edge, "group", None)
                if owner is not None and owner not in (root, placement):
                    continue
                mesh = placement.mesh
                a, b = (mesh.vertex_at(inverse.map(p) if inverse is not None else p)
                        for p in (edge.a, edge.b))
                hit = mesh.find_edge(a, b) if a is not None and b is not None else None
                if hit is not None:
                    return Attachment.capture(scene, hit, placement, ctx.world)
        if kind_of(edge) == "edge":
            return Attachment.capture(scene, edge, getattr(scene, "edit_group", None), ctx.world)
    if kind == "on_face":
        hit = ctx.viewport.pick_face_any(ctx.screen.x(), ctx.screen.y())
        if hit:
            face, owner = hit
            return Attachment.capture(scene, face, owner or getattr(scene, "edit_group", None), ctx.world)
    return None
