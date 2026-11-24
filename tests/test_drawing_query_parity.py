# SPDX-License-Identifier: GPL-3.0-or-later
"""Ordered outside contacts and duplicate handles from the Python reference."""
import hashlib
import json
import random

from ingetrazo_core import Vec3
from core.edits import touching_segments
from core.geometry import Edge, Face
from core.topology import find_duplicate_edge


def query_results():
    results = []
    for seed in range(72):
        rng = random.Random(seed)
        origin = 2**18 if seed % 2 else 0
        def p(x,y,z=0):
            return Vec3(origin+x, y, z)
        edges = [Edge(p(rng.randrange(-8,9), rng.randrange(-8,9)),
                      p(rng.randrange(-8,9), rng.randrange(-8,9))) for _ in range(12)]
        a,b = edges[0].a, edges[0].b
        edges.extend([Edge(b,a), Edge(a,a)])
        outer = [p(-4,-4),p(4,-4),p(4,4),p(-4,4)]
        hole = [p(-2,-2),p(-2,2),p(2,2),p(2,-2)]
        face = Face(outer, [hole])
        segments = [(e.a,e.b) for e in edges] + [(b,a), (a,a), (p(0,0),p(.25,.75))]
        for _ in range(20):
            z=rng.choice([0., 0.000099, 0.000101, .05])
            segments.append((p(rng.uniform(-6,6),rng.uniform(-6,6),z),
                             p(rng.uniform(-6,6),rng.uniform(-6,6),z)))
        rows = []
        for other, faces in ((edges, [face]), ([], [face]), (edges, [])):
            hits = touching_segments(segments, iter(other), iter(faces))
            rows.append([[u.toTuple(),v.toTuple()] for u,v in hits])
        duplicates = [find_duplicate_edge(iter(edges), *pair) for pair in segments]
        rows.append([next((i for i,e in enumerate(edges) if e is hit), None) for hit in duplicates])
        results.append(rows)
    return hashlib.sha256(json.dumps(results,separators=(",",":")).encode()).hexdigest()


def test_outside_contacts_and_duplicate_order():
    assert query_results() == '3a4184db1bf71e496f72a28466556cdf4f4de729c85e2cd0ad4f0d016fc7fe26'


def overlay_results(monkeypatch):
    from core import scan_index
    from core.edits import _ScanFaces
    from core.mesh import Mesh
    outputs = []
    for store in (False, True):
        monkeypatch.setenv("INGETRAZO_STORE", str(int(store)))
        for seed in range(16):
            mesh = Mesh()
            rng = random.Random(seed)
            def face_at(i):
                return [Vec3(i,0,0),Vec3(i+.8,0,0),Vec3(i,.8,0)]
            for i in range(10):
                mesh.add_face(face_at(i))
            overlay = _ScanFaces(scan_index.scan(mesh))
            all_points = [Vec3(-1,-1,0),Vec3(12,2,0)]
            for i in range(12):
                existing = overlay.pick(all_points)
                chosen = rng.choice(existing)
                if i % 3 == 0:
                    overlay.remove(chosen)
                    assert chosen not in overlay
                elif i % 3 == 1:
                    overlay[overlay.index(chosen)] = Face(face_at(rng.randrange(10)))
                else:
                    overlay.append(Face(face_at(rng.randrange(10))))
                x = rng.randrange(5)
                hits = overlay.pick([Vec3(x,0,0), Vec3(x+3,1,0)])
                outputs.append([f.vertices[0].x() for f in hits])
    return hashlib.sha256(json.dumps(outputs).encode()).hexdigest()


def test_scanned_face_overlay_order(monkeypatch):
    assert overlay_results(monkeypatch) == '8f63c08af006b799abdeadead54e5cdeb7799f155083081ac6f459029874b9fa'
