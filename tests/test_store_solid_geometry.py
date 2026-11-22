# SPDX-License-Identifier: GPL-3.0-or-later
import pytest
from PySide6.QtGui import QVector3D as V

from core.bim import face_set_volume
from core.solids import solid_report
from core.protocol import queries
from core.store_mesh import StoreMesh
from tests.test_core_protocol import _scene, needs_store

pytestmark = needs_store


def empty(store):
    sc = _scene(store)
    sc.mesh.remove_face(sc.mesh.faces[0])
    for edge in list(sc.mesh.edges):
        sc.mesh.remove_edge(edge)
    sc.mesh.purge_debris()
    return sc


def box(mesh, x, open_top=False):
    points = [V(x+i, j, k) for k in (0, 1) for j in (0, 1) for i in (0, 1)]
    sides = [(0, 2, 3, 1), (0, 1, 5, 4), (1, 3, 7, 5), (3, 2, 6, 7),
             (2, 0, 4, 6), (4, 5, 7, 6)]
    for ring in sides[:5 if open_top else 6]:
        mesh.add_face([points[i] for i in ring])


@pytest.mark.parametrize('fixture,want', [('closed', 1), ('open', None), ('joined', None)])
def test_solid_reads(monkeypatch, fixture, want):
    monkeypatch.setattr(StoreMesh, '_materialise', lambda self: pytest.fail('materialised'))
    for store in (False, True):
        sc = empty(store)
        box(sc.mesh, 0, open_top=fixture == 'open')
        if fixture == 'joined':
            box(sc.mesh, 1)
        assert solid_report(sc.mesh) == (want is not None, want)
        assert queries.solid_report(sc)['volume'] == want
        assert face_set_volume(sc.mesh.faces) == want
        assert queries.face_set_volume(sc, [f.key for f in sc.mesh.faces])["volume"] == want


def test_triangle_order_and_snapshot(monkeypatch):
    monkeypatch.setattr(StoreMesh, '_materialise', lambda self: pytest.fail('materialised'))
    results = []
    for store in (False, True):
        sc = empty(store)
        def ring(xys):
            return [V(x, y, 1) for x, y in xys]
        face = sc.mesh.add_face(ring([(0,0),(10,0),(10,8),(6,8),(6,5),(0,5)]),
                                [ring([(x,1),(x,2),(x+1,2),(x+1,1)]) for x in (1,4,7)])
        result = queries.triangles(sc, [face.key])['faces'][0]['triangles']
        assert result
        results.append(result)
        if store:
            from core._native import qt_hypot
            snap = sc.mesh._store.snapshot()
            sc.mesh.remove_face(face)
            assert snap.face_triangles(face.key, qt_hypot()) == result
    assert results[0] == results[1]

def test_orient_feed(monkeypatch):
    from core.history import History
    from core.protocol.commands import run
    from tests.test_core_protocol import _expected, _got, _read
    monkeypatch.setattr(StoreMesh, '_materialise', lambda self: pytest.fail('materialised'))
    sc = _scene(True)
    sc.mesh.add_face([V(2,0,0), V(2,2,0), V(4,2,0), V(4,0,0)])
    history = History(sc)
    before = _read(sc.mesh)
    response = run(history, "orient_faces", face=sc.mesh.faces[0].key)
    assert response["ok"]
    assert _got(response["step"].meshes[0]) == _expected(before, _read(sc.mesh))
    assert run(history, "undo")["ok"]
    assert _read(sc.mesh) == before


@pytest.mark.parametrize("hole", [False, True])
def test_split_pushed_cap(monkeypatch, hole):
    from core.cap_rebuild import _canon_faces
    from tests.test_core_protocol import _expected, _got, _read
    from core.history import History
    from core.protocol.commands import run
    monkeypatch.setattr(StoreMesh, '_materialise', lambda self: pytest.fail('materialised'))
    results = []
    for store in (False, True):
        sc = empty(store)
        def quad(a, b, z=0):
            return [V(a,0,z), V(b,0,z), V(b,4,z), V(a,4,z)]
        opening = [V(1,1,0), V(1,2,0), V(2,2,0), V(2,1,0)]
        base = sc.mesh.add_face(quad(0,6), [opening] if hole else [])
        h = History(sc)
        assert run(h, "push_pull", face=base.key, distance=1)["ok"]
        cap = next(f for f in sc.mesh.faces if all(p.z() == 1 for p in f.vertices))
        sc.mesh.remove_face(cap)
        sc.mesh.add_face(quad(0,3,1), [[p+V(0,0,1) for p in opening]] if hole else [])
        seed = sc.mesh.add_face(quad(3,6,1))
        sc.mesh.add_edge(V(8,0,1), V(9,0,1))
        before = _read(sc.mesh)
        result = run(h, "rebuild_cap", face=seed.key)
        assert result["ok"]
        after = _read(sc.mesh)
        if store:
            assert _got(result["step"].meshes[0]) == _expected(before, after)
        assert run(h, "undo")["ok"] and _read(sc.mesh) == before
        assert run(h, "redo")["ok"] and _read(sc.mesh) == after
        assert sc.mesh.vertex_at(V(8,0,1)) is None
        caps = [f for f in sc.mesh.faces if all(p.z()==1 for p in f.vertices)]
        assert len(caps)==1 and len(caps[0].holes)==int(hole)
        results.append(_canon_faces([(f.vertices,f.holes,f.interior) for f in sc.mesh.faces]))
    assert results[0] == results[1]


@pytest.mark.parametrize("op,expected", [
    ("outer_shell",[1.5]), ("union",[1.5]), ("subtract",[.5]),
    ("trim",[.5]), ("intersect",[.5]), ("split",[.5,.5,.5])])
def test_store_solid_tool_feed(monkeypatch, op, expected):
    from core.group import Group
    from core.history import History
    from core.protocol.commands import run
    monkeypatch.setattr(StoreMesh, '_materialise', lambda self: pytest.fail('materialised'))
    sc = empty(True)
    inputs = []
    for x in (0,.5):
        m = empty(True).mesh
        box(m,x)
        inputs.append(Group(m))
    sc.groups.extend(inputs)
    history = History(sc)
    result = run(history, "solid_operation", operation=op, placements=[g.uid for g in inputs])
    assert result["ok"]
    made = [g for g in sc.groups if g not in inputs]
    assert all(isinstance(g.mesh, StoreMesh) for g in made)
    assert [solid_report(g.mesh)[1] for g in made] == expected
    delta = result["step"].placements
    assert set(delta["created"]) == {g.uid for g in made}
    assert set(delta["removed"]) == {g.uid for g in inputs if g not in sc.groups}
    assert result["step"].complete
    undo = run(history, "undo")["step"].placements
    assert set(undo["removed"]) == set(delta["created"])
    assert set(undo["created"]) == set(delta["removed"])


def test_partition_between_boxes(monkeypatch):
    from plugins.solid_inspector import inspect_mesh
    monkeypatch.setattr(StoreMesh, '_materialise', lambda self: pytest.fail('materialised'))
    for store in (False, True):
        sc = empty(store)
        box(sc.mesh, 0)
        box(sc.mesh, 1)
        partitions = [f for f in sc.mesh.faces if all(p.x() == 1 for p in f.vertices)]
        assert len(partitions) == 2
        sc.mesh.remove_face(partitions[1])
        assert solid_report(sc.mesh) == (False, None)
        assert len(inspect_mesh(sc.mesh)["over"]) == 4
        partitions[0].interior = True
        assert solid_report(sc.mesh) == (True, 2)
        assert inspect_mesh(sc.mesh)["volume"] == 2
