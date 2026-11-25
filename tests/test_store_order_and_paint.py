# SPDX-License-Identifier: GPL-3.0-or-later
"""Mesh construction preserves archived graph order, edge flags and paint."""
import hashlib
import json
import random

from ingetrazo_core import Vec3
from core.mesh import Mesh


def output():
    rows=[]
    for seed in range(24):
        mesh=Mesh()
        rng=random.Random(seed)
        for i in range(12):
            x,y=rng.randrange(10),rng.randrange(10)
            pts=[Vec3(x+dx,y+dy,0) for dx,dy in ((0,0),(1,0),(1,1),(0,1))]
            f=mesh.add_face(pts)
            f.attrs["paint"]={"row":i}
            f.interior=bool(i%3)
        for i,e in enumerate(mesh.edges):
            e.layer = None if i%3 else f"L{i%5}"
            e.curve = i if i%4 else None
            e.soft,e.flat,e.hidden=bool(i%3),bool(i%2),bool(i%5)
        index={v.key:i for i,v in enumerate(mesh.vertices)}
        rows.append([[v.position.toTuple() for v in mesh.vertices],
                     [(index[e.v0.key],index[e.v1.key],e.soft,e.flat,e.hidden,e.layer,e.curve) for e in mesh.edges],
                     [([index[v.key] for v in f.loop],f.interior,dict(f.attrs)) for f in mesh.faces]])
    return hashlib.sha256(json.dumps(rows,separators=(",",":")).encode()).hexdigest()


def test_store_preserves_recorded_order_and_paint():
    assert output() == 'ba2795ba40451b269ba166141d7ea8cbacc555ac83a8a2c4b424f1031da3b2a0'
