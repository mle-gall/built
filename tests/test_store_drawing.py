# SPDX-License-Identifier: GPL-3.0-or-later
"""Differential drawing through History, including exact stable-key feed records."""
import pytest
from PySide6.QtGui import QVector3D as V
from core import store_mesh as S, scan_index
from core.history import History, SnapshotMutation
from core.protocol.commands import run
from core.edits import join_preview
from core.topology import orient_coplanar_faces, winding_footprint
from tests.test_core_protocol import _scene, _read, _expected, _got, _inverse, needs_store
from tests.test_scan_index import _fingerprint


@needs_store
@pytest.mark.parametrize("case", ["closing", "chord", "chain", "inner", "touch", "divide_join", "curve", "orientation"])
def test_drawing_matches_python_and_change_feed(monkeypatch, case):
    def refuse(*args):
        pytest.fail("drawing materialised the store")

    monkeypatch.setattr(S.StoreMesh, "_materialise", refuse)
    states = []
    for store in (False, True):
        monkeypatch.setattr(scan_index, "_forced_off", not store)
        sc = _scene(store)
        m = sc.mesh
        h = History(sc)
        def command(name, **args):
            before = _read(m)
            result = run(h, name, **args)
            assert result["ok"], result
            after = _read(m)
            if store:
                expected = _expected(before, after)
                assert _got(result["step"].meshes[0]) == expected
                assert _got(run(h, "undo")["step"].meshes[0]) == _inverse(expected)
                assert _read(m) == before
                assert _got(run(h, "redo")["step"].meshes[0]) == expected
                assert _read(m) == after
            return _fingerprint(m)

        if case == "divide_join":
            edge = m.edges[0]
            edge.soft = True
            command("divide_edges", edges=[edge.key], n=4)
            selected = [e for e in m.edges if e.soft]
            before = _fingerprint(m)
            assert sorted(join_preview(m, selected).values()) == ["join"] * 3
            assert _fingerprint(m) == before
            state = command("join_edges", edges=[e.key for e in selected])
        elif case == "curve":
            import math
            pts = [V(5 + math.cos(i*math.tau/18), math.sin(i*math.tau/18), 0)
                   for i in range(18)]
            ring = m.add_face(pts)
            m.tag_curve(pts, closed=True)
            a = m.find_edge(ring.loop[3], ring.loop[4])
            state = command("divide_edges", edges=[a.key, a.key], n=4)
            assert len(set(e.curve for e in m.edges if e.curve is not None)) == 4
        elif case == "orientation":
            footprint = winding_footprint(m)
            m.remove_face(m.faces[0])
            m.add_face([V(0, 1, 0), V(1, 1, 0), V(1, 0, 0), V(0, 0, 0)])
            m.add_face([V(3, 0, 0), V(5, 0, 0), V(5, 2, 0), V(3, 2, 0)])
            h.execute(SnapshotMutation(lambda sc: orient_coplanar_faces(sc.mesh, footprint)))
            assert all(f.normal().z() > 0 for f in m.faces)
            state = _fingerprint(m)
        elif case in ("inner", "chain"):
            y = 0 if case == "chain" else 0.5
            ring = [(0.5, y, 0), (1.5, y, 0), (1.5, 1.5, 0), (0.5, 1.5, 0)]
            state = command("add_edges", segments=list(zip(ring, ring[1:] + ring[:1])))
            assert len(m.faces) == 2
        else:
            if case == "closing":
                a,b = (3,0,0), (4,0,0)
                m.add_edge(V(*a), V(3,1,0))
                m.add_edge(V(3,1,0), V(*b))
            elif case == "touch":
                m.add_hole(m.faces[0], [V(.5,.5,0), V(1,.5,0), V(1,1,0), V(.5,1,0)])
                a,b = (0,.5,0), (2,.5,0)
            else:
                a,b = (0,0,0), (2,2,0)
            state = command("add_edge", a=a, b=b)
        if store:
            assert type(m) is S.StoreMesh
        states.append(state)
    assert states[0] == states[1]


@needs_store
@pytest.mark.parametrize("curved", [False, True])
def test_store_rectangle_never_enumerates_face_views(monkeypatch, curved):
    from tests.test_scan_index import _grid_points
    from core.edits import build_add_edges
    from core.history import AddFaceCommand
    from core.scene import Scene
    import numpy as np
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    sc = Scene()
    n = 4 if curved else 57
    sc.mesh.add_faces_bulk(np.asarray(_grid_points(n, n + 1)), np.full(n*n, 4), np.ones(n*n, int))
    if curved:
        sc.mesh.edges[0].curve = 123
        sc.mesh.faces[0].attrs["color"] = "red"
    hist = History(sc)
    rect = [V(n+1,0,0), V(n+2,0,0), V(n+2,1,0), V(n+1,1,0)]
    iterate = S._Seq.__iter__
    def guarded(seq):
        assert seq._kind != "face", "Rectangle walked the mesh face list"
        return iterate(seq)
    monkeypatch.setattr(S._Seq, "__iter__", guarded)
    monkeypatch.setattr(S.StoreMesh, "_materialise", lambda *args: pytest.fail("materialised"))
    cmd = build_add_edges(sc, list(zip(rect, rect[1:] + rect[:1])), detect_faces=False,
                          extra=[AddFaceCommand(rect)], plane_rebuild=True)
    hist.execute(cmd)
    assert not hist.last_error, hist.last_error
    assert type(sc.mesh) is S.StoreMesh
    assert len(sc.mesh.faces) == n*n + 1
    if curved:
        corner = sc.mesh.vertex_at(V(0,0,0))
        assert any(f.attrs.get("color") == "red" for f in sc.mesh.faces_at(corner))


@needs_store
def test_cycle_graph_keeps_edge_order_across_planned_edits():
    from core.topology import CycleGraph, find_cycles_through, find_smallest_cycle_through
    from core.geometry import Edge
    results = []
    for store in (False, True):
        m = _scene(store).mesh
        a, b = V(0, 0, 0), V(2, 2, 0)
        graph = CycleGraph(m.edges)
        def loops():
            return [[p.toTuple() for p in ring] for ring in find_cycles_through(m.edges, a, b, graph=graph)]
        before = loops()
        assert len(before) == 2
        assert [v.toTuple() for v in find_smallest_cycle_through(m.edges, a, b)] == before[0]
        assert graph.remove(m.edges[0])
        after = loops()
        graph.add(Edge(a, V(2, 0, 0)))
        assert len(after) == 1 and len(loops()) == 2
        results.append((before, after, loops()))
    assert results[0] == results[1]


@needs_store
def test_store_ring_queries_match_the_position_kernel():
    from core import topology as topo
    results = []
    for store in (False, True):
        m = _scene(store).mesh
        face = m.faces[0]
        opening = [V(.5,.5,0), V(1,.5,0), V(1,1,0), V(.5,1,0)]
        m.add_hole(face, opening)
        neighbour = [V(1,.5,0), V(1.5,.5,0), V(1.5,1,0), V(1,1,0)]
        band = [V(0,.2,0), V(2,.2,0), V(2,.4,0), V(0,.4,0)]
        loose = m.add_edge(V(2,2,0), V(3,3,0))
        wall = m.add_face([V(2,0,0), V(2,2,0), V(2,2,1), V(2,0,1)])
        idx, merged = topo.hole_merged_with_loop(face, neighbour)
        assert idx == 0
        kind, owner = topo.classify_push_edge(face, V(2,.5,0), V(2,1.5,0), m.faces)
        assert kind == "perp" and owner == wall
        assert topo.orphaned_edges_at(m.edges, m.faces, [V(2,2,0)]) == [loose]
        assert topo.find_containing_face(m.faces, neighbour) == face
        pieces = topo.carve_loop_by_chords(face, band)
        assert len(pieces) == 2
        refined = topo.refine_loop_with_points(face.vertices, band, mesh=m)
        segments, cuts = topo.plan_edge_split(m.edges, V(-1,-1,0), V(2,2,0))
        results.append(([p.toTuple() for p in merged], [p.toTuple() for p in refined],
                        [[p.toTuple() for p in ring] for ring in pieces],
                        [(a.toTuple(), b.toTuple()) for a,b in segments],
                        [(e.a.toTuple(), e.b.toTuple(), p.toTuple()) for e,p in cuts.items()]))
    assert results[0] == results[1]
