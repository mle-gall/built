"""Stable element names survive edits and snapshots without recycling."""
from copy import deepcopy

import numpy as np
import pytest
from built_core import Vec3 as V

from built_core.mesh import Mesh, DeferredMesh
from built_core.scene import Scene
from built_core.history import History, MoveVerticesCommand


def keys(m):
    return [e.key for e in (*m.vertices, *m.edges, *m.faces)]


def triangle(m):
    return m.add_face([V(0, 0, 0), V(2, 0, 0), V(0, 2, 0)])


def test_unique_move_restore_and_clear():
    m = Mesh()
    f = triangle(m)
    before = keys(m)
    assert len(before) == len(set(before))
    snap = m.capture_state()
    m.move_vertex(f.loop[0], V(0, 0, 1))
    assert keys(m) == before
    m.place_vertex(f.loop[0], V(0, 0, 2))
    assert keys(m) == before
    discarded = m.vertex(V(8, 8, 8)).key
    m.restore_state(snap)
    assert keys(m) == before
    assert m.vertex(V(9, 9, 9)).key not in {*before, discarded}
    m.clear()
    triangle(m)
    assert set(keys(m)).isdisjoint(before)


def test_history_move_undo_redo():
    scene = Scene()
    f = triangle(scene.mesh)
    before = keys(scene.mesh)
    history = History(scene)
    history.execute(MoveVerticesCommand(f.vertices, V(0, 0, 1)))
    after = keys(scene.mesh)
    history.undo()
    assert keys(scene.mesh) == before
    history.redo()
    assert keys(scene.mesh) == after


@pytest.mark.parametrize('recorded', [False, True])
@pytest.mark.parametrize('existing', [False, True])
def test_split_keeps_first_half_and_reused_edges(recorded, existing):
    m = Mesh()
    e = m.add_edge(V(0, 0, 0), V(2, 0, 0))
    mid = m.vertex(V(1, 0, 0))
    first = m.add_edge(e.a, mid.position) if existing else None
    expected = first.key if first else e.key
    if recorded:
        rec = m.split_edge_at(e, mid)
        halves = rec['e0'], rec['e1']
    else:
        halves = m.split_edge(e, mid.position)
    assert halves[0].key == expected
    assert halves[1].key != expected
    assert len(keys(m)) == len(set(keys(m)))
    if recorded:
        m.revert_stitch(rec)
        assert e in m.edges
        assert e.key != halves[1].key


def test_weld_redirect_and_coincident_survivor():
    m = Mesh()
    e = m.add_edge(V(0, 0, 0), V(2, 0, 0))
    keep = m.vertex(V(1, 0, 0))
    old_key, keep_key = e.key, keep.key
    m._weld_into(keep, e.v0)
    assert m.find_edge(keep, e.v1).key == old_key
    assert keep.key == keep_key
    dup = m.vertex(V(3, 0, 0))
    dup.position = V(keep.position)
    m.weld_coincident()
    assert keep in m.vertices and dup not in m.vertices
    assert keep.key == keep_key


def test_collapse_and_dedupe_rules():
    m = Mesh()
    a = m.add_edge(V(0, 0, 0), V(1, 0, 0))
    b = m.add_edge(V(1, 0, 0), V(2, 0, 0))
    retired = {a.key, b.key, a.v1.key}
    rec = m.collapse_vertex(a.v1)
    assert rec['new_edge'].key not in retired
    m.revert_stitch(rec)
    assert retired <= set(keys(m))
    f = triangle(m)
    g = triangle(m)
    assert m.dedupe_faces() == 1
    assert f in m.faces and g not in m.faces


def test_dissolve_fresh_face_and_undo():
    m = Mesh()
    f = triangle(m)
    g = m.add_face([V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    shared = next(e for e in m.edges if len(e.faces) == 2)
    retired = {f.key, g.key, shared.key}
    rec = m.dissolve_edge_recorded(shared)
    assert rec['merged'].key not in retired
    m.revert_stitch(rec)
    assert retired <= set(keys(m))


def test_bulk_deferred_and_copy():
    m = Mesh()
    pos = np.array([[0, 0, 0], [2, 0, 0], [0, 2, 0]], dtype=float)
    DeferredMesh.defer(m, lambda mesh: mesh.add_faces_bulk(pos, [3], [1]), 1)
    assert '_deferred' in m.__dict__
    assert len(keys(m)) == 7
    assert len(set(keys(m))) == 7
    copied = deepcopy(m)
    assert set(keys(copied)).isdisjoint(keys(m))
    assert len(set(keys(copied))) == 7


def test_selection_identity_and_copy():
    from built_core.mesh import Face
    scene = Scene()
    f = triangle(scene.mesh)
    scene.selection = {f}
    handle = Face(f.loop, key=f.key)
    assert handle in scene.selection
    scene.selection.add(handle)
    assert len(scene.selection) == 1
    scene.selection = {f}
    copied = deepcopy(scene)
    assert copied.mesh.faces[0] in copied.selection
    assert f not in copied.selection


def test_relink_and_attributes_keep_keys():
    m = Mesh()
    f = triangle(m)
    e = m.edges[0]
    before = f.key, e.key
    f.attrs['color'] = '#ffffff'
    e.hidden = True
    m.remove_face(f)
    m.remove_edge(e)
    m.relink_edge(e)
    m.relink_face(f)
    assert (f.key, e.key) == before


def test_split_snapshot_round_trip_and_weld_existing_edge():
    m = Mesh()
    e = m.add_edge(V(0, 0, 0), V(2, 0, 0))
    before = m.capture_state()
    halves = m.split_edge(e, V(1, 0, 0))
    after = m.capture_state()
    after_keys = keys(m)
    m.restore_state(before)
    assert m.edges == [e]
    m.restore_state(after)
    assert keys(m) == after_keys
    keep, dup = halves[0].v0, halves[0].v1
    destination = m.add_edge(keep.position, halves[1].v1.position)
    expected = destination.key
    m._weld_into(keep, dup)
    assert destination in m.edges and destination.key == expected
    assert len(keys(m)) == len(set(keys(m)))


def test_bulk_welded_paths_and_multiple_meshes():
    meshes = [Mesh(), Mesh()]
    for m in meshes:
        vs, ids = m.bulk_weld(np.array([[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]]))
        m.add_edges_welded(vs, ids[:2], ids[1:])
        m.add_faces_welded(vs, ids, [3], [1])
        assert len(keys(m)) == len(set(keys(m))) == 7
    assert set(keys(meshes[0])).isdisjoint(keys(meshes[1]))


def test_added_face_boundary_keys_survive_redo():
    from built_core.history import AddFaceCommand
    scene = Scene()
    history = History(scene)
    history.execute(AddFaceCommand([V(0, 0, 0), V(2, 0, 0), V(0, 2, 0)]))
    expected = set(keys(scene.mesh))
    history.undo()
    history.redo()
    assert set(keys(scene.mesh)) == expected
