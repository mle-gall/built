# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Fillet (round) the edges of a solid — Rafael's «herramienta de redondeo»
(review of 2026-09-10, C3).

An edge shared by two faces is replaced by a strip of quads on the
cylinder of the given radius tangent to both faces; the faces are cut back
to the tangent lines. Where the strip ends:

* at a plain vertex with one more face (the end cap of a box edge) the
  strip is cut by that face's plane and the cap gets the arc in place of
  its corner;
* at an open end (no other face) the section is perpendicular to the edge;
* where two rounded edges meet (a chain: the rim of a slab, the rounded
  outline of a prism) both strips stop on the bisecting (mitre) plane;
* where three rounded edges meet (a box corner) each strip stops at the
  other edges' tangent distance and a spherical patch fills the corner.

Everything else — four rounded edges at a vertex, extra faces at a vertex,
a rebuilt face that would leave its plane — is refused with a reason, so
the model is never left half-cut. The result is a plan applied to the
mesh in one go (:func:`apply_fillet`), which the tool wraps in a snapshot
command. Strip seams and the tangent lines are softened so the rounding
reads as one smooth surface; the arcs on end caps are curves.

The same plan is a bevel. The size can be read four ways (``measure``):
the arc's radius, the distance cut back on each face, the width of the
new face (the chord between the two tangent lines) or its depth (from
the old edge to that chord). ``profile`` shapes the section, the
superellipse of the usual bevel: 0.5 is the circular arc, 0.25 a
straight chamfer, toward 1 it fills out to the old square edge, toward 0
it hollows into a cove. One segment is a flat chamfer whatever the
profile. ``shape`` swaps the superellipse for a drawn outline — steps, a
round with support loops, a cornice or a crown moulding, see
:mod:`ingetrazo_core.bevel_profiles`; a box corner filled with it is the
spherical product of the outline with itself.
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field

from ingetrazo_core import Vec3 as QVector3D

from ingetrazo_core import bevel_profiles
from core.i18n import tr
from core.mesh import edge_flags, stamp_edge_flags

#: Faces meeting at less than this angle (or more than 180° minus it) are
#: too flat or too sharp to round.
MIN_ANGLE_DEG = 2.0
#: Rebuilt loops may leave their face plane by this much (relative).
PLANE_TOL = 1e-4
#: How the size of a fillet is read.
MEASURES = ("radius", "offset", "width", "depth")
#: The profile that is the circular arc — the plain fillet.
CIRCULAR = 0.5
#: How a box corner (three edges at a vertex) is closed: a curved patch
#: that follows the profile (the usual «grid fill»), or a flat cut (the
#: usual «cutoff») — each strip end capped by a flat face.
CORNERS = ("round", "cut")


def _key(p: QVector3D) -> tuple[int, int, int]:
    return (round(p.x() * 1e5), round(p.y() * 1e5), round(p.z() * 1e5))


def _unit(v: QVector3D) -> QVector3D | None:
    ln = v.length()
    return None if ln < 1e-12 else v / ln


@dataclass
class FilletPlan:
    radius: float                # the size, read as ``measure`` says
    segments: int
    profile: float = CIRCULAR
    measure: str = "radius"
    corners: str = "round"
    shape: str | None = None
    removed_faces: list = field(default_factory=list)
    removed_edges: list = field(default_factory=list)
    # (loop positions, [hole positions], attrs, interior) of rebuilt faces
    rebuilt: list = field(default_factory=list)
    strips: list = field(default_factory=list)     # quad position loops
    patches: list = field(default_factory=list)    # triangle position loops
    soft: list = field(default_factory=list)       # (p, q) pairs to soften
    curves: list = field(default_factory=list)     # arc polylines to tag
    face_attrs: dict = field(default_factory=dict) # attrs for new faces
    # (p, q, edge flags) carried to the edge p–q once the faces are in
    flags: list = field(default_factory=list)

    _store_plan: object = field(default=None, repr=False, compare=False)

    @property
    def new_faces(self) -> list:
        return self.strips + self.patches


def _exponent(profile: float) -> float:
    """Superellipse exponent of a profile: 0.5 → 2 (circle), 0.25 → 1
    (straight), toward 1 → ∞ (square), toward 0 → 0 (cove). The section's
    middle point sits at √profile of the way from the centre to the old
    edge."""
    return math.log(4.0) / math.log(1.0 / profile)


def plan_fillet(mesh, edges, radius: float, segments: int = 8,
                profile: float = CIRCULAR, measure: str = "radius",
                corners: str = "round", shape: str | None = None):
    """Plan the rounding of ``edges`` (of ``mesh``) to the size ``radius``,
    read as ``measure`` (one of :data:`MEASURES`), with the section shaped
    by ``profile`` and box corners closed as ``corners`` says (one of
    :data:`CORNERS`) — or, with a ``shape`` from
    :data:`ingetrazo_core.bevel_profiles.PRESETS`, the section drawn as that
    outline. Returns a :class:`FilletPlan`, or a message (str)
    saying why it cannot be done."""
    edges = [e for e in dict.fromkeys(edges)]
    if not edges:
        return tr("Pick an edge to round.")
    if radius <= 0:
        return tr("The radius must be positive.")
    if not 0.0 < profile < 1.0:
        return tr("The profile must be between 0 and 1.")
    if measure not in MEASURES:
        measure = "radius"
    if corners not in CORNERS:
        corners = "round"
    if isinstance(shape, (list, tuple)):
        if not bevel_profiles.valid(shape):
            return tr("The drawn profile must run from one face to the other.")
        shape = tuple((float(x), float(y), bool(sm)) for x, y, sm in shape)
    elif shape not in bevel_profiles.PRESETS:
        shape = None
    segments = max(int(segments), 1)
    store = getattr(mesh, "_store", None)
    if store is not None and hasattr(store, "plan_fillet"):
        from core.store_mesh import _hypot
        hypot = _hypot()
        if hypot:
            return _store_round_plan(
                mesh, FilletPlan(radius, segments, profile, measure, corners, shape),
                store.plan_fillet, [e.key for e in edges], radius, segments,
                profile, measure, corners, "custom" if isinstance(shape, tuple) else shape or "",
                shape if isinstance(shape, tuple) else [], hypot, vertices=False)
    return _object_fillet(edges, FilletPlan(radius, segments, profile, measure, corners, shape))


def _shared_attrs(f1, f2) -> dict:
    """The new faces take the material when both neighbours share it."""
    a, b = f1.attrs or {}, f2.attrs or {}
    keys = ("color", "texture", "mat", "opacity", "pbr")
    if all(a.get(k) == b.get(k) for k in keys):
        return {k: a[k] for k in keys if k in a}
    return {}


def _fmt(p: QVector3D) -> str:
    return f"({p.x():.2f}, {p.y():.2f}, {p.z():.2f})"


def _object_fillet(selected, plan):
    """Pass just the incident geometry, keeping distinct handles at coincident points."""
    from ingetrazo_core import MeshStore

    ends = dict.fromkeys(v for e in selected for v in (e.v0, e.v1))
    around = {v: list(v.faces()) for v in ends}
    disks = {v: list(v.edges) for v in ends}
    edges = dict.fromkeys(selected)
    for disk in disks.values():
        edges.update(dict.fromkeys(e for e in disk if e.faces))
    faces = dict.fromkeys(f for fs in around.values() for f in fs)
    faces.update(dict.fromkeys(f for e in edges for f in e.faces))
    vertices = dict.fromkeys(v for e in edges for v in (e.v0, e.v1))
    vertices.update(dict.fromkeys(v for f in faces for r in (f.loop, *f.hole_loops) for v in r))
    vi = {v: i for i, v in enumerate(vertices)}
    ei = {e: i for i, e in enumerate(edges)}
    fi = {f: i for i, f in enumerate(faces)}
    shape = plan.shape
    outline = bevel_profiles.sample(shape, plan.segments) if shape else []
    return _store_round_plan(
        None, plan, MeshStore.plan_object_fillet,
        [(v.position.toTuple(), [ei[e] for e in disks.get(v, ()) if e in ei],
          [fi[f] for f in around.get(v, ())]) for v in vertices],
        [((vi[e.v0], vi[e.v1]), [fi[f] for f in e.faces]) for e in edges],
        [([[vi[v] for v in r] for r in (f.loop, *f.hole_loops)], f.normal().toTuple())
         for f in faces], len(selected), plan.radius, plan.segments, plan.profile,
        plan.measure, plan.corners == "cut", outline,
        isinstance(shape, tuple) or shape in bevel_profiles.ASYMMETRIC, shape == "steps",
        vertices=False, handles=(list(edges), list(faces)))


def _store_round_plan(mesh, plan, planner, *args, vertices, handles=None):
    """Adapt a native preview to the same FilletPlan the tools display."""
    from core.store_mesh import EdgeView, FaceView
    try:
        native = planner(*args)
    except ValueError as exc:
        if len(exc.args) != 2:
            raise
        message, point = exc.args
        return tr(message, p=_fmt(QVector3D(*point))) if point else tr(message)

    def vectors(points):
        return [QVector3D(*p) for p in points]

    if handles is None:
        plan._store_plan = native
        def edge_at(key):
            return EdgeView(mesh, key)
        def face_at(key):
            return FaceView(mesh, key)
    else:
        edge_at, face_at = (rows.__getitem__ for rows in handles)
    plan.strips = [vectors(r) for r in native.strips]
    plan.patches = [vectors(r) for r in native.patches]
    plan.curves = [vectors(r) for r in native.curves]
    plan.soft = [tuple(vectors(pair)) for pair in native.soft]
    plan.removed_edges = [edge_at(k) for k in native.edges]
    plan.removed_faces = [face_at(k) for k in native.faces]
    for face, rings in zip(plan.removed_faces, native.rebuilt):
        plan.rebuilt.append((vectors(rings[0]), [vectors(h) for h in rings[1:]],
                             dict(face.attrs), face.interior))
    sources = [face_at(k) for k in native.materials]
    plan.flags = [(QVector3D(*a), QVector3D(*b), edge_flags(edge_at(k)))
                  for k, a, b in native.flags]
    if not vertices:
        material = _shared_attrs(*sources)
    else:
        paints = [{k: f.attrs[k] for k in ("color", "texture", "mat", "opacity", "pbr")
                   if k in f.attrs} for f in sources]
        material = paints[0] if paints and all(p == paints[0] for p in paints) else {}
    plan.face_attrs["strip"] = material
    return plan


def apply_fillet(mesh, plan: FilletPlan):
    """Apply a plan to the mesh: remove the cut faces and edges, add the
    rebuilt faces, the strips and the patches, soften the seams, tag the
    end arcs as curves."""
    if plan._store_plan is not None:
        change = mesh._store.apply_fillet(
            plan._store_plan, [mesh.next_curve_id() for _ in plan.curves])
        created = change.created["face"]
        own = mesh.__dict__["_attr_own"]
        for key, (_, _, attrs, _) in zip(created, plan.rebuilt):
            own[key] = dict(attrs)
        material = plan.face_attrs["strip"]
        if material:
            for key in created[len(plan.rebuilt):]:
                own[key] = dict(material)
        segments = sum(len(loop) + sum(map(len, holes))
                       for loop, holes, _, _ in plan.rebuilt)
        segments += sum(map(len, plan.strips)) + sum(map(len, plan.patches))
        mesh._dirty(len(created) + segments + len(plan.curves)
                    + len(change.removed["face"]) + len(change.removed["edge"]))
        return change
    for f in plan.removed_faces:
        mesh.remove_face(f)
    for e in plan.removed_edges:
        if not e.faces:
            mesh.remove_edge(e)
    for loop, holes, attrs, interior in plan.rebuilt:
        face = mesh.add_face(loop, holes)
        face.attrs = dict(attrs)
        face.interior = interior
    attrs = plan.face_attrs.get("strip", {})
    for loop in plan.new_faces:
        face = mesh.add_face(loop)
        if attrs:
            face.attrs = dict(attrs)
    for p, q, flags in plan.flags:
        a, b = mesh.vertex_at(p), mesh.vertex_at(q)
        e = mesh.find_edge(a, b) if (a is not None and b is not None) else None
        if e is not None:
            stamp_edge_flags(e, flags)
    for p, q in plan.soft:
        a, b = mesh.vertex_at(p), mesh.vertex_at(q)
        e = mesh.find_edge(a, b) if (a is not None and b is not None) else None
        if e is not None:
            e.soft = True
    for pts in plan.curves:
        mesh.tag_curve(pts, closed=False)
