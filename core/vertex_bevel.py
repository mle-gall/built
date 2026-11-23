# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Bevel the corners (vertices) of a model instead of its edges — the
usual bevel's «Affect: Vertices».

Each chosen vertex is cut back by the same distance along every edge
that reaches it. In each face the corner is replaced by the profile's
curve between the two cut points: the superellipse drawn in the
parallelogram of the two edges, tangent to both (``profile`` 0.5 is
circular when the edges are square, 0.25 straight, toward 1 the old
sharp corner, toward 0 a cove). Then:

* a solid corner where three edges meet is closed by a cap that runs
  exactly through the three face curves — the surface
  ``X^r + Y^r + Z^r = 2`` in the frame of the three cut edges, which at a
  box corner and profile 0.5 is the sphere through them; one segment
  makes it the flat triangle that cuts the tip;
* a corner of a flat figure, or of an open surface, has nothing to close:
  only its faces lose the corner;
* more edges at a solid corner close with a flat polygon, one segment
  only, when the cut points lie in one plane.

Anything else is refused with a reason and the model is left as it was.
The result is a :class:`core.fillet.FilletPlan`, applied in one go by
:func:`core.fillet.apply_fillet` — one undo step.
"""
from __future__ import annotations

import math
import sys

from ingetrazo_core import MeshStore, Vec3 as QVector3D

from core.fillet import (CIRCULAR, MIN_ANGLE_DEG, FilletPlan,
                         _exponent, _fmt, _unit)
from core.i18n import tr
from core.mesh import edge_flags


def _face_curve(origin, ua, ub, r, segments):
    points = MeshStore.bevel_face_curve(origin.toTuple(), ua.toTuple(), ub.toTuple(), r, segments)
    return [QVector3D(*p) for p in points]


def plan_vertex_bevel(mesh, vertices, size: float, segments: int = 8,
                      profile: float = CIRCULAR):
    """Plan bevelling ``vertices`` (of ``mesh``) by ``size`` along each of
    their edges. Returns a :class:`FilletPlan`, or a message (str) saying
    why it cannot be done."""
    verts = list(dict.fromkeys(vertices))
    if not verts:
        return tr("Pick a corner to bevel.")
    if size <= 0:
        return tr("The radius must be positive.")
    if not 0.0 < profile < 1.0:
        return tr("The profile must be between 0 and 1.")
    segments = max(int(segments), 1)
    from core.store_mesh import _hypot
    store = getattr(mesh, "_store", None)
    if store is not None and hasattr(store, "plan_vertex_bevel"):
        from core.fillet import _store_round_plan
        hypot = _hypot()
        if hypot:
            return _store_round_plan(
                mesh, FilletPlan(size, segments, profile, "offset", "round"),
                store.plan_vertex_bevel, [v.key for v in verts], size,
                segments, profile, hypot, vertices=True)
    r = _exponent(profile)
    chosen = {v.key for v in verts}
    plan = FilletPlan(size, segments, profile, "offset", "round")

    # The cut point on each (corner, edge) and the edge vector to it.
    cut: dict[tuple[int, int], QVector3D] = {}
    for v in verts:
        if not v.edges or not v.faces():
            return tr("Only a corner of a face can be bevelled.")
        for e in v.edges:
            if not e.faces:
                return tr("The corner at {p} has an edge with no face.",
                          p=_fmt(v.position))
            if len(e.faces) > 2:
                return tr("The corner at {p} has too many faces to round.",
                          p=_fmt(v.position))
            w = e.other(v)
            need = size * (2.0 if w.key in chosen else 1.0)
            if need >= e.length() - 1e-9:
                return tr("The size is too large for the edge at {p}.",
                          p=_fmt(v.position))
            d = _unit(w.position - v.position)
            if d is None:
                return tr("A zero-length edge cannot be rounded.")
            cut[(v.key, e.key)] = d * size

    def edge_between(a, b):
        return mesh.find_edge(a, b)

    # Each face's curve at each chosen corner, in the face's loop order.
    curves: dict[tuple[int, int], list] = {}
    touched: dict[int, object] = {}
    for v in verts:
        for f in v.faces():
            touched[f.key] = f
            for loop in [f.loop] + list(f.hole_loops):
                m = len(loop)
                for i, x in enumerate(loop):
                    if x != v:
                        continue
                    ep = edge_between(loop[i - 1], v)
                    en = edge_between(v, loop[(i + 1) % m])
                    if ep is None or en is None:
                        return tr("Could not read the faces around the edge.")
                    ua, ub = cut[(v.key, ep.key)], cut[(v.key, en.key)]
                    cos_a = QVector3D.dotProduct(ua, ub) / (size * size)
                    if cos_a < -math.cos(math.radians(MIN_ANGLE_DEG)):
                        return tr("The edges at {p} are too flat to bevel.",
                                  p=_fmt(v.position))
                    curves[(f.key, v.key, ep.key)] = _face_curve(
                        v.position, ua, ub, r, segments)

    # Rebuild every face that loses a corner.
    def rebuilt_loop(face, loop):
        out = []
        m = len(loop)
        for i, x in enumerate(loop):
            if x.key not in chosen:
                out.append(QVector3D(x.position))
                continue
            ep = edge_between(loop[i - 1], x)
            out.extend(QVector3D(p) for p in curves[(face.key, x.key, ep.key)])
        clean = []
        for p in out:
            if not clean or (p - clean[-1]).length() > 1e-7:
                clean.append(p)
        if len(clean) > 1 and (clean[0] - clean[-1]).length() <= 1e-7:
            clean.pop()
        return clean

    attrs = None
    for f in touched.values():
        loop = rebuilt_loop(f, f.loop)
        holes = [rebuilt_loop(f, h) for h in f.hole_loops]
        if len(loop) < 3 or any(len(h) < 3 for h in holes):
            return tr("A face would collapse — the radius is too large.")
        plan.removed_faces.append(f)
        plan.rebuilt.append((loop, holes, dict(f.attrs), bool(f.interior)))
        keep = {k: f.attrs[k] for k in ("color", "texture", "mat", "opacity", "pbr")
                if k in (f.attrs or {})}
        attrs = keep if attrs is None else (attrs if attrs == keep else {})
    plan.face_attrs["strip"] = attrs or {}

    # Close each solid corner.
    for v in verts:
        faces = list(v.faces())
        closed = all(len(e.faces) == 2 for e in v.edges)
        face_curves = [pts for key, pts in curves.items() if key[1] == v.key]
        if not closed:
            for pts in face_curves:
                if segments > 1:
                    plan.curves.append([QVector3D(p) for p in pts])
            continue
        if len(v.edges) < 3:
            return tr("Nothing to bevel at {p}.", p=_fmt(v.position))
        out = QVector3D()
        for f in faces:
            out += f.normal()
        try:
            patches, soft = MeshStore.bevel_corner(
                v.position.toTuple(), [cut[(v.key, e.key)].toTuple() for e in v.edges],
                [[p.toTuple() for p in row] for row in face_curves], out.toTuple(),
                segments, r, _hypot(), sys.version_info >= (3, 12))
        except ValueError as exc:
            return tr(str(exc), p=_fmt(v.position))
        plan.patches.extend([QVector3D(*p) for p in row] for row in patches)
        plan.soft.extend((QVector3D(*a), QVector3D(*b)) for a, b in soft)

    # Old edges go; the shortened ones keep their flags.
    seen = set()
    for v in verts:
        for e in v.edges:
            if e.key in seen:
                continue
            seen.add(e.key)
            plan.removed_edges.append(e)
            a, b = e.v0, e.v1
            pa = (a.position + cut[(a.key, e.key)] if a.key in chosen
                  else QVector3D(a.position))
            pb = (b.position + cut[(b.key, e.key)] if b.key in chosen
                  else QVector3D(b.position))
            plan.flags.append((pa, pb, edge_flags(e)))
    return plan
