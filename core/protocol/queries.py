# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Read queries of the core protocol (docs/core-protocol.md §3).

Every answer is JSON-safe: element keys as decimal strings, meshes by
mesh key, placements by uid. Nothing here writes the document, takes an
undo snapshot or materialises a store-backed mesh."""
from __future__ import annotations

from core.protocol.keys import (KINDS, element, elements, from_json_key,
                                kind_of, mesh_key, resolve_mesh, store_of,
                                to_json_key)


def _xyz(p):
    return [float(p.x()), float(p.y()), float(p.z())]


def mesh_info(scene, mesh=None) -> dict:
    """``{mesh, backing, vertices, edges, faces, revision}`` of a mesh
    (``mesh``: a reference :func:`resolve_mesh` takes)."""
    m = resolve_mesh(scene, mesh)
    st = store_of(m)
    if st is not None:
        nv, ne, nf = st.counts()
    else:
        nv, ne, nf = len(m.vertices), len(m.edges), len(m.faces)
    return {"mesh": mesh_key(m), "backing": "store" if st is not None else "python",
            "vertices": nv, "edges": ne, "faces": nf,
            "active": m is scene.mesh,
            "revision": st.revision if st is not None else m._mut_serial}


def faces(scene, keys, mesh=None) -> dict:
    """Each face by key: its rings' positions (outer ring first, then the
    holes), its corner keys and its normal; unknown keys under
    ``missing``."""
    m = resolve_mesh(scene, mesh)
    keys = [from_json_key(k) for k in keys]
    found = elements(m, "face", keys)
    st = store_of(m)
    out = []
    for k in keys:
        f = found.get(k)
        if f is None:
            continue
        if st is not None:
            rings = [[list(p) for p in ring] for ring in st.face_positions(k)]
            corners = [[to_json_key(v) for v in ring] for ring in st.face_rings(k)]
        else:
            loops = [f.loop] + list(f.hole_loops or [])
            rings = [[_xyz(v.position) for v in loop] for loop in loops]
            corners = [[to_json_key(v.key) for v in loop] for loop in loops]
        out.append({"key": to_json_key(k), "rings": rings, "corners": corners,
                    "normal": _xyz(f.normal())})
    return {"mesh": mesh_key(m), "faces": out,
            "missing": [to_json_key(k) for k in keys if k not in found]}


def edges(scene, keys, mesh=None) -> dict:
    """Each edge by key: its end keys and positions, soft / hidden, its
    faces' keys."""
    m = resolve_mesh(scene, mesh)
    keys = [from_json_key(k) for k in keys]
    found = elements(m, "edge", keys)
    out = []
    for k in keys:
        e = found.get(k)
        if e is None:
            continue
        out.append({"key": to_json_key(k),
                    "ends": [to_json_key(e.v0.key), to_json_key(e.v1.key)],
                    "positions": [_xyz(e.v0.position), _xyz(e.v1.position)],
                    "soft": bool(e.soft), "hidden": bool(getattr(e, "hidden", False)),
                    "faces": [to_json_key(f.key) for f in e.faces]})
    return {"mesh": mesh_key(m), "edges": out,
            "missing": [to_json_key(k) for k in keys if k not in found]}


def vertices(scene, keys, mesh=None) -> dict:
    m = resolve_mesh(scene, mesh)
    keys = [from_json_key(k) for k in keys]
    found = elements(m, "vertex", keys)
    return {"mesh": mesh_key(m),
            "vertices": [{"key": to_json_key(k), "position": _xyz(found[k].position)}
                         for k in keys if k in found],
            "missing": [to_json_key(k) for k in keys if k not in found]}


def selection(scene) -> dict:
    """The selection by keys: placements by uid, elements by (mesh key,
    kind, element key) — elements are those of the open mesh."""
    from core.group import Group
    m = scene.mesh
    placements, out = [], {k: [] for k in KINDS}
    for x in scene.selection:
        if isinstance(x, Group):
            placements.append(x.uid)
            continue
        kind = kind_of(x)
        if kind is not None:
            out[kind].append(x.key)
    return {"mesh": mesh_key(m), "placements": sorted(placements),
            **{k: [to_json_key(v) for v in sorted(out[k])] for k in KINDS}}


def select(scene, *, mesh=None, faces=(), edges=(), vertices=(),
           mode="replace") -> dict:
    """Select elements of the open mesh by key (``mode``: replace, add,
    remove, clear); returns :func:`selection`. Keys that name nothing are
    listed under ``missing``. Selecting changes no document content."""
    if mode not in ("replace", "add", "remove", "clear"):
        raise ValueError("mode must be replace, add, remove or clear")
    m = resolve_mesh(scene, mesh)
    if m is not scene.mesh:
        raise ValueError("elements can be selected in the open mesh only")
    picked, missing = [], []
    for kind, keys in (("face", faces), ("edge", edges), ("vertex", vertices)):
        keys = [from_json_key(k) for k in keys or ()]
        found = elements(m, kind, keys)
        picked.extend(found[k] for k in keys if k in found)
        missing.extend(to_json_key(k) for k in keys if k not in found)
    if mode == "clear":
        scene.clear_selection()
    elif mode == "remove":
        scene.select(picked, mode="remove")
    else:
        scene.select(picked, additive=mode == "add")
    return {**selection(scene), "missing": missing}


def feed(history, since_revision=0) -> dict:
    """The change feed after ``since_revision`` as JSON (see feed.py)."""
    got = history.feed.since(since_revision)
    return {"revision": got["revision"], "complete": got["complete"],
            "steps": [s.to_json() for s in got["steps"]]}
