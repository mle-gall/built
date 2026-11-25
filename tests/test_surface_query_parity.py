# SPDX-License-Identifier: GPL-3.0-or-later
"""Reference ordering for shared rims, nested faces and orphaned edges."""
import hashlib
import json
import random

from built_core import Vec3
from built_core.geometry import Face, Edge
from built_core.topology import hole_merged_with_loop, find_containing_face, orphaned_edges_at, face_is_bordered


def output():
    rows = []
    for seed in range(48):
        rng = random.Random(seed)
        off = 1e6 if seed % 2 else 0
        def p(x,y):
            return Vec3(x+off, y, x*.25-y*.125)
        def box(x,y,w,h):
            return [p(x,y),p(x+w,y),p(x+w,y+h),p(x,y+h)]
        hole=box(0,0,4,4)
        if seed % 3: hole.reverse()
        hole=hole[seed%4:] + hole[:seed%4]
        g=Face(box(-10,-10,20,20), [box(-8,-8,1,1),hole])
        y=rng.choice([0,1,2,3,4])
        drawn=box(4+rng.choice([0,0,0,-1,1]), y, 2, rng.choice([1,2,3]))
        if seed%4: drawn.reverse()
        merged=hole_merged_with_loop(g,drawn)
        faces=[g,Face(hole),Face(drawn),Face(box(-9,-9,18,18))]
        edges=[Edge(r[i],r[(i+1)%len(r)]) for r in [drawn,hole] for i in range(len(r))]
        edges.extend(Edge(v,p(12,12)) for v in g.vertices+drawn)
        def ordinal(objects,o):
            return next((i for i,obj in enumerate(objects) if obj is o),None)
        rows.append([None if merged is None else [merged[0],[v.toTuple() for v in merged[1]]],
                     [ordinal(faces,find_containing_face(iter(faces),q,ex)) for q in [hole,drawn,box(1,1,.1,.1),[]] for ex in [None,g]],
                     [ordinal(edges,e) for e in orphaned_edges_at(iter(edges),iter(faces),drawn[:2])],
                     [face_is_bordered(f,iter(faces)) for f in faces]])
    return hashlib.sha256(json.dumps(rows,separators=(",",":")).encode()).hexdigest()


def test_surface_queries_match_reference():
    assert output() == '1e4820984f7fc058bdc98ea709c6b5b20aaa30595a048f9718e34171f18670f8'
