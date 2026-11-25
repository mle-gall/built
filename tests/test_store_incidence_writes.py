# SPDX-License-Identifier: GPL-3.0-or-later
import pytest
from ingetrazo_core import Vec3 as V
from core.mesh import Mesh


@pytest.mark.parametrize("side", ["v0", "v1"])
def test_loose_endpoint_replaces_disk_incidence(side):
    mesh = Mesh()
    edge = mesh.add_edge(V(0, 0, 0), V(1, 0, 0))
    other = mesh.add_edge(V(3, 0, 0), V(4, 0, 0)).v0
    old = getattr(edge, side)
    setattr(edge, side, other)
    assert getattr(edge, side) == other
    assert edge in other.edges and edge not in old.edges
    with pytest.raises(ValueError, match="degenerate"):
        setattr(edge, side, edge.other(other))


def test_face_ring_writes_update_both_sides():
    mesh = Mesh()
    face = mesh.add_face([V(0, 0, 0), V(5, 0, 0), V(0, 5, 0)])
    small = mesh.add_face([V(1, 1, 0), V(2, 1, 0), V(1, 2, 0)])
    loop = list(face.loop)
    face.loop.reverse()
    assert list(face.loop) == loop[::-1]
    face.loop = loop
    holes = face.hole_loops
    holes.append(list(small.loop))
    holes[0].reverse()
    assert face.hole_loops[0] == small.loop[::-1]
    face.hole_loops[0].reverse()
    face.hole_loops[0] *= 1
    inner = next(iter(small.loop[0].edges))
    assert face in inner.faces and small in inner.faces
    faces = list(inner.faces)
    inner.faces.reverse()
    assert inner.faces == faces[::-1]
    face.hole_loops[0].reverse()
    assert face.hole_loops[0] == small.loop[::-1]
    face.hole_loops = []
    assert inner.faces == [small]
    with pytest.raises(ValueError, match="face ring"):
        inner.v0 = loop[0]
    with pytest.raises(ValueError, match="face rings"):
        inner.faces.clear()
    with pytest.raises(ValueError, match="endpoints"):
        loop[0].edges.clear()
    with pytest.raises(ValueError, match="endpoints"):
        loop[0].edges = set()


@pytest.mark.parametrize("kind", ["vertices", "edges", "faces"])
def test_keyed_mesh_lists_remove_and_restore(kind):
    mesh = Mesh()
    if kind == "vertices":
        item = mesh.vertex(V(0, 0, 0))
    elif kind == "edges":
        item = mesh.add_edge(V(0, 0, 0), V(1, 0, 0))
    else:
        item = mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
    seq = getattr(mesh, kind)
    assert seq.pop() == item and not seq
    seq.append(item)
    seq *= 1
    assert seq[:] == [item]
    with pytest.raises(ValueError, match="unique"):
        seq.append(item)
    seq *= 1
    assert seq[:] == [item]


@pytest.mark.parametrize(("name", "value"), [
    ("soft", True), ("flat", True), ("hidden", True), ("curve", 37), ("layer", "new"),
])
def test_edge_properties_edit_store(name, value):
    mesh = Mesh()
    edge = mesh.add_edge(V(0, 0, 0), V(1, 0, 0))
    setattr(edge, name, value)
    assert getattr(mesh.edges[0], name) == value
    with pytest.raises(TypeError):
        setattr(edge, name, {})
    assert getattr(mesh.edges[0], name) == value


def test_face_attributes_and_vertex_position():
    mesh = Mesh()
    face = mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
    face.attrs = {"paint": "blue"}
    face.interior = True
    face.loop[0].position = V(0, 0, 1)
    assert mesh.faces[0].attrs == {"paint": "blue"} and mesh.faces[0].interior
    assert mesh.vertex_at(V(0, 0, 1)) == face.loop[0]


def test_invalid_and_stale_ring_writes_are_atomic():
    mesh = Mesh()
    face = mesh.add_face([V(0, 0, 0), V(4, 0, 0), V(0, 4, 0)])
    vertices = face.loop[:]
    before = mesh.capture_state()
    foreign = Mesh().vertex(V(0, 0, 0))
    with pytest.raises(ValueError, match="from this mesh"):
        face.loop[1] = foreign
    with pytest.raises(ValueError, match="from this mesh"):
        mesh.edges[0].v0 = foreign
    assert mesh.capture_state() == before
    face.hole_loops = [vertices]
    held = face.hole_loops[0]
    face.hole_loops.clear()
    with pytest.raises(ValueError, match="replaced"):
        held.reverse()
    assert face.loop == vertices and face.hole_loops == []
    mesh._store.check()
