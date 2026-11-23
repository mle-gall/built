# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Follow Me sweep: extrude a profile face along an edge path.

The discrete prism-miter construction: at every path vertex a *joint plane*
is placed (normal = bisector of the incoming and outgoing directions; at open
ends, the segment direction itself). The profile's ring at each station is
obtained by sliding the previous ring parallel to the incoming segment onto
the joint plane — the exact miter, so square corners join without pinching.
Closed paths connect the last span straight back to the first ring, welding
the loop with zero seam.

Headless engine API (the AI-native action layer): the tool is a thin click
shell over :func:`sweep_profile`.
"""
from __future__ import annotations

from copy import deepcopy

from ingetrazo_core import Vec3 as QVector3D

from core import doc_model
from core.history import run_stitch
from core.topology import _key

#: Dihedral cosine above which a sweep seam reads as a *curve* facet and is
#: softened (hidden) — same rule as Push/Pull's cylinder seams.
_CURVE_FACET_COS = 0.85


def order_path_edges(edges):
    """Chain the selected edges into an ordered polyline.

    Returns ``(points, closed)`` or ``None`` when the edges do not form one
    simple chain (branching, disjoint pieces)."""
    edges = list(edges)
    if not edges:
        return None
    adj: dict = {}
    for e in edges:
        adj.setdefault(e.v0, []).append(e.v1)
        adj.setdefault(e.v1, []).append(e.v0)
    if any(len(nbrs) > 2 for nbrs in adj.values()):
        return None                                  # branching
    ends = [v for v, nbrs in adj.items() if len(nbrs) == 1]
    if len(ends) not in (0, 2):
        return None
    closed = not ends
    start = ends[0] if ends else edges[0].v0
    pts = [QVector3D(start.position)]
    prev, cur = None, start
    for _ in range(len(edges)):
        nxt = next((v for v in adj[cur] if v != prev), None)
        if nxt is None:
            return None
        pts.append(QVector3D(nxt.position))
        prev, cur = cur, nxt
    if closed:
        if cur != start:
            return None                              # disjoint loops
        pts.pop()                                    # implicit wrap
    elif len(pts) != len(edges) + 1:
        return None                                  # disjoint chains
    return pts, closed


def _project_ring(points, direction, plane_pt, plane_n):
    """Slide each point parallel to ``direction`` onto the joint plane."""
    denom_dir = QVector3D.dotProduct(direction, plane_n)
    if abs(denom_dir) < 1e-9:
        return None                                  # segment ⟂ joint plane
    out = []
    for p in points:
        t = QVector3D.dotProduct(plane_pt - p, plane_n) / denom_dir
        out.append(p + direction * t)
    return out


def _wall(quad):
    """A wall quad without its repeated corners, or ``None`` when fewer than
    three distinct corners remain."""
    out = []
    for p in quad:
        if not out or (p - out[-1]).length() >= 1e-9:
            out.append(p)
    if len(out) > 1 and (out[0] - out[-1]).length() < 1e-9:
        out.pop()
    return out if len(out) >= 3 else None


def _clip_half(loop, keep_tol):
    """The part of a closed ``loop`` (points as (h, x, y) in the revolution
    frame) with x >= 0 — Sutherland–Hodgman against the axis. Points on the
    cut land exactly ON the axis (x = y = 0), so every station repeats them
    bit for bit and the pole quads collapse to triangles."""
    out = []
    m = len(loop)
    for i in range(m):
        cur, nxt = loop[i], loop[(i + 1) % m]
        cin, nin = cur[1] >= -keep_tol, nxt[1] >= -keep_tol
        if cin:
            out.append((cur[0], max(cur[1], 0.0), cur[2])
                       if cur[1] > keep_tol else (cur[0], 0.0, 0.0))
        if cin != nin:
            t = cur[1] / (cur[1] - nxt[1])
            out.append((cur[0] + (nxt[0] - cur[0]) * t, 0.0, 0.0))
    clean = []
    for q in out:
        if not clean or any(abs(q[k] - clean[-1][k]) > 1e-12 for k in range(3)):
            clean.append(q)
    while len(clean) > 1 and all(abs(clean[0][k] - clean[-1][k]) <= 1e-12
                                 for k in range(3)):
        clean.pop()
    return clean if len(clean) >= 3 else None


def _revolution_rings(face, path):
    """Rings for a profile turned about an AXIS (#125/#128), or ``None``.

    A closed, planar, circular path whose axis lies in the profile's plane
    is a lathe: the classic sphere is a circle swept along a circle that
    shares its centre. The mitre construction only reproduces that when the
    profile stands exactly on a path vertex — anywhere else it SLIDES the
    profile onto the first joint plane and squashes it (a «sphere» with
    radii 0.93–1.00) — and a full circle, crossing the axis, was swept
    twice over itself. Here each station is the profile ROTATED about the
    axis to that path vertex, and a profile that crosses the axis keeps the
    half on its own side (the other half is the same surface again)."""
    import math

    n = len(path)
    if n < 3:
        return None
    pts = [(p.x(), p.y(), p.z()) for p in path]
    c = tuple(sum(p[k] for p in pts) / n for k in range(3))
    # Newell normal of the path polygon: the axis.
    ax = [0.0, 0.0, 0.0]
    for i in range(n):
        a, b = pts[i], pts[(i + 1) % n]
        ax[0] += (a[1] - b[1]) * (a[2] + b[2])
        ax[1] += (a[2] - b[2]) * (a[0] + b[0])
        ax[2] += (a[0] - b[0]) * (a[1] + b[1])
    la = math.sqrt(sum(v * v for v in ax))
    if la < 1e-12:
        return None
    ax = [v / la for v in ax]
    rel = [tuple(p[k] - c[k] for k in range(3)) for p in pts]
    radii = [math.sqrt(sum(v * v for v in r)) for r in rel]
    rmean = sum(radii) / n
    if rmean < 1e-9:
        return None
    tol = max(1e-4, 1e-3 * rmean)
    dot = lambda u, v: u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
    cross = lambda u, v: (u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2],
                          u[0] * v[1] - u[1] * v[0])
    if any(abs(dot(r, ax)) > tol for r in rel):
        return None                                   # not planar
    if max(radii) - min(radii) > tol + 1e-2 * rmean:
        return None                                   # not a circle
    fn = face.normal()
    fn = (fn.x(), fn.y(), fn.z())
    lf = math.sqrt(dot(fn, fn))
    if lf < 1e-12:
        return None
    fn = tuple(v / lf for v in fn)
    if abs(dot(fn, ax)) > 1e-3:
        return None                                   # axis not in the plane
    v0 = face.vertices[0]
    if abs(dot((v0.x() - c[0], v0.y() - c[1], v0.z() - c[2]), fn)) > tol:
        return None                                   # plane misses the axis
    radial = cross(ax, fn)
    lr = math.sqrt(dot(radial, radial))
    radial = tuple(v / lr for v in radial)
    fc = face.centroid()
    if dot((fc.x() - c[0], fc.y() - c[1], fc.z() - c[2]), radial) < 0.0:
        radial = tuple(-v for v in radial)
    side = cross(ax, radial)               # completes the right-handed frame

    def local(p):
        r = (p.x() - c[0], p.y() - c[1], p.z() - c[2])
        return (dot(r, ax), dot(r, radial), dot(r, side))

    size = max(max(abs(q) for q in local(v)) for v in face.vertices) or 1.0
    keep_tol = max(1e-7, 1e-6 * size)
    outer = [local(v) for v in face.vertices]
    if min(q[1] for q in outer) < -keep_tol:
        outer = _clip_half(outer, keep_tol)
        if outer is None:
            return None
    else:
        outer = [(q[0], q[1], 0.0) if abs(q[1]) > keep_tol else (q[0], 0.0, 0.0)
                 for q in outer]
    holes = []
    for h in face.holes:
        hl = [local(v) for v in h]
        xs = [q[1] for q in hl]
        if min(xs) >= -keep_tol:
            holes.append(hl)
        elif max(xs) > keep_tol:
            return None               # a hole across the axis: not a lathe
    loops = [outer] + holes

    rings = []
    for r in rel:
        # Station angle of this path vertex, measured from the profile.
        ca, sa = dot(r, radial), dot(r, side)
        norm = math.hypot(ca, sa) or 1.0
        ca, sa = ca / norm, sa / norm
        ring = []
        for lp in loops:
            ring.append([QVector3D(*(
                c[k] + ax[k] * h + (radial[k] * ca + side[k] * sa) * x
                + (side[k] * ca - radial[k] * sa) * y for k in range(3)))
                for h, x, y in lp])
        rings.append(ring)
    dirs = []
    for i in range(n):
        d = path[(i + 1) % n] - path[i]
        if d.length() < 1e-9:
            return None
        dirs.append(d.normalized())
    return rings, dirs, n


def sweep_rings(face, path, closed):
    if not doc_model.ENABLED:
        return _reference_sweep_rings(face, path, closed)
    import sys
    from ingetrazo_core import MeshStore
    from core.store_mesh import _hypot, _xyz
    profile = [[_xyz(p) for p in ring] for ring in (face.vertices, *face.holes)]
    result = MeshStore.project_sweep(profile, [_xyz(p) for p in path], closed, _hypot(), sys.version_info >= (3, 12))
    if result is None:
        return None
    stations, directions = result
    return ([_sweep_vectors(station) for station in stations],
            [QVector3D(*d) for d in directions], len(directions))


def _sweep_vectors(rings):
    return [[QVector3D(*p) for p in ring] for ring in rings]


def _sweep_polygons(rings, closed, start_cap):
    from ingetrazo_core import MeshStore
    from core.store_mesh import _hypot
    rows = [[[p.toTuple() for p in loop] for loop in station] for station in rings]
    return (_sweep_vectors(face) for face in MeshStore.sweep_polygons(rows, closed, start_cap, _hypot()))


def _reference_sweep_rings(face, path, closed):
    """The profile's rings (outer loop + holes) at every path station.

    Returns ``(rings, dirs, spans)`` or ``None`` on degenerate input — a
    180° reversal in the path, a segment parallel to a joint plane, a
    zero-length span."""
    if closed:
        rev = _revolution_rings(face, path)
        if rev is not None:
            return rev
    dirs = []
    n = len(path)
    spans = n if closed else n - 1
    if spans < 1:
        return None
    for i in range(spans):
        d = path[(i + 1) % n] - path[i]
        if d.length() < 1e-9:
            return None
        dirs.append(d.normalized())

    def joint_normal(i):
        if closed:
            a, b = dirs[(i - 1) % spans], dirs[i % spans]
        elif i == 0:
            return dirs[0]
        elif i >= spans:
            return dirs[-1]
        else:
            a, b = dirs[i - 1], dirs[i]
        s = a + b
        if s.length() < 1e-9:
            return None                              # 180° reversal
        return s.normalized()

    loops = [[QVector3D(v) for v in face.vertices]]
    loops += [[QVector3D(v) for v in h] for h in face.holes]

    stations = n                                  # one ring per path vertex
    rings: list = []
    for i in range(stations):
        pn = joint_normal(i)
        if pn is None:
            return None
        anchor = path[i % n]
        if i == 0:
            ring = [_project_ring(lp, dirs[0], anchor, pn) for lp in loops]
        else:
            ring = [_project_ring(prev_lp, dirs[i - 1], anchor, pn)
                    for prev_lp in rings[-1]]
        if any(lp is None for lp in ring):
            return None
        rings.append(ring)
    return rings, dirs, spans


def _wall_reversals(rings):
    """Outer walls follow a ring facing the next station, hole walls the opposite."""
    reverse = []
    for i, (start, end) in enumerate(zip(rings[0], rings[1])):
        travel = sum((b - a for a, b in zip(start, end)), QVector3D())
        origin = start[0]
        area = sum((QVector3D.crossProduct(a - origin, b - origin)
                    for a, b in zip(start, start[1:] + start[:1])), QVector3D())
        backwards = QVector3D.dotProduct(area, travel) < 0
        reverse.append(backwards != (i > 0))
    return reverse


def sweep_preview_faces(face, path, closed):
    """The sweep as plain polygons for a live preview — the walls of every
    span and, on an open path, the far cap. No stitch, no orientation, no
    mesh: the shape while it is still moving under the cursor."""
    from core.geometry import Face          # the preview polygon (attrs kw)
    got = sweep_rings(face, path, closed)
    if got is None:
        return []
    rings, _dirs, spans = got
    attrs = dict(face.attrs) if face.attrs else None
    if doc_model.ENABLED:
        return [Face(outer, holes or None, attrs=attrs) for outer, *holes in _sweep_polygons(rings, closed, False)]
    stations = len(rings)
    out = []
    reverse = _wall_reversals(rings)
    for s in range(spans):
        r0 = rings[s]
        r1 = rings[(s + 1) % stations] if closed else rings[s + 1]
        for lp0, lp1, flip in zip(r0, r1, reverse):
            m = len(lp0)
            for j in range(m):
                a, b = lp0[j], lp0[(j + 1) % m]
                b2, a2 = lp1[(j + 1) % m], lp1[j]
                quad = _wall([a, b, b2, a2])
                if quad is not None:
                    if flip:
                        quad.reverse()
                    out.append(Face(quad, attrs=attrs))
    if not closed:
        cap = [ring[::-1] if flip else ring for ring, flip in zip(rings[-1], reverse)]
        out.append(Face(cap[0], cap[1:] or None, attrs=attrs))
    return out


# ---- Manual (dragged) paths — "click and drag along the path" -----------

def manual_path_start(face, edge, toward=None):
    """The first station(s) of a path dragged from the profile ``face`` over
    ``edge``. When the edge crosses the profile's plane the sweep starts
    at the crossing (the profile sits mid-edge); otherwise it starts at the
    endpoint nearest the profile. Returns ``(start_point | None,
    [Vertex, ...])``: the chain's vertices come after the start point."""
    c = face.centroid()
    n = face.normal().normalized()
    a, b = edge.v0, edge.v1
    da = QVector3D.dotProduct(a.position - c, n)
    db = QVector3D.dotProduct(b.position - c, n)
    if da * db < -1e-12:
        t = da / (da - db)
        p = a.position + (b.position - a.position) * t
        far = b
        if toward is not None and ((a.position - toward).length()
                                   < (b.position - toward).length()):
            far = a
        return p, [far]
    near, far = (a, b) if abs(da) <= abs(db) else (b, a)
    return None, [near, far]


def manual_path_extend(chain, edge, skip=None, limit=400) -> bool:
    """Grow — or shrink — a dragged path with the edge under the cursor.
    Returns True when the chain changed.

    - an edge touching the chain's end extends it (or backs up over the
      last segment when the cursor returns along it);
    - an edge lying earlier on the chain shrinks the path back to it;
    - anywhere else, the shortest run of connected edges bridges the gap,
      so a fast drag that skips segments of an arc still follows it.
    ``skip(edge)`` vetoes edges (those in the profile's plane)."""
    if not chain or (skip is not None and skip(edge)):
        return False
    last = chain[-1]
    v0, v1 = edge.v0, edge.v1
    ids = {v.key: i for i, v in enumerate(chain)}
    if last == v0 or last == v1:
        other = v1 if last == v0 else v0
        if len(chain) >= 2 and other == chain[-2]:
            chain.pop()                              # backing up
            return True
        if other.key in ids:
            if other == chain[0] and len(chain) >= 3:
                chain.append(other)                  # the loop closes
                return True
            return False
        chain.append(other)
        return True
    if v0.key in ids and v1.key in ids:
        del chain[max(ids[v0.key], ids[v1.key]) + 1:]   # back to that edge
        return True
    route = _route(last, (v0, v1), skip, ids, limit)
    if route is None:
        return False
    chain.extend(route)
    reached = route[-1]
    other = v1 if reached == v0 else v0
    if other.key not in ids and other != reached:
        chain.append(other)
    return True


def _route(start, targets, skip, blocked_ids, limit):
    """Shortest run of edges (breadth-first) from ``start`` to either of
    ``targets``, never through vertices already on the chain."""
    from collections import deque
    prev = {start.key: None}
    seen = {start.key}
    q = deque([start])
    hit = None
    steps = 0
    while q and steps < limit and hit is None:
        v = q.popleft()
        steps += 1
        for e in v.edges:
            if skip is not None and skip(e):
                continue
            w = e.v1 if e.v0 == v else e.v0
            if w.key in seen or (w.key in blocked_ids and w != start):
                continue
            seen.add(w.key)
            prev[w.key] = v
            if w == targets[0] or w == targets[1]:
                hit = w
                break
            q.append(w)
    if hit is None:
        return None
    route = []
    v = hit
    while v is not None and v != start:
        route.append(v)
        v = prev[v.key]
    route.reverse()
    return route


def orient_closed_path(pts, face):
    """A closed path as the sweep wants it: station 0 at the vertex nearest
    the profile, travelling the way whose FIRST segment stands closest to
    perpendicular to the profile (the profile is perpendicular to one
    segment at its corner; starting along the other collapses the first
    ring)."""
    pts = [QVector3D(p) for p in pts]
    if len(pts) < 3:
        return pts
    c = face.centroid()
    k = min(range(len(pts)), key=lambda i: (pts[i] - c).length())
    pts = pts[k:] + pts[:k]
    n = face.normal().normalized()
    fwd = pts[1] - pts[0]
    back = pts[-1] - pts[0]
    if fwd.length() > 1e-9 and back.length() > 1e-9:
        af = abs(QVector3D.dotProduct(fwd.normalized(), n))
        ab = abs(QVector3D.dotProduct(back.normalized(), n))
        if ab > af + 1e-9:
            pts = [pts[0]] + pts[1:][::-1]
    return pts


def _store_sweep(mesh, key, path, closed, h):
    import sys
    store = mesh._store
    from core.mesh import Mesh
    from core.store_mesh import _xyz
    points = [_xyz(p) for p in path]
    stations = store.sweep_stations(key, points, closed, h, sys.version_info >= (3, 12))
    if stations is None:
        return False
    before = store.snapshot()
    cap_holes = None
    if not closed and len(stations[0]) > 2:
        from core.store_mesh import _hole_filter
        cap_holes = [_hole_filter([list(reversed(r)) for r in stations[0][1:]]),
                     _hole_filter(stations[-1][1:])]
    from core.store_mesh import FaceView
    attrs = FaceView(mesh, key).attrs
    change = store.sweep_face(key, stations, closed, h, cap_holes)
    if attrs:
        for created in change.created["face"]:
            FaceView(mesh, created).attrs = deepcopy(attrs)
    mesh._dirty()
    seed = {_key(p) for station in stations for ring in station for p in ring}
    run_stitch(mesh, seed, None, coplanar_merge=False)
    runs, _ = store.finish_sweep(before, key, points, stations, closed, h)
    for run in runs:
        store.set_curve_run(run, Mesh.next_curve_id())
    mesh._dirty()
    return True


def build_sweep_face(mesh, key, path, closed, rejection_message="degenerate sweep path"):
    """Plan one snapshot-backed sweep from a live profile key."""
    from core.history import CommandRejected, SnapshotMutation
    from core.protocol.keys import elements
    from core.store_face_edit import StoreFaceMutation
    profile = elements(mesh, "face", [key])[key]

    def mutate(scene):
        if not sweep_profile(scene.mesh, profile, path, closed):
            raise CommandRejected(rejection_message)

    command = StoreFaceMutation if hasattr(mesh, "_store") else SnapshotMutation
    return command(mutate)


def sweep_profile(mesh, face, path, closed) -> bool:
    """Sweep ``face`` (with holes) along ``path``; mutates ``mesh`` in place.

    Returns ``False`` (mesh untouched) on degenerate input — a 180° reversal
    in the path, a segment parallel to a joint plane, a zero-length span."""
    from core.store_mesh import _hypot, _store_of
    store = _store_of(mesh)
    h = _hypot() if store is not None else None
    if h:
        if mesh.__dict__.get("_resident_edit"):
            return _store_sweep(mesh, face.key, path, closed, h)
        from core.store_face_edit import resident_edit
        before = mesh.capture_state()
        with resident_edit(mesh):
            try:
                return _store_sweep(mesh, face.key, path, closed, h)
            except Exception:
                mesh.restore_state(before)
                raise

    got = sweep_rings(face, path, closed)
    if got is None:
        return False
    rings, _dirs, spans = got
    n = len(path)
    stations = n                                  # one ring per path vertex

    # Build: walls per span per loop edge; the closed path's last span goes
    # straight back to ring 0 (exact weld, no seam).
    before_edges = set(mesh.edges)
    before_faces = {f.key for f in mesh.faces}
    profile_loops = [list(face.loop)] + [list(h) for h in face.hole_loops]
    # Which profile edges and path segments were curves (circles, arcs): the
    # caps and the rails the sweep builds from them become curves too.
    profile_curves = [
        [getattr(mesh.find_edge(lp[j], lp[(j + 1) % len(lp)]), "curve", None)
         for j in range(len(lp))] for lp in profile_loops]
    span_curves = [getattr(_edge_at(mesh, path[s], path[(s + 1) % n]),
                           "curve", None) for s in range(spans)]
    attrs = face.attrs
    mesh.remove_face(face)                           # profile is consumed
    if doc_model.ENABLED:
        for outer, *holes in _sweep_polygons(rings, closed, True):
            mesh.add_face(outer, holes or None).attrs = deepcopy(attrs)
    else:
        _reference_build_sweep(mesh, rings, spans, stations, closed, attrs)

    # The profile's own edges go with it: when station 0 is a mitre (the
    # profile at a corner of a closed path) they no longer bound anything
    # and would stay behind as loose lines.
    for lp in profile_loops:
        n_lp = len(lp)
        for i in range(n_lp):
            e = mesh.find_edge(lp[i], lp[(i + 1) % n_lp])
            if e is not None and not e.faces:
                mesh.remove_edge(e)
    seed = {_key(p) for ring in rings for lp in ring for p in lp}
    run_stitch(mesh, seed, None, coplanar_merge=False)
    # Soften the seams of a curved sweep (shallow dihedral between successive
    # spans) so a moulding around a circle reads smooth; real corners stay.
    # An edge that existed before counts too when the sweep built BOTH of its
    # faces: the path circle of a sphere lies on its equator and the profile
    # on a meridian, and left hard they cut the sphere into four surfaces
    # that had to be painted one by one.
    for e in mesh.edges:
        if e.soft or len(e.faces) != 2:
            continue
        if e in before_edges and any(f.key in before_faces for f in e.faces):
            continue
        d = QVector3D.dotProduct(e.faces[0].normal().normalized(),
                                 e.faces[1].normal().normalized())
        if _CURVE_FACET_COS < d < 0.99995:
            e.soft = True
    _tag_sweep_curves(mesh, rings, closed, spans, profile_curves, span_curves)
    return True


def _reference_build_sweep(mesh, rings, spans, stations, closed, attrs):
    reverse = _wall_reversals(rings)
    for s in range(spans):
        r0 = rings[s]
        r1 = rings[(s + 1) % stations] if closed else rings[s + 1]
        for lp0, lp1, flip in zip(r0, r1, reverse):
            m = len(lp0)
            for j in range(m):
                a, b = lp0[j], lp0[(j + 1) % m]
                b2, a2 = lp1[(j + 1) % m], lp1[j]
                # A span that doesn't move this edge adds nothing; one that
                # moves only one end (a point on a revolution axis) is a
                # triangle.
                quad = _wall([a, b, b2, a2])
                if quad is not None:
                    if flip:
                        quad.reverse()
                    mesh.add_face(quad).attrs = deepcopy(attrs)
    if not closed:
        for start, station in ((True, rings[0]), (False, rings[-1])):
            cap = [ring[::-1] if start != flip else ring
                   for ring, flip in zip(station, reverse)]
            mesh.add_face(cap[0], cap[1:] or None).attrs = deepcopy(attrs)


def _edge_at(mesh, p, q):
    """The mesh edge from point ``p`` to point ``q``, or ``None``."""
    va, vb = mesh.vertex_at(p), mesh.vertex_at(q)
    if va is None or vb is None or va == vb:
        return None
    return mesh.find_edge(va, vb)


def _tag_sweep_curves(mesh, rings, closed, spans, profile_curves,
                      span_curves) -> None:
    """Give the sweep's curves their identity back, so one click selects the
    whole circle: the end caps of a profile with arcs in it, and the rails
    that follow a curved stretch of the path from a profile corner. Edges
    that are already a curve, or soft (a smooth seam), are left alone."""
    from core.mesh import Mesh
    stations = len(rings)

    def tag(edges):
        edges = [e for e in edges
                 if e is not None and e.curve is None and not e.soft]
        if edges:
            cid = Mesh.next_curve_id()
            for e in edges:
                e.curve = cid

    if not closed:
        for ring in (rings[0], rings[-1]):
            for lp, curves in zip(ring, profile_curves):
                m = len(lp)
                runs: dict = {}
                for j in range(m):
                    if j < len(curves) and curves[j] is not None:
                        runs.setdefault(curves[j], []).append(
                            _edge_at(mesh, lp[j], lp[(j + 1) % m]))
                for edges in runs.values():
                    tag(edges)
    if not any(c is not None for c in span_curves):
        return
    # A closed path's curve may run through station 0: start where a run
    # starts, so it is not cut in two there.
    start = 0
    if closed:
        start = next((s for s in range(spans)
                      if span_curves[s] != span_curves[s - 1]), 0)
    for loop_i, lp in enumerate(rings[0]):
        for j in range(len(lp)):
            run, run_c = [], None
            for k in range(spans):
                s = (start + k) % spans
                c = span_curves[s]
                if c != run_c:
                    tag(run)
                    run, run_c = [], c
                if c is not None:
                    run.append(_edge_at(mesh, rings[s][loop_i][j],
                                        rings[(s + 1) % stations][loop_i][j]))
            tag(run)
