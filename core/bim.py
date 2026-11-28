# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""BIM tagging layer — semantics ON TOP of freeform geometry.

The project's non-negotiable principle: the modeller is push/pull
freedom, and BIM lives as *metadata applied to selected geometry* (the
BlenderBIM pattern), never as rigid parametric primitives. A tag turns a set
of faces (or a whole group) into a named IFC object; tagged objects feed the
quantity takeoff — and later the IFC export to IngePresupuestos. Untagged
geometry stays plain drawing.

Storage:
- loose faces: ``face.attrs["ifc"] = {"id": n, "class": ..., "name": ...}``
  — the ``attrs`` dict already survives the engine's face churn (push/pull,
  rebuilds, heals) and serialises with the document, so tags are durable.
- groups: ``group.ifc = {"class": ..., "name": ...}`` (the group IS the
  object; its isolated mesh gives exact quantities).

Grouping never drops a tag: a tagged group nested in another, and tagged
faces that went into a group's mesh, are objects wherever they sit in the
tree, measured in world space. A tagged group is its geometry less what
its tagged descendants claim, so nothing is counted twice.

Quantities are computed honestly: any face set yields its AREA; VOLUME is
reported only when the set is watertight on its own (every edge of the set
bordered by exactly two faces of the set) — the guarantee the whole engine
exists to provide.
"""
from __future__ import annotations

import math

from built_core import Vec3 as QVector3D

#: Curated civil/architecture classes (IFC4 names).
from core.ifc_tables import IFC_CLASSES

#: Density of reinforcing steel, kg/m³ — the bar's weight is its volume
#: times this, as a takeoff bills it.
STEEL_DENSITY = 7850.0


#: Bumped by :func:`tag_faces` and :func:`untag_faces`, which change a
#: mesh's tags without the mesh's mutation serial knowing.
_TAG_EPOCH = 0


def _compose(base, m):
    if m is None:
        return base
    return m if base is None else base * m


def _placed_groups(groups, base=None):
    """``(group, base)`` for every group down the tree, depth first, with
    ``base`` the matrix its ancestors compose (``None`` while none of them
    carries one)."""
    for g in groups:
        yield g, base
        kids = getattr(g, "children", None)
        if kids:
            yield from _placed_groups(kids, _compose(base, getattr(g, "xform",
                                                                   None)))


def _group_meshes(scene):
    """Every group mesh in the document, each once (copies share theirs)."""
    seen: set = set()
    for g, _base in _placed_groups(scene.groups):
        if id(g.mesh) not in seen:
            seen.add(id(g.mesh))
            yield g.mesh


def _holds_face_tags(mesh) -> bool:
    """Whether any face of ``mesh`` is tagged — a lazy mesh answers from
    the document it was read from, without being built."""
    d = mesh.__dict__
    key = (d.get('_mut_serial'), _TAG_EPOCH, id(d.get('_deferred_source')))
    cached = d.get('_bim_face_tags')
    if cached is None or cached[0] != key:
        source = d.get('_deferred_source')
        if source is not None:
            table = source['meta'].get('face_attrs') or ()
            present = any(a.get('ifc') for a in table)
        else:
            present = any(f.attrs.get('ifc') for f in mesh.faces)
        cached = (key, present)
        d['_bim_face_tags'] = cached
    return cached[1]


def next_object_id(scene) -> int:
    """A fresh BIM object id (max existing + 1) — over the faces inside
    groups too, so ungrouping them never merges two objects."""
    from built_core.mesh import face_attrs
    top = 0
    for mesh in (scene.loose_mesh, *_group_meshes(scene)):
        for attrs in face_attrs(mesh):
            tag = attrs.get("ifc")
            if tag and isinstance(tag.get("id"), int):
                top = max(top, tag["id"])
    return top + 1


def tag_faces(faces, ifc_class: str, name: str, obj_id: int) -> None:
    global _TAG_EPOCH
    _TAG_EPOCH += 1
    faces = list(faces)
    previous = next((f.attrs["ifc"] for f in faces
                     if f.attrs.get("ifc", {}).get("id") == obj_id), {})
    tag = {**previous, "id": obj_id, "class": ifc_class, "name": name}
    for f in faces:
        f.attrs["ifc"] = dict(tag)


def untag_faces(faces) -> None:
    global _TAG_EPOCH
    _TAG_EPOCH += 1
    for f in faces:
        f.attrs.pop("ifc", None)


def tag_group(group, ifc_class: str, name: str) -> None:
    from core.ifc_identity import ensure_guid
    new = not group.ifc
    group.ifc = {**(group.ifc or {}), "class": ifc_class, "name": name}
    # A record tagged again keeps the GlobalId its uid path derives when it
    # has no stored GUID (docs/persistent-ids.md §7.1).
    if new:
        ensure_guid(group.ifc)


def untag_group(group) -> None:
    group.ifc = None


def face_set_area(faces) -> float:
    return sum(f.area() for f in faces)


def _loop_area(loop) -> float:
    """Planar area of a vertex loop (half the Newell normal's length)."""
    n = QVector3D(0.0, 0.0, 0.0)
    count = len(loop)
    for i in range(count):
        curr = loop[i].position
        nxt = loop[(i + 1) % count].position
        n = n + QVector3D(
            (curr.y() - nxt.y()) * (curr.z() + nxt.z()),
            (curr.z() - nxt.z()) * (curr.x() + nxt.x()),
            (curr.x() - nxt.x()) * (curr.y() + nxt.y()),
        )
    return 0.5 * n.length()


def face_net_area(f) -> float:
    """Face area with hole loops (door/window openings) subtracted."""
    net = getattr(f, "net_area", None)
    if callable(net):
        return net()
    area = f.area()
    for h in f.hole_loops:
        area -= _loop_area(h)
    return max(area, 0.0)


def face_set_volume(faces):
    """Volume enclosed by ``faces``, or ``None`` when the set is not
    watertight by itself. Signed tetrahedron sum over the set's triangles;
    closure = every edge of the set borders exactly two faces of the set.

    The tetrahedra are measured from the set's own first vertex and
    accumulated in Python floats (double precision). Summing from the
    world origin in ``QVector3D`` (single precision) instead makes the
    terms cancel out for geometry far from the origin — a column that
    reads exactly at the origin is already wrong by percent at 300 m
    and meaningless at kilometres (#1221). For a closed set the sum is
    the same from any reference point, so the set's own vertex costs
    nothing and keeps every term the size of the set itself."""
    faces = list(faces)
    if not faces:
        return None
    from core._native import qt_hypot
    from built_core.store_mesh import FaceView, StoreMesh
    first = faces[0]
    if isinstance(first, FaceView) and isinstance(first._m, StoreMesh) and qt_hypot():
        if all(isinstance(f, FaceView) and f._m is first._m for f in faces):
            snapshot = first._m._store.snapshot()
            try:
                return snapshot.face_set_volume([f.key for f in faces], qt_hypot())
            except KeyError:
                pass  # The caller included a detached face.
    counts: dict = {}
    for f in faces:
        for lp in (f.loop, *f.hole_loops):
            n = len(lp)
            for i in range(n):
                key = frozenset((lp[i].key, lp[(i + 1) % n].key))
                counts[key] = counts.get(key, 0) + 1
    if any(c != 2 for c in counts.values()):
        return None
    # Convert before subtracting: QVector3D arithmetic itself is float32.
    import math
    origin = faces[0].loop[0].position
    anchor = (float(origin.x()), float(origin.y()), float(origin.z()))
    terms = []
    for f in faces:
        for triangle in f.triangulate():
            a, b, c = [(float(v.x()) - anchor[0], float(v.y()) - anchor[1],
                        float(v.z()) - anchor[2]) for v in triangle]
            terms.append((a[0] * (b[1]*c[2] - b[2]*c[1])
                          + a[1] * (b[2]*c[0] - b[0]*c[2])
                          + a[2] * (b[0]*c[1] - b[1]*c[0])) / 6.0)
    return abs(math.fsum(terms))


def _group_signature(g, base=None, placements=None) -> tuple:
    """What a tagged group's quantities depend on: its tag, the matrix its
    ancestors compose and, for every placement down its tree, the mesh,
    that mesh's mutation serial, the composed matrix and whether it is
    tagged (a tagged descendant is left out). Equal signatures measure the
    same."""
    from built_core.geometry_arrays import owned_placements
    if placements is None:
        placements = owned_placements(g, base)
    tag = tuple(sorted((k, repr(v)) for k, v in (g.ifc or {}).items()))
    parts = []
    for placed, m in placements:
        mesh = placed.mesh
        parts.append((id(mesh), mesh.__dict__.get("_mut_serial"),
                      None if m is None else tuple(m.data()),
                      placed is not g and bool(getattr(placed, "ifc", None))))
    return (id(g), tag, tuple(parts))


def _untagged_part(g, excluded=()):
    """``g`` with its tagged descendants pruned — a stand-in
    :func:`built_core.group.world_mesh` folds like the group itself."""
    from types import SimpleNamespace
    return SimpleNamespace(
        mesh=g.mesh, xform=getattr(g, "xform", None),
        children=[_untagged_part(c, excluded) for c in getattr(g, "children", None)
                  or () if not getattr(c, "ifc", None) and c.uid not in excluded])


def _tagged_group_faces(g, base, excluded=()) -> list:
    """A tagged group's world-space faces: its own geometry and that of
    its nested placements, less the tagged ones (objects of their own)."""
    if (base is None and getattr(g, "xform", None) is None
            and not getattr(g, "children", None)):
        return list(g.mesh.faces)
    from types import SimpleNamespace

    from built_core.group import world_mesh
    from built_core.mesh import Mesh
    part = _untagged_part(g, excluded)
    if base is not None:
        part = SimpleNamespace(mesh=Mesh(), xform=base, children=[part])
    return list(world_mesh(part).faces)


def _moved_faces(faces, m) -> list:
    """``faces`` mapped through ``m`` into a mesh of their own, welded so a
    closed set stays closed for :func:`face_set_volume`."""
    from built_core.mesh import Mesh
    out = Mesh()
    moved = []
    for f in faces:
        nf = out.add_face([m.map(v) for v in f.vertices],
                          [[m.map(v) for v in h] for h in f.holes] or None)
        if nf is not None:
            nf.attrs.update(f.attrs)
            moved.append(nf)
    return moved


def _face_tags_in(mesh, memo, seen) -> dict:
    """``{id: (tag, indices)}`` of the tagged faces of a group's ``mesh``.
    With a ``memo``, a mesh known to hold none is not scanned again until
    it or a tag changes — the takeoff refreshes on every edit."""
    key = None
    if memo is not None:
        key = ("untagged", id(mesh), mesh.__dict__.get("_mut_serial"),
               _TAG_EPOCH)
        seen.add(key)
        if key in memo:
            return {}
    by_id: dict = {}
    if _holds_face_tags(mesh):
        facts_key = mesh.__dict__['_bim_face_tags'][0]
        cached = mesh.__dict__.get('_bim_tag_indices')
        if cached is not None and cached[0] == facts_key:
            return cached[1]
        from built_core.geometry_arrays import face_attributes
        for index, attrs in enumerate(face_attributes(mesh)):
            tag = attrs.get("ifc")
            if tag:
                by_id.setdefault(tag.get("id"), (tag, []))[1].append(index)
        mesh.__dict__['_bim_tag_indices'] = (facts_key, by_id)
    if key is not None and not by_id:
        memo[key] = True
    return by_id


def collect_objects(scene, memo: dict | None = None) -> list[dict]:
    """Every tagged BIM object in the document, with live quantities:
    ``{"key", "class", "name", "area", "volume" (or None), "faces" | "arrays"}``.
    Loose-face objects group by tag id; each tagged group is one object, at
    any depth of the tree. Tagged faces inside a group's mesh are one
    object per group and tag id: ``"arrays"`` (world space) plus ``"owner"``
    (the group) and ``"tag_id"`` — a tagged group's own mesh is that group's
    object, its face tags are not read twice.

    ``memo`` (a dict the caller keeps between calls) reuses a tagged
    group's object while its :func:`_group_signature` is unchanged, so an
    edit re-measures only what it touched (#857: 238 rebar bars took 25 s
    each time); entries no longer in the model are dropped from it."""
    out: list[dict] = []
    by_id: dict = {}
    # An untagged deferred root can be large too; metadata answers first.
    loose_faces = scene.loose_mesh.faces if _holds_face_tags(scene.loose_mesh) else ()
    for f in loose_faces:
        tag = f.attrs.get("ifc")
        if tag:
            by_id.setdefault(tag.get("id"), (tag, []))[1].append(f)
    for obj_id, (tag, faces) in sorted(by_id.items(),
                                       key=lambda kv: kv[0] or 0):
        out.append({
            "key": ("faces", obj_id),
            "class": tag.get("class", ""),
            "name": tag.get("name", ""),
            "area": face_set_area(faces),
            "volume": face_set_volume(faces),
            "faces": faces,
        })
    seen = set()
    for g, base in _placed_groups(scene.groups):
        tag = getattr(g, "ifc", None)
        if not tag:
            found = _face_tags_in(g.mesh, memo, seen)
            if not found:
                continue            # most groups: no matrix to compose
            m = _compose(base, getattr(g, "xform", None))
            from built_core.geometry_arrays import selected_arrays, quantities, face_attributes
            attributes = face_attributes(g.mesh)
            for obj_id, (ftag, indices) in sorted(
                    found.items(), key=lambda kv: kv[0] or 0):
                arrays = selected_arrays(g.mesh, indices, m)
                area, _, volume, _, _ = quantities(arrays)
                out.append({
                    "key": ("group-faces", id(g), obj_id),
                    "class": ftag.get("class", ""),
                    "name": ftag.get("name", ""),
                    "area": area,
                    "volume": volume,
                    "arrays": lambda arrays=arrays: arrays,
                    "attributes": [attributes[i] for i in indices],
                    "owner": g,
                    "tag_id": obj_id,
                })
            continue
        from built_core.geometry_arrays import owned_placements
        placements = list(owned_placements(g, base))
        sig = None
        if memo is not None:
            sig = _group_signature(g, base, placements)
            seen.add(sig)
            hit = memo.get(sig)
            if hit is not None and hit.get("group") is g:
                out.append(hit)
                continue
        # Quantities are world-space, and a component's nested placements
        # are part of the object being measured.
        from built_core.geometry_arrays import world_arrays, placement_quantities
        geometry = lambda g=g, placements=placements: world_arrays(g, placements=placements)
        area, _, volume, _, _ = placement_quantities(placements)
        obj = {
            "key": ("group", id(g)),
            "class": tag.get("class", ""),
            "name": tag.get("name", ""),
            "area": area,
            "volume": volume,
            "arrays": geometry,
            "group": g,
        }
        if memo is not None:
            memo[sig] = obj
        out.append(obj)
    if memo is not None:
        for sig in [k for k in memo if k not in seen]:
            del memo[sig]
    return out


#: Standard IFC4 quantity-set name per class (falls back to BaseQuantities).
QSET_NAMES = {
    "IfcWall": "Qto_WallBaseQuantities",
    "IfcSlab": "Qto_SlabBaseQuantities",
    "IfcColumn": "Qto_ColumnBaseQuantities",
    "IfcBeam": "Qto_BeamBaseQuantities",
    "IfcFooting": "Qto_FootingBaseQuantities",
    "IfcRoof": "Qto_RoofBaseQuantities",
    "IfcStair": "Qto_StairBaseQuantities",
    "IfcRamp": "Qto_RampBaseQuantities",
    "IfcDoor": "Qto_DoorBaseQuantities",
    "IfcWindow": "Qto_WindowBaseQuantities",
    "IfcRailing": "Qto_RailingBaseQuantities",
    "IfcCovering": "Qto_CoveringBaseQuantities",
    "IfcPile": "Qto_PileBaseQuantities",
    "IfcMember": "Qto_MemberBaseQuantities",
    "IfcSpace": "Qto_SpaceBaseQuantities",
    "IfcReinforcingBar": "Qto_ReinforcingElementBaseQuantities",
}

#: Budget unit each class is measured in (the metrado unit downstream tools
#: like IngePresupuestos bill by): m2 = surface partidas, m3 = concrete,
#: m = linear elements, und = counted units.
METRADO_UNIT = {
    "IfcWall": "m2", "IfcSlab": "m2", "IfcRoof": "m2", "IfcStair": "m2",
    "IfcRamp": "m2", "IfcCovering": "m2", "IfcSpace": "m2",
    "IfcColumn": "m3", "IfcBeam": "m3", "IfcFooting": "m3",
    "IfcPile": "m", "IfcMember": "m", "IfcRailing": "m",
    "IfcDoor": "und", "IfcWindow": "und",
    "IfcReinforcingBar": "kg",
}


def _extents(faces):
    """World-axis extents (dx, dy, dz) of a face set's vertices."""
    xs, ys, zs = [], [], []
    for f in faces:
        for lp in (f.loop, *f.hole_loops):
            for v in lp:
                p = v.position
                xs.append(p.x()), ys.append(p.y()), zs.append(p.z())
    if not xs:
        return 0.0, 0.0, 0.0
    return (max(xs) - min(xs), max(ys) - min(ys), max(zs) - min(zs))


def _upward_net_area(faces, closed: bool) -> float:
    """Net area of the upward-facing faces (roof slopes, stair treads).
    On a watertight set normals are outward-consistent so the sign is
    trusted; on open sheets the winding is arbitrary, so |nz| decides."""
    total = 0.0
    for f in faces:
        nz = f.normal().z()
        if (nz if closed else abs(nz)) > 0.2:
            total += face_net_area(f)
    return total


def _face_width_height(f):
    """In-plane horizontal and vertical extents of a face — the honest
    OverallWidth/OverallHeight of a door or window drawn in any wall
    orientation. Falls back to bbox extents for horizontal faces."""
    n = f.normal()
    u = QVector3D.crossProduct(QVector3D(0.0, 0.0, 1.0), n)
    if u.length() < 1e-6:                       # face lies flat
        dx, dy, dz = _extents([f])
        return max(dx, dy), dz
    u = u.normalized()
    us = [QVector3D.dotProduct(v.position, u) for v in f.loop]
    zs = [v.position.z() for v in f.loop]
    return max(us) - min(us), max(zs) - min(zs)


def class_quantities(ifc_class: str, faces):
    """Standard per-class BaseQuantities from freeform geometry, honestly:
    every value is measured off the tagged faces (volume only when the set
    is watertight on its own), named as the IFC4 ``Qto_*`` sets so any
    downstream consumer — IngePresupuestos included — reads the metrado
    straight from the file.

    Returns ``(qset_name, entries, metrado)`` where ``entries`` is a list of
    ``(kind, name, value)`` with kind in {"area", "volume", "length",
    "weight"} and
    ``metrado`` is ``(value, unit)`` in the class's budget unit (value may
    be ``None`` when not computable, e.g. volume of an open set)."""
    faces = list(faces)
    unit = METRADO_UNIT.get(ifc_class, "m2")
    if not faces:
        return QSET_NAMES.get(ifc_class, "BaseQuantities"), [], (None, unit)
    volume = face_set_volume(faces)
    closed = volume is not None
    total = face_set_area(faces)
    dx, dy, dz = _extents(faces)
    big = max(faces, key=lambda f: f.area())
    entries: list[tuple[str, str, float]] = []

    def add(kind, name, value):
        if value is not None and value > 1e-9:
            entries.append((kind, name, float(value)))

    metrado = None
    if ifc_class == "IfcWall":
        side_gross = big.area()
        side_net = face_net_area(big)
        add("length", "Height", dz)
        if dz > 1e-9:
            add("length", "Length", side_gross / dz)
        if side_net > 1e-9 and closed:
            add("length", "Width", volume / side_net)
        add("area", "GrossSideArea", side_gross)
        add("area", "NetSideArea", side_net)
        add("volume", "GrossVolume", volume)
        metrado = side_net
    elif ifc_class == "IfcSlab":
        gross = big.area()
        net = face_net_area(big)
        perim = sum((big.loop[i].position
                     - big.loop[(i + 1) % len(big.loop)].position).length()
                    for i in range(len(big.loop)))
        add("length", "Perimeter", perim)
        if net > 1e-9 and closed:
            add("length", "Width", volume / net)
        add("area", "GrossArea", gross)
        add("area", "NetArea", net)
        add("volume", "GrossVolume", volume)
        metrado = net
    elif ifc_class in ("IfcRoof", "IfcStair", "IfcRamp"):
        up = _upward_net_area(faces, closed)
        add("length", "Length", max(dx, dy))
        add("area", "GrossArea", up)
        add("volume", "GrossVolume", volume)
        metrado = up
    elif ifc_class == "IfcColumn":
        add("length", "Length", dz)
        if dz > 1e-9 and closed:
            add("area", "CrossSectionArea", volume / dz)
        add("volume", "GrossVolume", volume)
        metrado = volume
    elif ifc_class == "IfcBeam":
        length = max(dx, dy, dz)
        add("length", "Length", length)
        if length > 1e-9 and closed:
            add("area", "CrossSectionArea", volume / length)
        add("volume", "GrossVolume", volume)
        metrado = volume
    elif ifc_class == "IfcFooting":
        add("length", "Height", dz)
        add("area", "GrossSurfaceArea", total)
        add("volume", "GrossVolume", volume)
        metrado = volume
    elif ifc_class == "IfcPile":
        add("length", "Length", dz)
        add("volume", "GrossVolume", volume)
        metrado = dz
    elif ifc_class in ("IfcMember", "IfcRailing"):
        length = max(dx, dy) if ifc_class == "IfcRailing" else max(dx, dy, dz)
        add("length", "Length", length)
        add("volume", "GrossVolume", volume)
        metrado = length
    elif ifc_class == "IfcReinforcingBar":
        # A straight bar (#495): its length is the longest extent, its
        # diameter the round section that holds its volume, its weight the
        # steel in it -- reinforcement is billed by the kilo.
        length = max(dx, dy, dz)
        add("length", "Length", length)
        if closed and length > 1e-9:
            add("length", "NominalDiameter",
                2.0 * (volume / (math.pi * length)) ** 0.5)
            add("weight", "Weight", volume * STEEL_DENSITY)
            metrado = volume * STEEL_DENSITY
    elif ifc_class == "IfcCovering":
        net = sum(face_net_area(f) for f in faces)
        add("area", "GrossArea", total)
        add("area", "NetArea", net)
        metrado = net
    elif ifc_class in ("IfcDoor", "IfcWindow"):
        width, height = _face_width_height(big)
        add("length", "Width", width)
        add("length", "Height", height)
        add("area", "Area", face_net_area(big))
        metrado = 1.0
    elif ifc_class == "IfcSpace":
        floor = max((face_net_area(f) for f in faces
                     if abs(f.normal().z()) > 0.7), default=0.0)
        add("length", "Height", dz)
        add("area", "NetFloorArea", floor)
        add("volume", "GrossVolume", volume)
        metrado = floor
    else:
        add("area", "GrossArea", total)
        add("volume", "GrossVolume", volume)
        metrado = total
    if metrado is not None and metrado <= 1e-9:
        metrado = None
    return QSET_NAMES.get(ifc_class, "BaseQuantities"), entries, (metrado, unit)


def class_quantities_arrays(ifc_class, arrays):
    """Class quantities from placed rings; no Python geometry is constructed."""
    import numpy as np
    from built_core.geometry_arrays import Arrays, assembled, measurements
    a = arrays if isinstance(arrays, Arrays) else assembled(arrays)
    unit = METRADO_UNIT.get(ifc_class, "m2")
    data = measurements(a)
    if data is None:
        return QSET_NAMES.get(ifc_class, "BaseQuantities"), [], (None, unit)
    gross, net, normals, volume, closed, starts = data
    total = float(gross.sum())
    dx, dy, dz = np.ptp(a.pos[a.corners], axis=0)
    big = int(np.argmax(gross))
    ring = int(np.r_[0, np.cumsum(a.ring_counts)[:-1]][big])
    points = a.pos[a.corners[starts[big]:starts[big] + a.ring_sizes[ring]]]
    perim = float(np.linalg.norm(points - np.roll(points, -1, axis=0), axis=1).sum())
    u = np.cross([0., 0., 1.], normals[big])
    length = np.linalg.norm(u)
    height = float(np.ptp(points[:, 2]))
    if length >= 1e-6:
        width_height = float(np.ptp(points @ (u / length))), height
    else:
        width_height = float(np.ptp(points, axis=0)[:2].max()), height
    entries: list[tuple[str, str, float]] = []

    def add(kind, name, value):
        if value is not None and value > 1e-9:
            entries.append((kind, name, float(value)))

    metrado = None
    if ifc_class == "IfcWall":
        side_gross = float(gross[big])
        side_net = float(net[big])
        add("length", "Height", dz)
        if dz > 1e-9:
            add("length", "Length", side_gross / dz)
        if side_net > 1e-9 and closed:
            add("length", "Width", volume / side_net)
        add("area", "GrossSideArea", side_gross)
        add("area", "NetSideArea", side_net)
        add("volume", "GrossVolume", volume)
        metrado = side_net
    elif ifc_class == "IfcSlab":
        gross = float(gross[big])
        net = float(net[big])
        add("length", "Perimeter", perim)
        if net > 1e-9 and closed:
            add("length", "Width", volume / net)
        add("area", "GrossArea", gross)
        add("area", "NetArea", net)
        add("volume", "GrossVolume", volume)
        metrado = net
    elif ifc_class in ("IfcRoof", "IfcStair", "IfcRamp"):
        up = float(net[(normals[:, 2] if closed else abs(normals[:, 2])) > .2].sum())
        add("length", "Length", max(dx, dy))
        add("area", "GrossArea", up)
        add("volume", "GrossVolume", volume)
        metrado = up
    elif ifc_class == "IfcColumn":
        add("length", "Length", dz)
        if dz > 1e-9 and closed:
            add("area", "CrossSectionArea", volume / dz)
        add("volume", "GrossVolume", volume)
        metrado = volume
    elif ifc_class == "IfcBeam":
        length = max(dx, dy, dz)
        add("length", "Length", length)
        if length > 1e-9 and closed:
            add("area", "CrossSectionArea", volume / length)
        add("volume", "GrossVolume", volume)
        metrado = volume
    elif ifc_class == "IfcFooting":
        add("length", "Height", dz)
        add("area", "GrossSurfaceArea", total)
        add("volume", "GrossVolume", volume)
        metrado = volume
    elif ifc_class == "IfcPile":
        add("length", "Length", dz)
        add("volume", "GrossVolume", volume)
        metrado = dz
    elif ifc_class in ("IfcMember", "IfcRailing"):
        length = max(dx, dy) if ifc_class == "IfcRailing" else max(dx, dy, dz)
        add("length", "Length", length)
        add("volume", "GrossVolume", volume)
        metrado = length
    elif ifc_class == "IfcReinforcingBar":
        # A straight bar (#495): its length is the longest extent, its
        # diameter the round section that holds its volume, its weight the
        # steel in it -- reinforcement is billed by the kilo.
        length = max(dx, dy, dz)
        add("length", "Length", length)
        if closed and length > 1e-9:
            add("length", "NominalDiameter",
                2.0 * (volume / (math.pi * length)) ** 0.5)
            add("weight", "Weight", volume * STEEL_DENSITY)
            metrado = volume * STEEL_DENSITY
    elif ifc_class == "IfcCovering":
        net = float(net.sum())
        add("area", "GrossArea", total)
        add("area", "NetArea", net)
        metrado = net
    elif ifc_class in ("IfcDoor", "IfcWindow"):
        width, height = width_height
        add("length", "Width", width)
        add("length", "Height", height)
        add("area", "Area", float(net[big]))
        metrado = 1.0
    elif ifc_class == "IfcSpace":
        floor = float(np.max(net[abs(normals[:, 2]) > .7], initial=0))
        add("length", "Height", dz)
        add("area", "NetFloorArea", floor)
        add("volume", "GrossVolume", volume)
        metrado = floor
    else:
        add("area", "GrossArea", total)
        add("volume", "GrossVolume", volume)
        metrado = total
    if metrado is not None and metrado <= 1e-9:
        metrado = None
    return QSET_NAMES.get(ifc_class, "BaseQuantities"), entries, (metrado, unit)



def object_quantities(obj):
    """Dispatch legacy loose-face objects and array-backed groups."""
    if "arrays" in obj:
        return class_quantities_arrays(obj["class"], obj["arrays"]())
    return class_quantities(obj["class"], obj["faces"])


def quantities_csv(scene, schedule=None) -> str:
    """The takeoff table (metrado) as CSV — the pre-IFC bridge to
    IngePresupuestos: model → budget quantities in one export. ``metrado``
    is the per-class budget measure (wall face m², column m³, pile m,
    door und); area/volume stay as the raw shell numbers."""
    if schedule:
        import csv
        import io
        stream = io.StringIO()
        csv.writer(stream, lineterminator="\n").writerows(junction_schedules(scene)[schedule])
        return stream.getvalue()
    lines = ["class,name,metrado,unit,area_m2,volume_m3"]
    for obj in collect_objects(scene):
        _, _, (metrado, unit) = object_quantities(obj)
        met = "" if metrado is None else f"{metrado:.4f}"
        vol = "" if obj["volume"] is None else f"{obj['volume']:.4f}"
        name = (obj["name"] or "").replace('"', "'")
        lines.append(f"{obj['class']},\"{name}\",{met},{unit},"
                     f"{obj['area']:.4f},{vol}")
    return "\n".join(lines) + "\n"


class JunctionInventory:
    """Resolve the evaluated junction tree and Repeat stations to placed parts."""

    def __init__(self, scene, rows):
        self.rows = {r["uid"]: r for r in rows if r["kind"] == "junction"}
        groups = scene.groups_by_uid()
        for g in groups.values():
            record = g.ifc or {}
            row = record.get("junction_export")
            if row:
                self.rows.setdefault(row["uid"], row)
            elif isinstance(record.get("connects"), dict):
                # J0 release records predate the detail snapshot.
                connection = record["connects"]
                roles = {"main": {"placement": connection.get("relating")}}
                roles.update({str(i): {"placement": other} for i, other in enumerate(connection.get("related", []))})
                self.rows[g.uid] = dict(uid=g.uid, owner=g.uid, roles=roles, state="ok",
                                        contents=[dict(placement=p) for p in connection.get("realizing", [])],
                                        detail={"name": connection.get("type") or g.name})
        self.parts = {}
        self.assemblies = {}
        self.hidden = set()
        self.top = {}
        self.members = {}
        repeat_sources = {r["uid"]: r.get("source") for r in rows if r["kind"] == "repeat"}
        owners = {r["owner"]: uid for uid, r in self.rows.items()}
        for uid, row in self.rows.items():
            self.assemblies[row["owner"]] = uid
            if row.get("parent") in self.rows:
                self.top[uid] = row["parent"]
                self.hidden.add(row["owner"])
            roles = row.get("roles", {})
            first = next((k for k in ("main", "host", "ply_1", "a") if k in roles),
                         min(roles, default=None))
            def member(role):
                placement = role.get("placement")
                return placement[-1] if isinstance(placement, list) else placement
            self.members[uid] = ([member(roles[first])] +
                                 [member(v) for k, v in roles.items() if k != first]) if first else []
            for c in row.get("contents", []):
                placed = c.get("placement")
                child_uid = c.get("junction") or owners.get(placed)
                if child_uid and child_uid != uid:
                    self.top[child_uid] = uid
                    self.hidden.add(placed)
                elif c.get("repeat") and placed in groups:
                    parent = groups[placed]
                    source = repeat_sources.get(c["repeat"]) or (parent.ifc or {}).get("junction_source")
                    child = owners.get(source)
                    if child:
                        self.hidden.add(placed)
                        for copy in parent.children:
                            self.hidden.add(copy.uid)
                            for part, _ in _placed_groups(copy.children):
                                if part.ifc:
                                    self.parts[part.uid] = child
                elif placed:
                    self.parts[placed] = uid
        self.roots = {}
        for uid in self.rows:
            top = uid
            seen = set()
            while top in self.top and top not in self.roots:
                if top in seen:
                    raise ValueError("Cyclic junction ownership")
                seen.add(top)
                top = self.top[top]
            self.roots[uid] = self.roots.get(top, top)
        self.realizers = {uid: [] for uid in self.rows}
        for placed, uid in self.parts.items():
            while uid in self.rows:
                self.realizers[uid].append(placed)
                uid = self.top.get(uid)
        for uid, row in self.rows.items():
            if row.get("meeting") == "hosted" and not self.realizers[uid]:
                self.hidden.add(row["owner"])


def _section_area(rings):
    """Stock area in mm² from the profile's directed lines and circular arcs."""
    area = 0.
    for index, ring in enumerate(rings):
        twice_area = 0.
        for ((x, y), centre), ((u, v), _) in zip(ring, ring[1:] + ring[:1]):
            twice_area += x*v - y*u
            if centre is not None:
                a, b = x-centre[0], y-centre[1]
                c, d = u-centre[0], v-centre[1]
                angle = math.atan2(a*d - b*c, a*c + b*d)
                twice_area += (a*a + b*b) * (angle - math.sin(angle))
        area += abs(twice_area) / 2 * (1 if index == 0 else -1)
    return area


def junction_schedules(scene):
    """Header and data cells for delivery CSVs and placed schedule tables."""
    from collections import defaultdict
    from built_core.command_protocol import live_holes, live_rows, sweep_profile
    source = getattr(scene, "_delivery_source", scene)
    rows = live_rows(source)
    inventory = JunctionInventory(scene, rows)
    groups = scene.groups_by_uid()
    sweeps = {r["owner"]: r for r in rows if r["kind"] == "sweep"}
    sections = {}
    bolts, plates, welds, holes, uses = (defaultdict(list) for _ in range(5))

    def detail(uid):
        return (inventory.rows[uid].get("detail") or {}).get("name") or "Custom junction"

    def section(uid):
        if uid not in sections:
            try:
                sections[uid] = sweep_profile(sweeps[uid])
            except (KeyError, ValueError):
                sections[uid] = {}
        return sections[uid]

    def stock_length(uid):
        points = sweeps.get(uid, {}).get("path", {}).get("points", [])
        return round(1000 * sum(math.dist(a, b) for a, b in zip(points, points[1:])), 8) if len(points) > 1 else None

    for uid, j in inventory.rows.items():
        if j["owner"] in groups:
            uses[(detail(uid), (j.get("detail") or {}).get("rev", ""), j["state"])].append(bool(j.get("overrides")))
    for uid, child in inventory.parts.items():
        if uid not in groups:
            continue
        r = groups[uid].ifc or {}
        root = inventory.roots[child]
        if r.get("class") == "IfcMechanicalFastener":
            bolts[(r.get("standard", ""), r.get("grade", ""), r.get("d", ""), r.get("length", ""))].append(root)
        elif r.get("class") == "IfcFastener" and r.get("predefined_type") == "WELD":
            welds[(r.get("weld_type", r.get("type", "")), r.get("throat", ""))].append((root, r.get("length", 0)))
        elif r.get("class") == "IfcPlate":
            profile = section(uid)
            length = stock_length(uid)
            if profile and profile["family"] != "rectangle":
                dims = (profile.get("web") or "", profile["width"], length, profile["name"])
                area = _section_area(profile["rings"])
                volume = area * length if length is not None else None
            else:
                dims = (length if length is not None else r.get("thickness", ""),
                        profile.get("width", r.get("width", "")), profile.get("height", r.get("length", "")), "")
                volume = math.prod(dims[:3]) if all(isinstance(d, (float, int)) for d in dims[:3]) else None
            plates[(r.get("material", ""), *dims, volume)].append(root)
    for cut in rows:
        hole = cut.get("cutter", {}).get("hole")
        if not hole or cut["owner"] not in groups:
            continue
        owner = cut["owner"]
        ply = groups[owner].ifc or {}
        thickness = ply.get("thickness")
        if ply.get("class") == "IfcPlate" and section(owner).get("family") == "rectangle":
            thickness = stock_length(owner)
        joint = inventory.rows.get(cut.get("junction"), {})
        for role in joint.get("roles", {}).values():
            if role.get("placement") == owner:
                face = str(role.get("face", ""))
                if face.startswith(("flange", "web")):
                    thickness = section(owner).get(face.split("_")[0]) or thickness
        for _, a, b, _ in live_holes(source, cut["uid"]):
            depth = thickness if thickness is not None else (
                round(math.dist(a, b)*1000, 4) if hole.get("depth") == "through" else "")
            holes[(ply.get("material", ""), depth, hole["d"])].append(cut["uid"])

    def names(owners):
        return ", ".join(sorted({detail(j) for j in owners}))

    tables = {}
    if bolts:
        tables["fasteners"] = [["Diameter (mm)", "Count", "Length (mm)", "Standard", "Grade", "Details", "Junction count"]]
        for k, v in sorted(bolts.items(), key=str):
            standard, grade, diameter, length = k
            tables["fasteners"].append([diameter, len(v), length, standard, grade, names(v), len(set(v))])
    if plates:
        tables["plates"] = [["Material", "Thickness (mm)", "Width (mm)", "Length (mm)", "Count", "Unit mass (kg)", "Total mass (kg)", "Profile", "Details"]]
        for k, v in sorted(plates.items(), key=str):
            material, thickness, width, length, profile, volume = k
            steel = material in ("S235", "S275", "S355", "Steel", "A992")
            mass = volume * STEEL_DENSITY / 1e9 if steel and volume is not None else None
            tables["plates"].append([material, thickness, width, length, len(v), mass,
                                     mass * len(v) if mass is not None else None, profile, names(v)])
    if welds:
        tables["welds"] = [["Type", "Throat (mm)", "Length (m)", "Count", "Details"]]
        for k, v in sorted(welds.items(), key=str):
            tables["welds"].append([*k, sum(length for _, length in v)/1000, len(v), names(root for root, _ in v)])
    if holes:
        tables["holes"] = [["Material", "Ply thickness (mm)", "Diameter (mm)", "Count"],
                           *[[*k, len(v)] for k, v in sorted(holes.items(), key=str)]]
    if uses:
        tables["junctions"] = [["Detail", "Revision", "Status", "Uses", "Overridden"],
                               *[[*k, len(v), sum(v)] for k, v in sorted(uses.items(), key=str)]]
    return tables
