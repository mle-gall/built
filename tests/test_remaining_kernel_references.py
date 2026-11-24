"""Ordered kernel results recorded from integration 186f0d8f."""
import hashlib
import json
import random
from types import SimpleNamespace

from ingetrazo_core import Vec3 as V
from core import arrangement as ar, push_scans as scans, snap_query as sq, topology as top
from core.mesh import Mesh


def digest(rows):
    return hashlib.sha256(json.dumps(rows, separators=(',', ':')).encode()).hexdigest()


def snap_rows():
    rows = []
    for seed in range(32):
        r = random.Random(seed)
        edges = []
        for i in range(seed):
            p = V(r.choice([0, 650000]), r.randrange(-2, 4), i)
            flags = {name: bool(r.getrandbits(1)) for name in
                     ('center', 'component_origin', 'arc_midpoint', 'guide', 'soft', 'hidden', 'figure')}
            edges.append(SimpleNamespace(a=p, b=p+V(1, 2, 0), context=None if i%3 else '',
                                         curve=0 if i%4 else None, **flags))
        a, b, tags, groups = sq.edge_columns(iter(edges))
        q = sq.make_query(edges, lambda p: p.toTuple()[:2], (1., 2.))
        hit = sq.point_result(q, edges, 3., lambda p: p.z() == 2, centers=[(V(0,2,1),'g')])
        rows.append((a.shape, b.shape, a.tolist(), b.tolist(), tags, groups,
                     None if hit is None else (hit.kind,hit.point.toTuple(),hit.context)))
    return rows


def plane_rows():
    rows = []
    for seed in range(48):
        r = random.Random(seed)
        n = V(r.randrange(-2, 3), r.randrange(-2, 3), r.choice([.1,1,2]))
        u,v = ar.plane_basis(n)
        x,y = r.choice([0, 650000]), r.choice([0, 6860000])
        ring = [(x+i,y+j) for i,j in ((0,0),(2,0),(1,1),(2,3),(0,3))]
        if seed%2: ring.reverse()
        rows.append(([p.toTuple() for p in (u,v)],ar._key(x+.00005,y-.00005),
                     ar._signed_area(ring),ar._area_floor(ring),
                     [ar._point_in_polygon(q,ring) for q in (ring[0],(x+1,y+1),(x+2,y+2))],
                     ar._classify([list(range(5))],dict(enumerate(ring)))))
    return rows


def mesh_rows():
    rows = []
    for seed in range(32):
        mesh = Mesh()
        r = random.Random(seed)
        x = r.choice([0, 650000])
        pts = [V(x+a,b,0) for a,b in ((0,0),(4,0),(4,4),(0,4))]
        f = mesh.add_face(pts)
        if seed%2: mesh.add_face(pts[::-1])
        if seed%3: mesh.add_hole(f,[V(x+1,1,0),V(x+2,1,0),V(x+1,2,0)])
        f.reverse_rings()
        c = V(x+1,0,0)
        changes = top.split_edge_in_faces(mesh.faces,pts[0],pts[1],c,[pts[-1]] if seed%2 else [])
        before = top.defects_near(mesh.vertices)
        old = mesh.capture_state()
        mesh.split_edge_at(mesh.edges[0],mesh.vertex(c))
        n = mesh.collapse_collinear()
        mesh.restore_state(old)
        rows.append((before, top.new_defects(before,top.defects_near(mesh.vertices)), n,
                     [[p.toTuple() for p in ring] for _,ring in changes],
                     [[p.toTuple() for p in ring] for ring in [f.vertices,*f.holes]],
                     len(mesh.vertices[0].faces())))
    return rows


def scan_rows():
    rows = []
    for seed in range(32):
        mesh = Mesh()
        pts = [V(650000+i,seed,i%3) for i in range(10)]
        for a,b in zip(pts,pts[1:]): mesh.add_edge(a,b)
        points = pts[::1+seed%4]
        near = scans.vertices_at_keys(mesh, iter(points))
        box = scans._grown_box([p.toTuple() for p in points])
        rows.append((box,None if near is None else [v.position.toTuple() for v in near],
                     len(scans.edges_near_points(mesh, [p.toTuple() for p in points]) or [])))
    return rows


def test_snap_query_reference():
    assert digest(snap_rows()) == '752557761b1e02e55cbd5395bfbe945b9dd76358a48dcd74d991d375477330bc'


def test_planar_helpers_reference():
    assert digest(plane_rows()) == 'ccc4e4564dc05efd8cace4a79da0e29599fe3f77bf2398a6b7b3e032d362c513'


def test_materialised_mesh_reference(monkeypatch):
    monkeypatch.setenv('INGETRAZO_STORE','0')
    assert digest(mesh_rows()) == 'd252446679246d0719b0b5f4f81eb569de1797150e2504c8bf7b25f6efad7014'


def test_push_scan_reference(monkeypatch):
    monkeypatch.setenv('INGETRAZO_STORE','0')
    assert digest(scan_rows()) == '5ce69f52b7511e4d417a427afd38bcb5cf366c3fb22133fd9e551ad58e1cfc22'


def repair_rows():
    rows = []
    for seed in range(24):
        mesh = Mesh()
        origin = 650000 if seed % 2 else 0
        def rect(x, y, w, h, z=0):
            return [V(origin + x + i, y + j, z) for i,j in ((0,0),(w,0),(w,h),(0,h))]
        rim = rect(0,0,6,6)
        mother = mesh.add_face(rim)
        mother.attrs = {'name':'outer'}
        if seed % 3:
            mesh.add_hole(mother, rect(1,1,1,1))
        small = rect(0,0,3,6,1 if seed % 4 == 0 else 0)
        mesh.add_face(small if seed % 3 else small[::-1]).attrs = {'name':'small'}
        mesh.add_edge(rim[0], V(origin+8,0,0)).soft = True
        point = V(origin+2,0,0)
        split = top.split_edge_in_faces(mesh.faces,rim[0],rim[1],point,
                                        [V(origin+4,3,0)] if seed%3 else ())
        same = top.face_exists(mesh.faces,rim[::-1])
        removed = top.heal_overlapping_faces(mesh, coverage=(seed % 5)/4)
        rows.append((same, [[p.toTuple() for p in r] for _,r in split], len(removed),
                     [([p.toTuple() for p in f.vertices],
                       [[p.toTuple() for p in h] for h in f.holes],dict(f.attrs))
                      for f in mesh.faces],
                     [(e.a.toTuple(), e.b.toTuple(), e.soft) for e in mesh.edges]))
    return rows


def test_repair_reference(monkeypatch):
    monkeypatch.setenv('INGETRAZO_STORE','0')
    assert digest(repair_rows()) == 'ee76db670b286da00c61cf8867ee4ed8ac8eeaa440265fe7424e6a1bc22773a7'


def test_store_repair_reference(monkeypatch):
    monkeypatch.setenv('INGETRAZO_STORE','1')
    assert digest(repair_rows()) == 'ee76db670b286da00c61cf8867ee4ed8ac8eeaa440265fe7424e6a1bc22773a7'
