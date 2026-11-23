# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Stable keys of the core protocol (docs/core-protocol.md §2).

- **Mesh**: ``"m<n>"``, given to a mesh the first time the protocol names
  it and kept for the mesh's lifetime in this session (a mesh object is
  the geometry a placement, a definition or the loose model draws; a
  copy is another mesh with another key). Not saved with the document.
- **Element**: ``(mesh key, element key)``. The element key is the int
  ``Vertex.key`` / ``Edge.key`` / ``Face.key`` the kernel already carries:
  unique in the process, kept across undo/redo, remove/relink and the
  store's restores. In JSON it travels as a decimal string (store keys
  pass 2**53, which a JavaScript client would round).
- **Placement**: ``Group.uid`` (saved with the document).
- **Tag**: ``Layer.key``. **Material**: its name in ``scene.materials``.

Never ``id()``, never a Python object across the boundary."""
from __future__ import annotations

import itertools
import weakref

_counter = itertools.count(1)
_key_of: "weakref.WeakKeyDictionary" = weakref.WeakKeyDictionary()
_mesh_of: "weakref.WeakValueDictionary" = weakref.WeakValueDictionary()

KINDS = ("vertex", "edge", "face")


def mesh_key(mesh) -> str:
    """The protocol key of ``mesh`` (given on first use)."""
    mesh = getattr(mesh, "_instance_definition", mesh)
    k = _key_of.get(mesh)
    if k is None:
        k = f"m{next(_counter)}"
        _key_of[mesh] = k
        _mesh_of[k] = mesh
    return k


def known_mesh(key: str):
    """The mesh a key was given to, while it lives; else None."""
    return _mesh_of.get(key)


def resolve_mesh(scene, ref=None):
    """A mesh from a protocol reference: ``None`` / ``"active"`` (the mesh
    being edited, ``scene.mesh``), ``"loose"``, a mesh key, or a placement
    uid (the mesh that placement draws). ``KeyError`` when unknown."""
    if ref is None or ref == "active":
        return scene.mesh
    if ref == "loose":
        return getattr(scene, "loose_mesh", scene.mesh)
    if isinstance(ref, (list, tuple)):
        group = scene.resolve_occurrence(ref)
        if group is not None:
            return group.mesh
    if not isinstance(ref, str):
        raise KeyError(ref)
    m = known_mesh(ref)
    if m is not None:
        return m
    from core.group import iter_placements
    for root in scene.groups:
        for g, _matrix in iter_placements(root):
            if g.uid == ref:
                return g.mesh
    raise KeyError(ref)


def kind_of(element) -> str | None:
    """``"vertex"``, ``"edge"``, ``"face"`` or None (not a mesh element)."""
    from core.mesh import Edge, Face, Vertex
    if isinstance(element, Face):
        return "face"
    if isinstance(element, Edge):
        return "edge"
    if isinstance(element, Vertex):
        return "vertex"
    return None


def store_of(mesh):
    """The ``MeshStore`` behind a store-backed mesh, else None."""
    from core.store_mesh import StoreMesh
    return mesh.__dict__["_store"] if mesh.__class__ is StoreMesh else None


def element(mesh, kind: str, key: int):
    """The live element of ``mesh`` with ``key``, or None. A store-backed
    mesh answers without listing its elements."""
    if kind not in KINDS:
        raise ValueError(f"kind must be one of {KINDS}")
    st = store_of(mesh)
    if st is not None:
        from core import store_mesh as S
        try:
            if not st.is_live(key) or (key >> 38) & 3 != KINDS.index(kind):
                return None
        except (OverflowError, TypeError):
            return None
        view = (S.VertexView, S.EdgeView, S.FaceView)[KINDS.index(kind)]
        return view(mesh, key)
    seq = {"vertex": mesh.vertices, "edge": mesh.edges, "face": mesh.faces}[kind]
    return next((x for x in seq if x.key == key), None)


def elements(mesh, kind: str, keys) -> dict:
    """``{key: element}`` for the live ones among ``keys`` (one pass over a
    Python mesh's list, direct lookups on a store)."""
    keys = [int(k) for k in keys]
    if store_of(mesh) is not None:
        out = {}
        for k in keys:
            x = element(mesh, kind, k)
            if x is not None:
                out[k] = x
        return out
    want = set(keys)
    seq = {"vertex": mesh.vertices, "edge": mesh.edges, "face": mesh.faces}[kind]
    return {x.key: x for x in seq if x.key in want}


def to_json_key(key: int) -> str:
    return str(int(key))


def from_json_key(value) -> int:
    """An element key from JSON: a decimal string (or an int)."""
    if type(value) is int:
        return value
    if isinstance(value, str) and value.lstrip("-").isdigit():
        return int(value)
    raise ValueError(f"element key {value!r}: a decimal string")
