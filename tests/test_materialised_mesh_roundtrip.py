"""Golden object and tombstone graphs recorded before porting the compatibility bridge."""
import hashlib
import json
import random

import numpy as np
from ingetrazo_core import Vec3

from ingetrazo_core.store_mesh import StoreMesh
from core.topology import heal_overlapping_faces


def test_seeded_store_relink_weld_and_face_recovery():
    rng = random.Random(932740)
    digest = hashlib.sha256()

    def record(value):
        digest.update(json.dumps(value, sort_keys=True).encode())

    def key(v):
        return v.key & ((1 << 40) - 1)

    for i in range(48):
        mesh = StoreMesh()
        x, y = rng.randint(-100, 100), rng.randint(-100, 100)

        def square(left, bottom, right, top):
            return [Vec3(x + u, y + v, 0.) for u, v in
                    ((left,bottom), (right,bottom), (right,top), (left,top))]

        outer = square(0, 0, 12, 10)
        small = square(1, 1, rng.randint(3, 11), rng.randint(3, 9))
        f = mesh.add_face(outer, [small] if i % 2 else [])
        f.attrs["data"] = {"index": i}
        g = mesh.add_face(small)
        loose = mesh.add_edge(Vec3(x+20, y, 0), Vec3(x+21, y, 0))
        loose.soft = True
        loose.layer = "review"
        mesh.remove_face(g)
        mesh.remove_edge(loose)
        saved = mesh.capture_state()
        mesh.restore_state(saved)
        mesh.relink_edge(loose)
        mesh.relink_face(g)
        points = np.asarray([(x+j*.000051, y, 0.) for j in rng.sample(range(25), 25)]
                            + [(x, y+20., 0.)]*2)
        vs, ids = mesh.bulk_weld(points)
        record([[key(v) for v in vs], ids.tolist()])
        removed = heal_overlapping_faces(mesh, coverage=(i % 7)/7, partial=bool(i % 3))
        record([key(f) for f in removed])
        record([[key(v), v.position.toTuple(), sorted(key(e) for e in v.edges)]
                for v in mesh.vertices])
        record([[key(e), key(e.v0), key(e.v1), [key(f) for f in e.faces],
                 e.layer, e.soft] for e in mesh.edges])
        record([[key(f), [[key(v) for v in r] for r in [f.loop,*f.hole_loops]], f.attrs]
                for f in mesh.faces])

    # Store-local keys and ordered incidence recorded at integration commit 1a75bd48.
    assert digest.hexdigest() == "0c1b2eb2397f662f744bb3eccaf01f9dfc28bf55d829ff8f68c7f1dcbdee173f"


def test_seeded_fold_keeps_winding_attributes_and_soft_edges():
    from core.topology import fold_nonplanar_faces, _key

    outline = [(5, 0), (4, 2), (2, 4), (-1, 5), (-4, 3),
               (-5, 0), (-4, -2), (-2, -4), (1, -5), (4, -3)]
    rng = random.Random(67852)
    digest = hashlib.sha256()
    for seed in range(48):
        n = rng.randint(4, 10)
        mesh = StoreMesh()
        points = [Vec3(*outline[i * len(outline) // n], rng.randrange(-128, 128) / 256)
                  for i in range(n)]
        face = mesh.add_face(points)
        face.attrs["seed"] = seed
        for edge in mesh.edges:
            edge.curve = 17
        moved = {_key(p) for p in points[:n // 2]} if seed % 2 else set()
        assert fold_nonplanar_faces(mesh, moved=moved) == [face]
        def ring(f):
            coords = [p.toTuple() for p in f.vertices]
            return min(coords[i:] + coords[:i] for i in range(len(coords)))
        graph = (sorted((ring(f), f.attrs["seed"]) for f in mesh.faces),
                 sorted((sorted((e.a.toTuple(), e.b.toTuple())), e.soft) for e in mesh.edges))
        digest.update(json.dumps(graph, sort_keys=True).encode())
    assert digest.hexdigest() == "7070fb7121678e9a1614702738933e82747b6ef5518b79b781b4f31352e5e2a4"


def test_seeded_orphan_trim_and_displaced_registry():
    from core.topology import prune_collinear_orphan_edges

    rng = random.Random(19654)
    digest = hashlib.sha256()
    for seed in range(32):
        mesh = StoreMesh()
        intervals = [(0, 12), (3, 6), (8, 10), (11, 16)]
        rng.shuffle(intervals)
        for a, b in intervals:
            e = mesh.add_edge(Vec3(a, 0, 0), Vec3(b, 0, 0))
            e.layer = str(a)
            e.soft = a % 2 == 0
        ends = [mesh.add_edge(Vec3(i, 4, 0), Vec3(i, 6, 0)).v0 for i in range(4)]
        for v in ends[1:]:
            mesh.place_vertex(v, ends[0].position)
        mesh.move_vertex(ends[0], Vec3(2, 2, 0))
        mesh.move_vertex(ends[rng.randrange(1, 4)], Vec3(-2, 1, 0))
        removed = prune_collinear_orphan_edges(mesh)
        debris = mesh.purge_debris()
        graph = ([(e.a.toTuple(), e.b.toTuple(), e.layer, e.soft) for e in mesh.edges],
                 [e.layer for e in removed], [mesh.registers(v) for v in ends], debris,
                 sorted((cell, v.key % 2**40) for cell, v in mesh._registry.items()),
                 sorted((cell, [v.key % 2**40 for v in vs]) for cell, vs in mesh._displaced.items()))
        digest.update(json.dumps(graph).encode())
    assert digest.hexdigest() == "9e36e5bf1ae1499218675f3b412554e1504bc51174e4966b8f1447eb93b91c4d"
