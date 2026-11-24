# SPDX-License-Identifier: GPL-3.0-or-later
"""Scalar materialised face edits preserve keys, radial multiplicity and retained edges."""
import hashlib
import json
import random

from ingetrazo_core import Vec3
from core.mesh import Mesh


def scalar_results(monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE", "0")
    rows = []
    for seed in range(32):
        rng = random.Random(seed)
        mesh = Mesh()
        def square(x, y, size):
            return [Vec3(x+dx*size,y+dy*size,0) for dx,dy in ((0,0),(1,0),(1,1),(0,1))]
        outer = square(0,0,6)
        if seed%2:
            outer.insert(1,outer[0])
        f=mesh.add_face(iter(outer),[square(1,1,3),square(2,2,.5)])
        other=mesh.add_face(square(6,0,6))
        edge=mesh.find_edge(f.loop[-2],f.loop[-3])
        if edge is not None: edge.soft=True
        ring=mesh.add_hole(f,square(4.5,2,1))
        def state():
            vs={v.key:i for i,v in enumerate(mesh.vertices)}
            fs={g.key:i for i,g in enumerate(mesh.faces)}
            return [[(vs[e.v0.key],vs[e.v1.key],[fs[g.key] for g in e.faces]) for e in mesh.edges],
                    [[vs[v.key] for v in r] for r in f.hole_loops],
                    sorted(fs[g.key] for g in mesh.surface_of(f)) if f in mesh.faces else []]
        rows.append(state())
        mesh.remove_hole(f,ring)
        mesh.relink_hole(f,ring)
        if rng.randrange(2):
            mesh.remove_face(other)
        mesh.remove_face(f)
        rows.append(state())
        mesh.relink_face(f)
        rows.append(state())
    return hashlib.sha256(json.dumps(rows,separators=(",",":")).encode()).hexdigest()


def test_scalar_face_edit_incidence(monkeypatch):
    assert scalar_results(monkeypatch) == 'c68bcb95e23206149fedc3c2039793d468f87c11640c3ae2c29cd5327fa98043'
