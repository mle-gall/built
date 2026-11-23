# SPDX-License-Identifier: GPL-3.0-or-later
"""Push decisions retained until the default document suite completes."""
from __future__ import annotations
from ingetrazo_core import Vec3 as QVector3D
from core import push_scans
from core.topology import _key, loop_inside_face
from core.history import subtract_loop_from_face

class _Fingerprint:
    """Closed-solid guard for an object mesh without a store snapshot."""
    def __init__(self, mesh):
        self.counts = len(mesh.faces), len(mesh.edges)
        self.positions = [_key(v.position) for v in mesh.vertices]

    def same_now(self, mesh):
        return (self.counts == (len(mesh.faces), len(mesh.edges))
                and sorted(self.positions) == sorted(_key(v.position) for v in mesh.vertices))




def guarded_push(mesh, apply, check_motion, closed, flat, store_of):
    guard = (mesh.capture_state()
             if closed(mesh) and not flat(mesh) else None)
    store_guard = getattr(guard, "snap", None)
    fp_before = _Fingerprint(mesh) if guard is not None and store_guard is None else None
    apply()
    if guard is not None and not (closed(mesh) or flat(mesh)):
        mesh.restore_state(guard)
        return True
    if guard is not None and check_motion:
        from core.store_mesh import _store_of
        store = _store_of(mesh)
        if store_guard is not None and store is not None:
            unchanged = store.push_shape_matches(store_guard)
        elif fp_before is not None:
            unchanged = fp_before.same_now(mesh)
        else:
            # A store operation materialised its mesh during the push.
            unchanged = ((len(mesh.vertices), len(mesh.edges), len(mesh.faces)) == store_guard.counts()
                         and sorted(_key(store_guard.vertex_position(k)) for k in store_guard.keys("vertex"))
                         == sorted(_key(v.position) for v in mesh.vertices))
        if unchanged:
            return True

    return False


_RIM_SNAP = 5e-4
# How far apart a hand-drawn face's corners may stand, along its own normal,
# and still be read as one flat face meeting the opposite wall (#1076).
_OUT_OF_TRUE = 2e-3


def _onto_rim(p: QVector3D, face: Face) -> QVector3D:
    """``p`` laid on ``face``'s outline when it lands within ``_RIM_SNAP``
    of it: a corner of the outline first, else the nearest point of an
    outline edge. A door a hair out of true, pushed through its wall,
    reaches the far side with its sill a hundredth of a millimetre above
    the floor's edge: left there, the opening's foot made a sliver and an
    edge with three faces instead of meeting the floor (#1076)."""
    verts = face.vertices
    near = min(verts, key=lambda v: (v - p).lengthSquared())
    if (near - p).length() <= _RIM_SNAP:
        return QVector3D(near)
    best, best_d = p, _RIM_SNAP
    m = len(verts)
    for i in range(m):
        a, b = verts[i], verts[(i + 1) % m]
        ab = b - a
        ll = ab.lengthSquared()
        if ll < 1e-18:
            continue
        t = QVector3D.dotProduct(p - a, ab) / ll
        if not 0.0 < t < 1.0:
            continue
        q = a + ab * t
        d = (q - p).length()
        if d < best_d:
            best, best_d = q, d
    return best




def push_through_face(face, d, faces, mesh, near, inside, subtract):
    """If pushing ``face`` by ``d`` along its normal reaches a face parallel
    to it on the far side of the solid (the opposite wall), return
    ``(far_face, back_loop)`` — the face to punch and the opening projected
    onto it. Otherwise ``None`` (a blind recess)."""
    fn = face.normal().normalized()
    push = fn if d > 0 else -fn
    origin = face.centroid()
    best = None
    # The far face must overlap the box the push sweeps (the face and its
    # copy |d| along the push): only those reach the normal and the
    # containment tests — every face's Newell normal was computed before,
    # on every push (09-10). Grown far beyond the 1e-4 tolerances.
    swept = list(face.vertices) + [v + push * abs(d) for v in face.vertices]
    sx = [p.x() for p in swept]; sy = [p.y() for p in swept]
    sz = [p.z() for p in swept]
    mg = 0.01 + 1e-6 * max(map(abs, sx + sy + sz))
    w0 = (min(sx) - mg, min(sy) - mg, min(sz) - mg)
    w1 = (max(sx) + mg, max(sy) + mg, max(sz) + mg)
    # The faces the window test below keeps, natively, in list order.
    near = (push_scans.faces_meeting(mesh, w0, w1)
            if mesh is not None else None)
    for g in (faces if near is None else near):
        if g == face:
            continue
        gv = g.vertices
        if (max(p.x() for p in gv) < w0[0] or min(p.x() for p in gv) > w1[0]
                or max(p.y() for p in gv) < w0[1]
                or min(p.y() for p in gv) > w1[1]
                or max(p.z() for p in gv) < w0[2]
                or min(p.z() for p in gv) > w1[2]):
            continue  # nowhere near what the push sweeps
        if abs(QVector3D.dotProduct(fn, g.normal().normalized())) < 0.999:
            continue  # not parallel
        dist = QVector3D.dotProduct(g.centroid() - origin, push)
        # How far each corner has to go to meet the far plane. A door a
        # fraction of a millimetre out of true reaches it with three
        # corners while its centroid is still short: pushed the wall's
        # thickness, it stopped at the centroid's 1e-4 and left its cap
        # on the far wall (#1076). Within _OUT_OF_TRUE the push meets
        # the face as soon as any corner does.
        # Measured to the far PLANE along the push: projected on the
        # push, the way to the far face's centroid also counted how far
        # to the side it lies, tenths of a millimetre on a door 1 mm out
        # of true, and the face read as beyond the push (#1094).
        toward = QVector3D.dotProduct(push, g.normal().normalized())
        reach = [QVector3D.dotProduct(g.centroid() - v,
                                      g.normal().normalized()) / toward
                 for v in face.vertices]
        lo, hi = min(reach), max(reach)
        if hi - lo > _OUT_OF_TRUE:
            lo = hi = dist
        if dist <= 1e-4 or lo > abs(d) + 1e-4:
            continue  # coplanar / behind / farther than the push reaches
        # Laid ON the far face's plane, not just carried ``dist`` along:
        # a hand-drawn door 0.2 mm out of true (one corner off its
        # plane) arrives that far off the opposite wall, outside every
        # 1e-4 test below -- no hole, and the cap left almost flush on
        # the wall wrecked the solid (#1076).
        gn = g.normal().normalized()
        gc = g.centroid()
        back_loop = []
        for v in face.vertices:
            p = v + push * dist
            back_loop.append(_onto_rim(
                p - gn * QVector3D.dotProduct(p - gc, gn), g))
        if loop_inside_face(g, back_loop):
            if best is None or dist < best[0]:
                best = (dist, g, back_loop, None)
            continue
        # Reaching the far face with the opening on its rim — the corner
        # piece left by a rounding arc pushed right through (Marco,
        # 2026-09-15: «debería eliminarme ese triángulo»), a door whose
        # sill is the floor. Not a hole: the far face is trimmed to what
        # is left of it (the arc becomes its outline) and the cap goes.
        # Pushed PAST the far face too, as a window's hole already was:
        # flush-only sent a door dragged a hair beyond the wall down the
        # blind-recess path, which poked the cap out the back and left
        # edges with three faces (#1076). Past it only where the push
        # leaves the material through that face (it faces along the
        # push): a face looking back at the push belongs to something
        # in front, which a push past it must not bite.
        if (not lo - 1e-4 <= abs(d) <= hi + 1e-4
                and QVector3D.dotProduct(gn, push) <= 0):
            continue
        remainder = subtract_loop_from_face(g, back_loop)
        if remainder is not None and (best is None or dist < best[0]):
            best = (dist, g, back_loop, remainder)
    if best is None:
        return None
    return best[1], best[2], best[3]



def translate_prism_cap(prism, keep, crosses):
    return prism and not keep and not crosses()


def push_region(base, base_holes, keep, entry_closed, classify, through, component_closed, flat):
    kinds = [classify(a, base[(i+1) % len(base)]) for i, a in enumerate(base)]
    backed = all(k != "free" for k in kinds)
    attached = backed and not keep
    far = through() if attached else None
    sheet = (not entry_closed and all(k == "coplanar" for k in kinds)
             and all(classify(a, h[(i+1) % len(h)]) == "coplanar"
                     for h in base_holes for i, a in enumerate(h))
             and not component_closed())
    return backed, attached, sheet, not flat() and not sheet, far
