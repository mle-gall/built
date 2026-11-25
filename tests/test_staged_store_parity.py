# SPDX-License-Identifier: GPL-3.0-or-later
"""Staged import preserves keys, snapshots and properties set between stages."""
import hashlib
import json

import numpy as np
from core.store_mesh import StoreMesh


def staged_result():
    records=[]
    for seed in range(32):
        rng=np.random.default_rng(seed)
        dtype=np.float32 if seed % 2 else np.float64
        points=rng.integers(-3, 4, (72,3)).astype(dtype)
        points[1:6]=points[:5] + np.array([0.00004,0,0],dtype=dtype)
        m=StoreMesh()
        verts, inv=m.bulk_weld(points)
        snapshot=m.capture_state()
        vkeys=[v.key for v in verts]
        flags=[(i%2==0, i if i%3 else None, f"L{i%3}", i%4==0, i%5==0) for i in range(18)]
        emap=m.add_edges_welded(verts,inv[:18],inv[18:36],flags)
        if len(emap):
            e=next(iter(emap.values()))
            e.layer="between"
            e.hidden=True
        ekeys=[e.key for e in m.edges]
        attrs=[{"n":i%3, "nested":[True]} for i in range(6)]
        faces=m.add_faces_welded(verts,inv[36:],np.full(12,3),np.full(6,2),attrs,edge_map=emap)
        assert [v.key for v in m.vertices]==vkeys
        assert [e.key for e in m.edges][:len(ekeys)]==ekeys
        at={k:i for i,k in enumerate(vkeys)}
        records.append([inv.tolist(),[v.position.toTuple() for v in verts],
                        [(at[e.v0.key],at[e.v1.key],e.soft,e.hidden,e.flat,e.curve,e.layer,
                          [f.key-faces[0].key for f in e.faces]) for e in m.edges],
                        [([[at[v.key] for v in ring] for ring in [f.loop,*f.hole_loops]],dict(f.attrs)) for f in faces]])
        m.restore_state(snapshot)
        assert [v.key for v in m.vertices]==vkeys and not len(m.edges) and not len(m.faces)
    return hashlib.sha256(json.dumps(records,separators=(",",":")).encode()).hexdigest()


def test_staged_import_graph_and_keys():
    assert staged_result() == 'f3aa6bb7a36e8d5d8e11d03cf9dd0b1f1988c75d30a97916eeb375e3c3795b76'
