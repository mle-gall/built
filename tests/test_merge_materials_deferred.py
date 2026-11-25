# SPDX-License-Identifier: GPL-3.0-or-later
"""A material name that collides when a file is linked or inserted is
restamped in a deferred mesh's tables, without building the mesh.

On the ride model three colliding names in the linked facilities built
every one of its meshes on open (4.8 s of 9.5 s). The faces must come out
exactly as the old restamp left them — built or not — and a mesh placed
twice is restamped once (renaming it per placement moved a name twice
when one new name was another's old one)."""
from __future__ import annotations

import pytest
from built_core import Matrix4 as QMatrix4x4, Vec3 as V

from core import _native
from core.group import Group, iter_placements
from built_core.document_insert import merge_materials
from core.materials import Material
from core.mesh import DeferredMesh, Mesh, face_attrs
from core.scene import Scene
from formats import igz


def _write(path):
    scene = Scene()
    for i, mat in enumerate(["A", "A (2)", None, "B"]):
        f = scene.mesh.add_face([V(i, 0, 0), V(i + 1, 0, 0), V(i, 1, 0)])
        if mat:
            f.attrs["mat"] = mat
        if i == 3:
            f.attrs["back"] = {"mat": "A", "color": [1, 0, 0]}
    proto = Mesh()
    for i, mat in enumerate(["A", "A (2)", "B"]):
        proto.add_face([V(i, 5, 0), V(i + 1, 5, 0), V(i, 6, 0)]).attrs[
            "mat"] = mat
    for dx in (0, 10):
        g = Group(proto, name=f"Silla {dx}")
        x = QMatrix4x4()
        x.translate(dx, 0, 0)
        g.xform = x
        scene.groups.append(g)
    igz.save_scene(scene, path)


def _source_materials():
    return {"A": Material("A", color=(1.0, 0.0, 0.0)),
            "A (2)": Material("A (2)", color=(0.0, 1.0, 0.0)),
            "B": Material("B", color=(0.0, 0.0, 1.0))}


def _host_materials():
    # "A" taken by another recipe: A → A (2), and then the source's own
    # "A (2)" → "A (2) (2)". "B" is the same recipe: kept.
    return {"A": Material("A", color=(0.5, 0.5, 0.5)),
            "B": Material("B", color=(0.0, 0.0, 1.0))}


def _component(src):
    """The definition a link makes of a loaded file (``built_core.link_load_source``;
    component_from_scene would build the loose mesh to see if it is empty)."""
    comp = Group(src.mesh, name="file")
    comp.adopt(src.groups)
    return comp


def _merged(path, deferred, monkeypatch):
    monkeypatch.setattr(_native, "wanted", lambda: deferred)
    src = Scene()
    igz.load_into(src, path)
    comp = _component(src)
    meshes = []
    for g, _m in iter_placements(comp):
        if all(g.mesh is not m for m in meshes):
            meshes.append(g.mesh)
    assert all((type(m) is DeferredMesh) == deferred for m in meshes)
    renamed = merge_materials(_source_materials(), _host_materials(), comp)
    return renamed, meshes


@pytest.fixture
def path(tmp_path, monkeypatch):
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)
    out = tmp_path / "file.igz"
    _write(out)
    return out


def test_deferred_meshes_are_restamped_without_being_built(path, monkeypatch):
    renamed, meshes = _merged(path, True, monkeypatch)
    assert renamed == {"A": "A (2)", "A (2)": "A (2) (2)"}
    assert all(type(m) is DeferredMesh for m in meshes)        # not built
    loose, proto = meshes
    assert [a.get("mat") for a in face_attrs(proto)] == [
        "A (2)", "A (2) (2)", "B"]
    assert [a.get("mat") for a in face_attrs(loose)] == [
        "A (2)", "A (2) (2)", None, "B"]


def test_built_and_deferred_end_the_same(path, monkeypatch):
    _r, eager = _merged(path, False, monkeypatch)
    _r, lazy = _merged(path, True, monkeypatch)
    for e, d in zip(eager, lazy):
        assert [f.attrs for f in d.faces] == [f.attrs for f in e.faces]
    loose, proto = lazy
    # The component placed twice: each face renamed once, not twice.
    assert [f.attrs["mat"] for f in proto.faces] == ["A (2)", "A (2) (2)", "B"]
    assert loose.faces[3].attrs["back"] == {"mat": "A (2)",
                                            "color": [1, 0, 0]}


def test_no_collision_leaves_the_tables_alone(path, monkeypatch):
    monkeypatch.setattr(_native, "wanted", lambda: True)
    src = Scene()
    igz.load_into(src, path)
    comp = _component(src)
    sources = [g.mesh.__dict__["_deferred_source"]
               for g, _m in iter_placements(comp)]
    assert merge_materials(_source_materials(), {}, comp) == {}
    assert [g.mesh.__dict__["_deferred_source"]
            for g, _m in iter_placements(comp)] == sources
