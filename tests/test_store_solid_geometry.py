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
