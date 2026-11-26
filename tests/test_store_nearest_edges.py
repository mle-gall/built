# SPDX-License-Identifier: GPL-3.0-or-later
"""``MeshStore.nearest_edges`` (Select's loose-edge hover) walks a BVH over
the store's edges instead of projecting every edge on every mouse move. It
must name the edges a projection of every edge names, in the same order,
after edits (which set the touched edges aside instead of rebuilding the
tree), undo, and with the eye close to a long edge."""
import math
import random

import pytest
from built_core import Matrix4, Vec3 as V

from built_core.mesh import Mesh

W, H, RADIUS = 800.0, 600.0, 12.0


def _camera(eye, target, fov=45.0):
    view = Matrix4()
    view.lookAt(eye, target, V(0, 0, 1))
    proj = Matrix4()
    proj.perspective(fov, W / H, 0.001, 10000.0)
    return list((proj * view).data())


def _px(m, p):
    c = [p[0] * m[r] + p[1] * m[4 + r] + p[2] * m[8 + r] + m[12 + r] for r in range(4)]
    w = c[3]
    return c, w


def _pixel(c, w):
    return ((c[0] / w * .5 + .5) * W, (1 - (c[1] / w * .5 + .5)) * H)


def _reference(mesh, m, x, y):
    """Every edge projected, the part behind the eye cut away."""
    out = []
    for i, e in enumerate(mesh.edges):
        a, b = [e.a.x(), e.a.y(), e.a.z()], [e.b.x(), e.b.y(), e.b.z()]
        (ca, wa), (cb, wb) = _px(m, a), _px(m, b)
        if wa <= 0 and wb <= 0:
            continue
        if wa <= 0 or wb <= 0:
            front = max(wa, wb)
            t = (front * 1e-6 - wa) / (wb - wa)
            p = [a[k] + t * (b[k] - a[k]) for k in range(3)]
            cp, wp = _px(m, p)
            if wa <= 0:
                ca, wa = cp, wp
            else:
                cb, wb = cp, wp
        (ax, ay), (bx, by) = _pixel(ca, wa), _pixel(cb, wb)
        dx, dy = bx - ax, by - ay
        t = min(1.0, max(0.0, ((x - ax) * dx + (y - ay) * dy) / max(dx * dx + dy * dy, 1e-12)))
        d = math.hypot(ax + t * dx - x, ay + t * dy - y)
        if d < RADIUS:
            out.append((i, d))
    out.sort(key=lambda h: (h[1], h[0]))
    return out


def _grid(n=24):
    m = Mesh()
    for i in range(n):
        for j in range(n):
            m.add_face([V(i, j, 0), V(i + 1, j, 0), V(i + 1, j + 1, 0), V(i, j + 1, 0)])
    return m


def _check(mesh, cameras, points):
    store = mesh.__dict__["_store"]
    for m in cameras:
        for x, y in points:
            got = [(i, d) for i, d, _t in store.nearest_edges(m, W, H, x, y, RADIUS)]
            want = _reference(mesh, m, x, y)
            assert [i for i, _ in got] == [i for i, _ in want], (x, y)
            assert all(abs(a - b) < 1e-6 for (_, a), (_, b) in zip(got, want))


CAMERAS = [
    _camera(V(12, -20, 25), V(12, 12, 0)),
    _camera(V(3.3, -0.4, 0.3), V(3.3, 5.0, 0.0)),   # low, along the edges
    _camera(V(5.01, 4.0, 0.004), V(5.01, 6.0, 0.0)),  # millimetres off the grid
    # 5 cm over the grid, 2 cm beside the line x = 3, above its edge from
    # y = 6 to 7: that edge's near end is behind the eye.
    _camera(V(3.02, 6.2, 0.05), V(3.02, 9.0, 0.0)),
]
POINTS = [(400.0, 300.0), (380.0, 320.0), (10.0, 590.0), (650.0, 120.0), (354.0, 409.0)]


def test_the_tree_names_what_every_edge_projected_names():
    _check(_grid(), CAMERAS, POINTS)


def test_after_edits_and_undo_the_tree_still_agrees():
    mesh = _grid()
    _check(mesh, CAMERAS[:1], POINTS)          # the tree is built
    state = mesh.capture_state()
    rng = random.Random(7)
    for _ in range(5):
        v = mesh.vertices[rng.randrange(len(mesh.vertices))]
        p = v.position if hasattr(v, "position") else v.pos
        mesh.move_vertex(v, V(p.x() + .3, p.y() - .2, p.z() + .5))
    mesh.add_face([V(30, 0, 0), V(31, 0, 0), V(31, 1, 0)])
    mesh.add_edge(V(2, 2, 0), V(9, 13, 2))
    mesh.remove_face(mesh.faces[17])
    _check(mesh, CAMERAS, POINTS)
    mesh.restore_state(state)
    _check(mesh, CAMERAS, POINTS)


def test_many_edits_rebuild_the_tree():
    mesh = _grid(12)
    _check(mesh, CAMERAS[:1], POINTS)
    for i in range(80):
        for j in range(70):
            mesh.add_edge(V(40 + i, j, 0), V(40 + i, j + 1, 0))
    _check(mesh, CAMERAS[:1], POINTS)
