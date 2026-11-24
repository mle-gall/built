"""Golden object and tombstone graphs recorded before porting the compatibility bridge."""
import hashlib
import json
import random

import numpy as np
from ingetrazo_core import Vec3

from core.store_mesh import empty_store_mesh
from core.topology import heal_overlapping_faces


def test_seeded_materialisation_relink_weld_and_face_recovery():
    rng = random.Random(932740)
    digest = hashlib.sha256()

    def record(value):
        digest.update(json.dumps(value, sort_keys=True).encode())

    def key(v):
        return v.key & ((1 << 40) - 1)

    for i in range(48):
        mesh = empty_store_mesh()
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
        mesh._materialise()
        handles = dict(mesh._objmap)
        mesh.restore_state(saved)
        mesh._materialise()
        assert all(mesh._objmap[k] is obj for k, obj in handles.items())
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

    assert digest.hexdigest() == "b9856a05e4efedbe2af0b5a51a669b0bb343a8b455620a414502271c3ec5aeaf"
