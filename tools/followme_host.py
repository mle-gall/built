# SPDX-License-Identifier: GPL-3.0-or-later
"""Mesh picks, renderer objects and localized messages for Follow Me."""
from ingetrazo_core import Vec3
from core.i18n import tr
from core.mesh import Edge, Face
from core.sweep import manual_path_extend, manual_path_start, order_path_edges

_MESSAGES = {
    "invalid": "Follow Me: the selected edges must form one simple path",
    "intro": "Follow Me: click the profile and drag along the path (it highlights red), click to finish — or select the path first, then click the profile",
    "drag": "Follow Me: drag along the path (it highlights red) — click to finish, Esc to cancel; hold Alt over a face to follow its perimeter",
    "touch": "Follow Me: touch the path edges to extrude along them",
}
_FAILED = "Follow Me: could not sweep along that path (reversal or degenerate segment)"


class FollowMeHost:
    wireframe_color = (0.85, 0.16, 0.16, 1.0)

    def __init__(self):
        super().__init__()
        self._entities = {}

    def _keep(self, obj):
        key = str(obj.key)
        self._entities[key] = obj
        return key

    @property
    def _profile(self):
        return self._entities.get(self._state("profile"))

    @property
    def hovered_face(self):
        return self._entities.get(self._state("hovered"))

    @property
    def _path(self):
        path = self._state("path")
        return [Vec3(*p) for p in path] if path is not None else None

    @property
    def _chain(self):
        return [self._entities[k] for k, _ in self._state("chain") or ()]

    def path_points(self):
        return [Vec3(*p) for p in self._state("points")], self._state("closed")

    def preview_faces(self):
        faces = super().preview_faces()
        if faces and self._profile.attrs:
            attrs = dict(self._profile.attrs)
            for face in faces:
                face.attrs = attrs
        return faces

    def host_sweep_pick_face(self, vp, px):
        face = vp.pick_face(*px)
        return self._keep(face) if face is not None else None

    def host_sweep_highlight(self, vp, key):
        vp.set_hover(self._entities.get(key))

    def host_sweep_selection(self, vp, _):
        self._entities.clear()
        edges = [e for e in vp.scene.selection if isinstance(e, Edge)]
        if edges:
            ordered = order_path_edges(edges)
            return ([p.toTuple() for p in ordered[0]], ordered[1]) if ordered else False
        faces = [f for f in vp.scene.selection if isinstance(f, Face)]
        if len(faces) == 1:
            return [p.toTuple() for p in faces[0].vertices], True

    def host_sweep_plane(self, vp, key):
        face = self._entities[key]
        return face.centroid().toTuple(), face.normal().normalized().toTuple()

    def host_sweep_ring(self, vp, key):
        return [p.toTuple() for p in self._entities[key].vertices]

    def host_sweep_chain(self, vp, args):
        edge = vp.pick_edge(*args["px"])
        if edge is None:
            return
        face = self._entities[args["profile"]]
        c, n = face.centroid(), face.normal().normalized()

        def in_plane(e):
            return abs(Vec3.dotProduct(e.a - c, n)) < 1e-6 and abs(Vec3.dotProduct(e.b - c, n)) < 1e-6

        chain = [self._entities[k] for k in args["chain"]]
        crossing = None
        if not chain:
            if in_plane(edge):
                return
            toward = Vec3(*args["world"]) if args["world"] is not None else None
            crossing, chain = manual_path_start(face, edge, toward)
        elif not manual_path_extend(chain, edge, skip=in_plane):
            return
        return (crossing.toTuple() if crossing is not None else None,
                [(self._keep(v), v.position.toTuple()) for v in chain])

    def host_sweep_profile(self, vp, key):
        face = self._entities[key]
        return (face.vertices, *face.holes)

    def host_sweep_message(self, vp, code):
        vp.flash_status(tr(_MESSAGES[code]), 6000 if code in ("intro", "drag") else 2500)

    def host_sweep_done(self, vp, ok):
        vp.set_hover(None)
        if ok:
            vp.scene.selection.clear()
        else:
            vp.flash_status(vp.history.last_rejected or tr(_FAILED))

    def _commit(self, vp, face, path, closed):
        from core.protocol.commands import run
        answer = run(vp.history, "follow_me", face=str(face.key), path=[p.toTuple() for p in path], closed=closed)
        self.host_sweep_done(vp, answer["ok"])
        vp.update()

    def on_release(self, vp):
        if self._send(vp, {"type": "release"})["redraw"]:
            vp.update()

    def on_deactivate(self, vp):
        super().on_deactivate(vp)
        self._entities.clear()
