# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Hidden-line removal for the sheet composer (C4, docs/composer-plan.md).

Given the scene and a parallel camera, produce the VISIBLE portions of the
model's edges as true 2D vector segments — what a drafter would ink. The
math is EXACT, not sampled: along each projected edge, the region occluded
by one triangle is the intersection of four linear inequalities in the
edge parameter t (three half-planes of the triangle's convex projection
plus the depth comparison, all linear under a parallel projection), so
every (edge, triangle) pair contributes one exact interval; visibility is
[0,1] minus their union.

Self-occlusion needs no adjacency bookkeeping: an edge lies in the plane
of each face it bounds, so the depth difference e(t) is ~0 across the
whole edge and never beats the epsilon.

Curved surfaces: a soft edge is included only when its faces straddle the
view (one toward the camera, one away) — the same silhouette rule the
viewport draws with; soft edges with a single face are open boundaries
and always profile.

Everything here is NumPy over camera-space arrays; Qt types appear only
while reading the scene.
"""
from __future__ import annotations

import math

import numpy as np

import ingetrazo_core as NATIVE

from core import progress as _progress

# ── Camera basis ────────────────────────────────────────────────────────────

def camera_basis(camera):
    """Right/up/forward unit vectors + eye, as NumPy rows, from an
    OrbitCamera (same derivation as its lookAt)."""
    eye = camera.eye()
    e = np.array([eye.x(), eye.y(), eye.z()], dtype=np.float64)
    t = np.array([camera.target.x(), camera.target.y(),
                  camera.target.z()], dtype=np.float64)
    up_v = camera.up_vector() if hasattr(camera, "up_vector") else camera.up
    up = np.array([up_v.x(), up_v.y(), up_v.z()],
                  dtype=np.float64)
    f = t - e
    f /= max(np.linalg.norm(f), 1e-12)
    r = np.cross(f, up)
    r /= max(np.linalg.norm(r), 1e-12)
    u = np.cross(r, f)
    return e, r, u, f


def _to_cam(points: np.ndarray, eye, right, up, fwd) -> np.ndarray:
    """World (N,3) → camera space (N,3): x right, y up, z = depth AWAY
    from the camera (bigger = farther)."""
    d = points - eye
    return np.stack([d @ right, d @ up, d @ fwd], axis=1)


# ── Scene collection ────────────────────────────────────────────────────────

def collect_geometry(scene, layers: bool = False):
    """World-space triangles (T,3,3) and edge segments (E,2,3) of every
    visible, non-billboard entity; instance groups are baked through their
    transform. Soft edges come back separately with their two face normals
    (silhouette test happens per view).

    ``layers=True`` also returns, for every hard and soft edge, the layer
    its line is inked with on a sheet — its own, else its object's (the
    insides of an object follow the object) — and the same for every
    triangle, whose layer its section cut is inked and filled with:
    ``(tris, hard, soft, hard_layers, soft_layers, tri_layers)``."""
    from core.group import visible_placements, world_mesh
    from core.layers import DEFAULT_LAYER, layer_of

    tris: list = []
    hard: list = []
    soft: list = []          # (p0, p1, n_a, n_b) — n_b None for boundaries
    hard_l: list = []
    soft_l: list = []
    tri_l: list = []

    def eat_mesh(mesh, visible=lambda _e: True, owner=DEFAULT_LAYER):
        for face in mesh.faces:
            _progress.tick()
            if not visible(face):
                continue
            own = layer_of(face)
            tag = own if own != DEFAULT_LAYER else owner
            for a, b, c in face.triangulate():
                tri_l.append(tag)
                tris.append(((a.x(), a.y(), a.z()),
                             (b.x(), b.y(), b.z()),
                             (c.x(), c.y(), c.z())))
        for e in mesh.edges:
            if not visible(e):
                continue
            p0 = (e.v0.position.x(), e.v0.position.y(), e.v0.position.z())
            p1 = (e.v1.position.x(), e.v1.position.y(), e.v1.position.z())
            if getattr(e, "hidden", False):
                continue
            own = layer_of(e)
            tag = own if own != DEFAULT_LAYER else owner
            if getattr(e, "soft", False):
                fs = [f for f in e.faces if visible(f)]
                if not fs:
                    continue
                na = fs[0].normal()
                nb = fs[1].normal() if len(fs) > 1 else None
                soft.append((p0, p1,
                             (na.x(), na.y(), na.z()),
                             None if nb is None
                             else (nb.x(), nb.y(), nb.z())))
                soft_l.append(tag)
            else:
                hard.append((p0, p1))
                hard_l.append(tag)

    eat_mesh(scene.loose_mesh, visible=scene.entity_visible)
    for g in scene.groups:
        if not scene.entity_visible(g) or getattr(g, "billboard", False):
            continue
        if g.children:
            mesh = world_mesh(g, visible_placements(scene, g))
        else:
            mesh = world_mesh(g) if g.xform is not None else g.mesh
        # A face hidden inside a group (Edit ▸ Hide) leaves the group's
        # chunk in the viewport; it leaves the sheet too.
        eat_mesh(mesh, visible=lambda e: not scene.entity_hidden(e),
                 owner=layer_of(g))
    if layers:
        return tris, hard, soft, hard_l, soft_l, tri_l
    return tris, hard, soft


def collect_circles(scene) -> list:
    """Every circle and arc the model carries, as ``(centre, radius,
    normal)`` in world space — the curves :func:`core.snap.curve_centers_of_face`
    reads off each visible face's boundary (a drawn circle, a rounded
    corner, a hole; an imported polyline by its shape), placements taken
    through their transform. It is what a sheet's radius / diameter tool
    recognises when the drafter clicks ON the arc, the way AutoCAD's
    DIMRADIUS asks for the arc and never for a centre nobody can see
    (Marco, 2026-09-20: «como que no reconoce el centro de un círculo»).
    The same curve on two faces (a hole and the pipe through it) is
    reported once."""
    from core.group import visible_placements
    from core.snap import curve_centers_of_face

    out: list = []
    seen: set = set()

    def eat(mesh, xform=None, visible=lambda _f: True):
        for face in mesh.faces:
            if not visible(face):
                continue
            n = face.normal()
            if xform is not None:
                n = xform.mapVector(n)
                if n.length() < 1e-9:
                    continue
                n.normalize()
            for c, r, _key in curve_centers_of_face(face, xform):
                key = (round(c.x(), 4), round(c.y(), 4), round(c.z(), 4),
                       round(r, 4), round(abs(n.x()), 3),
                       round(abs(n.y()), 3), round(abs(n.z()), 3))
                if key in seen:
                    continue
                seen.add(key)
                out.append((c, r, n))

    eat(scene.loose_mesh, visible=scene.entity_visible)
    for g in scene.groups:
        if not scene.entity_visible(g) or getattr(g, "billboard", False):
            continue
        for placed, m in visible_placements(scene, g):
            eat(placed.mesh, m)
    return out


def clip_to_section(tris, hard, soft, plane, split_cuts: bool = False):
    """Clip collected geometry to the KEPT side of the active section plane
    (S5 — the point of the sections track): triangles are cut, edges are
    shortened, and the plane∩triangle intersection segments join the hard
    edges — the sheet composer's hidden-line pass then draws real plans and
    cross-sections. ``plane`` is a core.section.SectionPlane; the hidden
    side is where ``plane.side(p) > 0``. Geometry kept whole comes back as
    the objects given.

    With ``split_cuts`` the chords come back as a fourth list instead of
    joining the hard edges — the drawing that inks them thicker and fills
    the poché needs to know which lines are the cut. In Rust
    (``rust/ingetrazo_core/src/section_cut.rs``)."""
    from ingetrazo_core import section_clip
    out = section_clip(tris, hard, soft, plane, bool(split_cuts))
    return tuple(out)


def solid_depth(ring, tris, normal) -> float | None:
    """How far the solid around section ring ``ring`` ((M, 3) world) runs
    through the plane, square to it: from a point inside the ring, the
    nearest of ``tris`` ((T, 3, 3), the solid's triangles) along
    ``normal`` and the nearest against it, added. ``None`` when a ray finds
    nothing (no point inside, an open solid)."""
    ring = np.asarray(ring, dtype=np.float64)
    T = np.asarray(tris, dtype=np.float64).reshape(-1, 3, 3)
    if len(ring) < 3 or not len(T):
        return None
    n = np.asarray(normal, dtype=np.float64)
    n = n / (np.linalg.norm(n) or 1.0)
    # A point inside: the middle of the ring's longest chord through its
    # centre would do for a convex ring; the centre itself for the usual
    # rectangle of a beam or a stud.
    origin = ring.mean(axis=0)
    v0 = T[:, 0]
    e1 = T[:, 1] - v0
    e2 = T[:, 2] - v0
    reach = []
    for d in (n, -n):
        # Moller-Trumbore, every triangle at once.
        pv = np.cross(d, e2)
        det = np.einsum("ij,ij->i", e1, pv)
        ok = np.abs(det) > 1e-12
        inv = np.where(ok, 1.0 / np.where(ok, det, 1.0), 0.0)
        tv = origin - v0
        u = np.einsum("ij,ij->i", tv, pv) * inv
        qv = np.cross(tv, e1)
        v = (qv @ d) * inv
        t = np.einsum("ij,ij->i", e2, qv) * inv
        hit = ok & (u >= -1e-9) & (v >= -1e-9) & (u + v <= 1 + 1e-9) \
            & (t > 1e-9)
        if not hit.any():
            return None
        reach.append(float(t[hit].min()))
    return reach[0] + reach[1]


def section_loops(cut_edges, tol: float | None = None,
                  members: list | None = None) -> list:
    """Chain the plane∩triangle chords of a section cut into CLOSED loops —
    the outlines of the solids the plane slices through, i.e. the areas a
    drafter fills with poché. Returns a list of (M, 3) arrays in world
    space, each a closed ring (first point not repeated).

    Endpoints closer than ``tol`` (default: a ten-millionth of the cut's
    span, at least 1e-9) are one. A watertight solid triangulated
    conformingly yields chords whose endpoints meet two by two, so walking
    the adjacency closes every ring; a chain that dead-ends (an open
    surface: a lone wall face, a roof without walls) is dropped rather than
    filled. Nested rings (a hollow wall, a pipe) are the caller's even-odd
    fill rule.

    ``members``, a list handed in, receives for each ring returned the
    rows of ``cut_edges`` it was chained from. In Rust
    (``rust/ingetrazo_core/src/section_cut.rs``)."""
    if not len(cut_edges):
        return []
    from ingetrazo_core import section_loops as native
    return native(cut_edges, None if tol is None else float(tol), members)


def closing_tags(cut_edges, cut_tags) -> list:
    """``cut_tags`` with each chord given the layer of the ring it closes.

    Rings are chained one layer at a time, so that a wall's cut takes the
    wall's fill and the door's beside it its own. But a solid is not always
    of one layer — a wall with one face moved to another — and then neither
    layer's chords close by themselves: the wall lost its fill altogether.
    The chords that close no ring within their own layer are chained
    together, whatever their layers, and each ring found that way goes to
    the layer most of its outline belongs to."""
    tags = list(cut_tags)
    if len(set(tags)) < 2:
        return tags
    E = np.asarray(cut_edges, dtype=np.float64).reshape(-1, 2, 3)
    by_tag: dict = {}
    for k, tag in enumerate(tags):
        by_tag.setdefault(tag, []).append(k)
    loose: list = []
    for tag, idx in by_tag.items():
        closed: list = []
        section_loops(E[idx], members=closed)
        inside = {j for ring in closed for j in ring}
        loose.extend(k for j, k in enumerate(idx) if j not in inside)
    if not loose or len({tags[k] for k in loose}) < 2:
        return tags
    mixed: list = []
    section_loops(E[loose], members=mixed)
    for ring in mixed:
        length: dict = {}
        for j in ring:
            k = loose[j]
            length[tags[k]] = length.get(tags[k], 0.0) + float(
                np.linalg.norm(E[k, 1] - E[k, 0]))
        # The most outline; names break a tie so the answer is one.
        winner = max(sorted(length, key=str), key=lambda t: length[t])
        for j in ring:
            tags[loose[j]] = winner
    return tags


# ── The section cap: the poché covers ───────────────────────────────────────
#
# A section cut leaves the solid OPEN: the clipper throws the hidden half
# away and nothing closes the hole, so the hidden-line pass still sees
# straight through the wall it just sliced and inks whatever sits behind —
# the far end of the board, the next divider — right across the poché.
# «Tapa pero no tapa», said Rafael, and the fill mode made no difference:
# the fill is painted UNDER the lines.
#
# So the rings the composer fills are also an OCCLUDER, exactly the same
# region under exactly the same even-odd rule: whatever lies strictly
# behind the cut plane and inside the poché is inside solid material, and
# solid material is not see-through. The cut chords themselves sit ON the
# plane, so they are never their own victims and the outline always survives.


def cap_depth_affine(plane, eye, right, up, fwd):
    """``(alpha, beta, gamma)`` with the cut plane's camera depth given by
    ``alpha·x + beta·y + gamma`` over the camera plane — a parallel camera
    maps a plane to a plane. ``None`` when the cut is seen edge-on: its cap
    projects to a line and covers nothing."""
    from core.triangulate import plane_axes
    u, v = plane_axes(plane.normal)
    p = plane.point
    pts = np.array([[p.x(), p.y(), p.z()],
                    [p.x() + u.x(), p.y() + u.y(), p.z() + u.z()],
                    [p.x() + v.x(), p.y() + v.y(), p.z() + v.z()]],
                   dtype=np.float64)
    c = _to_cam(pts, eye, right, up, fwd)
    area2 = ((c[1, 0] - c[0, 0]) * (c[2, 1] - c[0, 1])
             - (c[1, 1] - c[0, 1]) * (c[2, 0] - c[0, 0]))
    if abs(area2) < 1e-9:
        return None
    m = np.array([[c[0, 0], c[0, 1], 1.0],
                  [c[1, 0], c[1, 1], 1.0],
                  [c[2, 0], c[2, 1], 1.0]])
    try:
        return np.linalg.solve(m, c[:, 2])
    except np.linalg.LinAlgError:
        return None


def cap_ring_edges(loops):
    """Every ring of the cap as one pair of (K, 2) endpoint arrays ``P→Q``.
    Stacking all the rings together IS the even-odd rule: a hollow wall's
    inner ring flips the parity back, so its hole stays see-through."""
    if not loops:
        return None, None
    P = np.concatenate([np.asarray(r, dtype=np.float64)[:, :2] for r in loops])
    Q = np.concatenate([np.roll(np.asarray(r, dtype=np.float64)[:, :2], -1,
                                axis=0) for r in loops])
    return P, Q


def _geometry_as_lists(tris, hard, soft, soft_n):
    """Array geometry → the tuple lists ``clip_to_section`` walks."""
    t = [tuple(map(tuple, tri)) for tri in np.asarray(tris).tolist()]
    h = [(tuple(a), tuple(b)) for a, b in np.asarray(hard).tolist()]
    s = []
    for (p0, p1), (na, nb) in zip(np.asarray(soft).tolist(),
                                  np.asarray(soft_n).tolist()):
        open_edge = any(x != x for x in nb)            # NaN = no 2nd face
        s.append((tuple(p0), tuple(p1), tuple(na),
                  None if open_edge else tuple(nb)))
    return t, h, s


#: Line classes of a drawing, per segment (``HlrDrawing.kinds``).
KIND_EDGE = 0        #: a plain edge between two visible faces
KIND_PROFILE = 1     #: silhouette / outline against the background (Profiles)
KIND_CUT = 2         #: the section plane slicing through a solid
KIND_HIDDEN = 3      #: the part of an edge something stands in front of (dashed)


class HlrDrawing:
    """What :func:`hlr_drawing` returns: the visible segments in the camera
    plane (``segs`` (N,4)), their world endpoints (``world`` (N,2,3)),
    a class per segment (``kinds`` (N,) int8 — KIND_EDGE / KIND_PROFILE /
    KIND_CUT) and the closed section-cut rings to fill (``loops``, a list
    of (M,2) arrays in the camera plane). ``layers`` — only when asked
    for — is the layer each segment is inked with ((N,) object array of
    names; a section-cut chord carries the layer of the face it cuts), and
    ``loop_layers`` the layer of each ring, in the order of ``loops``.
    ``loop_depths`` — only for the layers asked for — how far the solid of
    each ring runs through the section plane, square to it, from inside
    the ring (model units; ``None`` where it was not measured): a stud cut
    in a plan runs its whole height, a board cut along its length only its
    thickness."""

    __slots__ = ("segs", "world", "kinds", "loops", "layers", "styles",
                 "labels", "loop_layers", "loop_depths")

    def __init__(self, segs, world, kinds, loops, layers=None,
                 loop_layers=None, loop_depths=None):
        self.segs = segs
        self.world = world
        self.kinds = kinds
        self.loops = loops
        self.layers = layers
        self.loop_layers = loop_layers
        self.loop_depths = loop_depths
        #: ``{layer: (rgb, linetype, weight, cut weight)}`` a sheet fills
        #: in for the exporters (core.layers.line_styles); ``None`` until
        #: it does.
        self.styles = None
        #: ``{tag key: name}`` the exporters write tags under
        #: (core.layers.export_labels); ``None`` until set.
        self.labels = None

    def __len__(self) -> int:
        return len(self.segs)


def hlr_drawing(scene, camera, geometry=None, profiles: bool = True,
                fills: bool = True, caps: bool = True,
                hidden: bool = False, layers: bool = False,
                depth_layers=None) -> HlrDrawing:
    """The full line drawing of *scene* under *camera* (parallel): visible
    segments classified as edge / profile / cut, plus the section-cut
    rings to fill. :func:`hlr_view` is the segments-only view of this.

    Profiles: soft edges where the surface turns away from the
    eye (and open boundaries), and hard edges with the background on one
    side — tested in 2D a hair off the visible segment's midpoint, so a
    lone face's outline, a box's contour and the eave of a roof all come out
    thick while the edges between two lit faces stay thin. ``profiles=False``
    skips the test (every non-cut line is KIND_EDGE).

    Fills: the chords of the active section chained into closed rings
    (:func:`section_loops`), projected to the camera plane.

    Caps: those same rings also OCCLUDE — whatever lies strictly behind the
    cut plane and inside the filled region is inside solid material, and a
    poché that lets the far side of the board show through «tapa pero no
    tapa» (Rafael, 2026-09-16). The cut chords lie ON the plane, so the
    outline of the cut always survives. ``caps=False`` draws the open
    silhouette instead.

    Hidden lines (``hidden=True``, issue #81, @pacaeiro: «I expected that
    Hidden line will show hidden lines of the model as dashed lines —
    that's the standard»): the parts of the hard edges that something
    stands in front of come back too, as KIND_HIDDEN, for the sheet to ink
    dashed. What lies inside the material behind a section cut stays out,
    as on a drawn section; silhouettes and cut chords have no hidden part.

    ``geometry`` — optional pre-collected arrays ``(tris, hard, soft,
    soft_n)`` as ``Viewport.hlr_geometry()`` returns them (world space:
    tris (T,3,3), hard (E,2,3), soft (S,2,3), soft_n (S,2,3) with NaN for
    an open boundary's missing second normal). Skips the per-face Python
    walk of :func:`collect_geometry` — 5 s on a 106k-triangle fountain —
    and runs the silhouette rule vectorised.

    `layers=True` tags every segment with the layer it is inked with
    (`HlrDrawing.layers`) — what a sheet needs to draw each layer with its
    own colour, line type and weight. Its section cut is tagged too, by
    the layer of the face each chord cuts, and its rings are chained layer
    by layer (`HlrDrawing.loop_layers`): a wall's cut gets the wall's pen
    and fill, the door's beside it its own. With a tagged ``geometry`` —
    the seven arrays ``Viewport.hlr_geometry(tags=True)`` returns, the
    layers of the triangles, hard edges and soft edges after the four —
    nothing is walked; without one, it walks the model itself.
    ``depth_layers`` — layer names — measures, for each ring of those
    layers, how far its solid runs through the plane
    (``HlrDrawing.loop_depths``, :func:`solid_depth`): where timber is cut
    across its grain or along it."""
    soft_n = None
    htags = stags = ttags = None
    whole_tris = whole_tags = ()
    cut_tags: list = []
    if layers and geometry is not None and len(geometry) == 7:
        tris, hard, soft, soft_n, ttags, htags, stags = geometry
        ttags, htags, stags = list(ttags), list(htags), list(stags)
    elif layers:
        tris, hard, soft, htags, stags, ttags = collect_geometry(
            scene, layers=True)
    elif geometry is None:
        tris, hard, soft = collect_geometry(scene)
    else:
        tris, hard, soft, soft_n = geometry
    # Active section cut: the composer's sheets honour it — the
    # whole reason sections exist here (plans and cross-cuts on paper).
    sp = (scene.active_section()
          if getattr(scene, "show_section_cuts", True)
          and hasattr(scene, "active_section") else None)
    cut_edges: list = []
    if sp is not None:
        if soft_n is not None:          # the clipper walks tuple lists
            tris, hard, soft = _geometry_as_lists(tris, hard, soft, soft_n)
            soft_n = None
        if htags is None:
            tris, hard, soft, cut_edges = clip_to_section(
                tris, hard, soft, sp, split_cuts=True)
        else:
            # The solids whole, for how deep each cut runs (depth_layers).
            whole_tris, whole_tags = tris, ttags
            # Clipped layer by layer, so each piece keeps its tag.
            tris, cut_edges, cut_tags = _clip_tris_tagged(tris, ttags, sp)
            hard, htags = _clip_tagged(hard, htags, sp, soft=False)
            soft, stags = _clip_tagged(soft, stags, sp, soft=True)
    _e, _r, _u, fwd = camera_basis(camera)

    def empty_drawing():
        return HlrDrawing(np.empty((0, 4)), np.empty((0, 2, 3)),
                          np.empty((0,), dtype=np.int8), [])

    n_hard = len(hard)
    if soft_n is not None:
        # silhouette rule, vectorised: a soft edge is a profile where its
        # two faces face opposite ways, or where it bounds an open surface.
        hard = np.asarray(hard, dtype=np.float64).reshape(-1, 2, 3)
        if len(soft):
            fa = np.asarray(soft_n)[:, 0, :] @ fwd
            fb = np.asarray(soft_n)[:, 1, :] @ fwd
            mask = np.isnan(fb) | ((fa < 0.0) != (fb < 0.0))
            hard = np.concatenate(
                [hard, np.asarray(soft, dtype=np.float64)[mask]])
            if htags is not None:       # the silhouettes keep their layer
                htags = list(htags) + [stags[k] for k in
                                       np.flatnonzero(mask).tolist()]
        if not len(hard) and not len(cut_edges):
            return empty_drawing()
    else:
        # silhouette rule for soft edges
        for si, (p0, p1, na, nb) in enumerate(soft):
            fa = float(np.dot(np.asarray(na), fwd))
            if nb is None:
                hard.append((p0, p1))   # open boundary: always a profile
            else:
                fb = float(np.dot(np.asarray(nb), fwd))
                if (fa < 0.0) == (fb < 0.0):
                    continue
                hard.append((p0, p1))
            if htags is not None:
                htags.append(stags[si])
        if not hard and not cut_edges:
            return empty_drawing()
    # Row layout of E: [hard edges | soft silhouettes | cut chords] — the
    # class of a row follows from its index.
    n_sil = len(hard) - n_hard
    parts = [np.asarray(hard, dtype=np.float64).reshape(-1, 2, 3)]
    if len(cut_edges):
        parts.append(np.asarray(cut_edges, dtype=np.float64).reshape(-1, 2, 3))
    eye, right, up, fwd = camera_basis(camera)
    E = np.concatenate(parts) if len(parts) > 1 else parts[0]   # (E,2,3)
    A = _to_cam(E[:, 0, :], eye, right, up, fwd)
    B = _to_cam(E[:, 1, :], eye, right, up, fwd)
    T = np.asarray(tris, dtype=np.float64).reshape(-1, 3, 3)
    TC = _to_cam(T.reshape(-1, 3), eye, right, up, fwd).reshape(-1, 3, 3)
    # The section cap: the rings the composer fills also COVER (see above).
    loops: list = []
    loop_tags: list | None = [] if htags is not None else None
    loop_depths: list = []
    cap_P = cap_Q = cap_abg = None
    if len(cut_edges):
        if loop_tags is None:
            rings = [(ring, None) for ring in section_loops(cut_edges)]
        else:
            # Chained one layer at a time: each solid closes on its own, so
            # its ring takes its layer's fill.
            by_tag: dict = {}
            for edge, tag in zip(cut_edges,
                                 closing_tags(cut_edges, cut_tags)):
                by_tag.setdefault(tag, []).append(edge)
            rings = [(ring, tag) for tag, edges in by_tag.items()
                     for ring in section_loops(edges)]
        wanted = set(depth_layers or ()) if loop_tags is not None else set()
        by_layer_tris: dict = {}
        for ring, tag in rings:
            loops.append(_to_cam(ring, eye, right, up, fwd)[:, :2])
            if loop_tags is not None:
                loop_tags.append(tag)
            if wanted:
                if tag in wanted:
                    if tag not in by_layer_tris:
                        if isinstance(whole_tris, np.ndarray):
                            by_layer_tris[tag] = whole_tris[
                                np.asarray(whole_tags, dtype=object) == tag]
                        else:
                            by_layer_tris[tag] = [
                                t for t, g in zip(whole_tris, whole_tags)
                                if g == tag]
                    n = sp.normal
                    loop_depths.append(solid_depth(
                        ring, by_layer_tris[tag], (n.x(), n.y(), n.z())))
                else:
                    loop_depths.append(None)
        if loops and caps:
            cap_abg = cap_depth_affine(sp, eye, right, up, fwd)
        if cap_abg is not None:
            cap_P, cap_Q = cap_ring_edges(loops)
    # Spans, the hidden remainder, the cap, Profiles, the collinear merge
    # and the hidden-under-visible drop: one pass in Rust
    # (ingetrazo_core sheets::drawing, docs/rust-sheets.md §2).
    row_tags = (list(htags) + list(cut_tags)
                if htags is not None else None)
    tag_ids = None
    if row_tags is not None:
        ids: dict = {}
        tag_ids = np.array([ids.setdefault(t, len(ids)) for t in row_tags],
                           dtype=np.int64)
    segs, world, kinds, rows = NATIVE.hlr_lines(
        np.ascontiguousarray(A), np.ascontiguousarray(B),
        np.ascontiguousarray(E), n_hard, n_sil, np.ascontiguousarray(TC),
        bool(profiles), bool(hidden),
        None if cap_abg is None else np.ascontiguousarray(cap_P),
        None if cap_abg is None else np.ascontiguousarray(cap_Q),
        None if cap_abg is None else np.asarray(cap_abg, dtype=np.float64),
        tag_ids)
    tags = (np.asarray(row_tags, dtype=object)[rows].reshape(-1)
            if row_tags is not None else None)
    return HlrDrawing(segs, world, kinds, loops if fills else [], tags,
                      (loop_tags if fills else []) if loop_tags is not None
                      else None,
                      (loop_depths if fills and depth_layers else None))


def _clip_tris_tagged(tris, tags, plane):
    """`clip_to_section` for triangles that carry a layer tag each: the
    kept triangles, and the cut chords with the tag of the triangle each
    one crosses — clipped one tag at a time, like `_clip_tagged`."""
    by_tag: dict = {}
    for tri, tag in zip(tris, tags):
        by_tag.setdefault(tag, []).append(tri)
    out, cuts, cut_tags = [], [], []
    for tag, group in by_tag.items():
        kept, _h, _s, chords = clip_to_section(group, [], [], plane, True)
        out.extend(kept)
        cuts.extend(chords)
        cut_tags.extend([tag] * len(chords))
    return out, cuts, cut_tags


def _clip_tagged(items, tags, plane, soft: bool):
    """`clip_to_section` for edges that carry a layer tag each: clipped
    one tag at a time, so every surviving piece keeps its own."""
    by_tag: dict = {}
    for item, tag in zip(items, tags):
        by_tag.setdefault(tag, []).append(item)
    out, out_tags = [], []
    for tag, group in by_tag.items():
        if soft:
            _t, _h, kept, _c = clip_to_section([], [], group, plane, True)
        else:
            _t, kept, _s, _c = clip_to_section([], group, [], plane, True)
        out.extend(kept)
        out_tags.extend([tag] * len(kept))
    return out, out_tags


def hlr_view(scene, camera, return_world: bool = False, geometry=None):
    """Visible edge segments of *scene* under *camera* (parallel), as an
    (N, 4) array of (x0, y0, x1, y1) in CAMERA-PLANE coordinates (model
    units, origin at the camera axis). This is the drawing a drafter would
    ink; the composer maps it to paper millimetres, the DXF export writes
    it in model units.

    With ``return_world`` also return the same segments' endpoints in
    WORLD coordinates, (N, 2, 3) — the anchor data for dimensions that
    follow the model. Both arrays share row order.

    ``geometry`` — see :func:`hlr_drawing`, which this wraps (no line
    classes, no fills: the cheap call for snapping and the DXF bridge)."""
    d = hlr_drawing(scene, camera, geometry=geometry, profiles=False,
                    fills=False)
    if not return_world:
        return d.segs
    return d.segs, d.world


# ── Perspective ─────────────────────────────────────────────────────────────
# The hidden-line pass above is parallel. A perspective turns into a parallel
# view by the projective map (x, y, z) → (x/w, y/w, 1/w): lines stay lines
# and planes stay planes, and 1/w orders the depth. Clipped to the view
# pyramid first — a point beside or behind the eye has no picture.

#: The view pyramid in clip space, as rows ``a`` with the kept side a·p ≥ 0:
#: left, right, bottom, top, near.
_PYRAMID = np.array([[1, 0, 0, 1], [-1, 0, 0, 1], [0, 1, 0, 1],
                     [0, -1, 0, 1], [0, 0, 1, 1]], dtype=np.float64)


def _clip_polygon(poly):
    """Sutherland–Hodgman against the view pyramid, in clip space."""
    for a in _PYRAMID:
        out = []
        n = len(poly)
        for i in range(n):
            p, q = poly[i], poly[(i + 1) % n]
            dp, dq = float(a @ p), float(a @ q)
            if dp >= 0.0:
                out.append(p)
            if (dp >= 0.0) != (dq >= 0.0):
                out.append(p + (q - p) * (dp / (dp - dq)))
        poly = out
        if len(poly) < 3:
            return []
    return poly


def _clip_triangles(C):
    """(T,3,4) clip-space triangles → the parts inside the pyramid."""
    if not len(C):
        return C
    D = C @ _PYRAMID.T                                  # (T,3,5)
    inside = (D >= 0.0).all(axis=(1, 2))
    outside = (D < 0.0).all(axis=1).any(axis=1)
    parts = [C[inside]]
    for tri in C[~inside & ~outside]:
        poly = _clip_polygon(list(tri))
        for i in range(1, len(poly) - 1):
            parts.append(np.stack([poly[0], poly[i], poly[i + 1]])[None])
    return np.concatenate(parts)


def _clip_segments(A, B):
    """Clip-space segments (E,4) → the parts inside the pyramid."""
    if not len(A):
        return A, B
    t0 = np.zeros(len(A))
    t1 = np.ones(len(A))
    keep = np.ones(len(A), dtype=bool)
    for a in _PYRAMID:
        da, db = A @ a, B @ a
        keep &= ~((da < 0.0) & (db < 0.0))
        with np.errstate(divide="ignore", invalid="ignore"):
            t = da / (da - db)
        t0 = np.where(da < 0.0, np.maximum(t0, t), t0)
        t1 = np.where(db < 0.0, np.minimum(t1, t), t1)
    keep &= t0 < t1
    A, B, t0, t1 = A[keep], B[keep], t0[keep, None], t1[keep, None]
    d = B - A
    return A + d * t0, A + d * t1


def hlr_perspective(scene, camera, geometry=None) -> np.ndarray:
    """Visible edge segments of *scene* in *camera*'s PERSPECTIVE (the
    two-point one included), (N, 4) as (x0, y0, x1, y1) in metres on the
    picture plane through the target — what sits at the target's depth
    comes out true size — with the origin at the centre of the view. Only
    what the view shows: the drawing is cut at the window's edges.

    ``geometry`` as for :func:`hlr_drawing`. The section in force cuts the
    model; its poché does not hide what lies behind it here."""
    from core.camera import OrbitCamera

    soft_n = None
    if geometry is None:
        tris, hard, soft = collect_geometry(scene)
    else:
        tris, hard, soft, soft_n = geometry
    sp = (scene.active_section()
          if getattr(scene, "show_section_cuts", True)
          and hasattr(scene, "active_section") else None)
    cut: list = []
    if sp is not None:
        if soft_n is not None:
            tris, hard, soft = _geometry_as_lists(tris, hard, soft, soft_n)
            soft_n = None
        tris, hard, soft, cut = clip_to_section(tris, hard, soft, sp,
                                                split_cuts=True)
    if soft_n is None:                  # tuple lists → arrays
        soft_n = np.array([[na, nb if nb is not None else (np.nan,) * 3]
                           for _p0, _p1, na, nb in soft],
                          dtype=np.float64).reshape(-1, 2, 3)
        soft = np.array([[p0, p1] for p0, p1, _na, _nb in soft],
                        dtype=np.float64).reshape(-1, 2, 3)
    tris = np.asarray(tris, dtype=np.float64).reshape(-1, 3, 3)
    soft = np.asarray(soft, dtype=np.float64).reshape(-1, 2, 3)
    soft_n = np.asarray(soft_n, dtype=np.float64).reshape(-1, 2, 3)
    lines = [np.asarray(hard, dtype=np.float64).reshape(-1, 2, 3),
             np.asarray(cut, dtype=np.float64).reshape(-1, 2, 3)]
    if len(soft):
        # A soft edge is a profile where its two faces turn opposite ways
        # to the EYE — in perspective the sight line differs per edge.
        eye = camera.eye()
        e = np.array([eye.x(), eye.y(), eye.z()], dtype=np.float64)
        v = soft.mean(axis=1) - e
        fa = np.einsum("ij,ij->i", soft_n[:, 0], v)
        fb = np.einsum("ij,ij->i", soft_n[:, 1], v)
        lines.append(soft[np.isnan(fb) | ((fa < 0.0) != (fb < 0.0))])
    E = np.concatenate(lines)

    m = camera.projection_matrix() * camera.view_matrix()
    M = np.array(m.data(), dtype=np.float64).reshape(4, 4, order="F")

    def to_clip(p):
        return p @ M[:, :3].T + M[:, 3]

    t = camera.target
    w_t = float(to_clip(np.array([t.x(), t.y(), t.z()]))[3])
    if w_t <= 1e-9:
        return np.empty((0, 4))
    half_h = w_t * math.tan(math.radians(camera.fov_deg) / 2.0)
    scale = np.array([half_h * camera.aspect, half_h, w_t])

    def to_parallel(c):
        # (x/w, y/w) scaled to metres at the target's depth; w_t/w for the
        # depth, which grows toward the eye.
        w = c[..., 3:4]
        return np.concatenate([c[..., :2] / w, 1.0 / w], axis=-1) * scale

    T = _clip_triangles(to_clip(tris.reshape(-1, 3)).reshape(-1, 3, 4))
    A, B = _clip_segments(to_clip(E[:, 0]), to_clip(E[:, 1]))
    if not len(A):
        return np.empty((0, 4))
    T = to_parallel(T)
    L = np.stack([to_parallel(A), to_parallel(B)], axis=1)
    # A parallel camera looking down −Z onto that space: x right, y up, and
    # the depth growing away from it.
    top = OrbitCamera()
    top.perspective = False
    top.target = type(camera.target)(0.0, 0.0, 0.0)
    top.yaw, top.pitch = -math.pi / 2.0, math.pi / 2.0
    top.distance = float(max(T[..., 2].max() if len(T) else 0.0,
                             L[..., 2].max())) + 1.0

    class _NoSection:
        pass

    d = hlr_drawing(_NoSection(), top,
                    geometry=(T, L, np.empty((0, 2, 3)), np.empty((0, 2, 3))),
                    profiles=False, fills=False, caps=False)
    return d.segs
