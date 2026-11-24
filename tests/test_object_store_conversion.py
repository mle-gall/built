# SPDX-License-Identifier: GPL-3.0-or-later
"""Object-to-store conversion preserves graph order, paint ownership and refusal atomicity."""
import hashlib
import json
import random

import pytest
from ingetrazo_core import Vec3
from core.mesh import Mesh, Vertex
from core.store_mesh import StoreRefusal, to_store


def output(monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE","0")
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
        vertices,edges,faces=map(list,(mesh.vertices,mesh.edges,mesh.faces))
        paints=[f.attrs for f in faces]
        views=to_store(mesh)
        assert [views[id(o)].key for o in vertices]==[v.key for v in mesh.vertices]
        assert [views[id(o)].key for o in edges]==[e.key for e in mesh.edges]
        assert all(views[id(f)].attrs is a for f,a in zip(faces,paints))
        index={v.key:i for i,v in enumerate(mesh.vertices)}
        rows.append([[v.position.toTuple() for v in mesh.vertices],
                     [(index[e.v0.key],index[e.v1.key],e.soft,e.flat,e.hidden,e.layer,e.curve) for e in mesh.edges],
                     [([index[v.key] for v in f.loop],f.interior,dict(f.attrs)) for f in mesh.faces]])
    broken=Mesh()
    v=broken.vertex(Vec3(1,2,3))
    broken.vertices.append(Vertex(v.position))
    with pytest.raises(StoreRefusal):
        to_store(broken)
    assert type(broken) is Mesh and len(broken.vertices)==2
    return hashlib.sha256(json.dumps(rows,separators=(",",":")).encode()).hexdigest()


def test_materialised_store_conversion(monkeypatch):
    assert output(monkeypatch) == '18749646a9aee5b810f578fe5088ee9e56bea831fad3288edf15609546d68a1a'
