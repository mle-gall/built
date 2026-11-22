# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Linked files: another .igz placed in this one as a live reference.

A linked file is a component whose definition is not the host's: it is the
source document, loaded from disk. The source stays a document of its own —
open it, edit it, save it, and every host that links it shows the change
(the window watches the files, :mod:`views.link_watcher`). Inside the host
the linked part moves, turns and scales like any component, but it cannot
be opened for editing there: entering it offers to open the source.

What is stored where
--------------------
* Each placed copy carries ``Group.ext["link"] = {"path": …}`` — the source,
  absolute — and ``"rel"``, the same path relative to the host when it was
  last saved, tried first so a project folder can move as a whole.
* The host's ``plugin_data["links"]`` keeps, per source, the tag overrides
  (below) and whether the source is *packed*.
* Unpacked (the default), the host file does not hold the source's
  geometry: the copies are saved as empty placements and filled in again
  from the source when the host is opened (:func:`resolve`). Packed —
  Blender's «Pack external data» — the geometry is saved in the host like
  any component's and the source is no longer read; unpacking links it
  again.

Tags
----
The source's tags come into the host under their own names prefixed by the
file's («Site / Wood»), so they never mix with the host's tags of the same
name, in a folder named after the file inside «Linked files», where the
source's own folders are rebuilt. Each tag shows as the source has it,
unless it was switched in the host: that switch is an *override*, kept by
the host and never written to the source, and the source's later changes to
that tag no longer reach it. Overrides are cleared per file
(:func:`clear_overrides`).
"""

from __future__ import annotations

import os
import threading
from pathlib import Path

LINK_KEY = "link"                 # Group.ext key on each placed copy
REGISTRY_KEY = "links"            # scene.plugin_data key
LINKED_FOLDER = "Linked files"    # the tag folder all links' folders sit in
SEP = " / "                       # between the file's name and a tag's

_loading = threading.local()      # the sources being loaded (cycle guard)


# ---- what is linked ----------------------------------------------------------

def link_of(group) -> dict | None:
    ext = getattr(group, "ext", None)
    link = ext.get(LINK_KEY) if isinstance(ext, dict) else None
    return link if isinstance(link, dict) and link.get("path") else None


def is_linked(group) -> bool:
    return link_of(group) is not None


def linked_ancestor(group):
    """``group`` or the first group above it that is a linked copy."""
    g = group
    while g is not None:
        if is_linked(g):
            return g
        g = getattr(g, "owner", None)
    return None


def _norm(path) -> str:
    return os.path.normcase(os.path.abspath(os.fspath(path)))


def linked_groups(scene, path=None) -> list:
    """Every linked copy in the document (at any depth), or only the copies
    of ``path``."""
    from core.purge import iter_groups
    want = _norm(path) if path is not None else None
    out = []
    for g in iter_groups(getattr(scene, "groups", None) or ()):
        link = link_of(g)
        if link and (want is None or _norm(link["path"]) == want):
            out.append(g)
    return out


def sources(scene) -> list:
    """The linked source files, each once, in first-placed order."""
    seen, out = set(), []
    for g in linked_groups(scene):
        p = _norm(link_of(g)["path"])
        if p not in seen:
            seen.add(p)
            out.append(p)
    return out


def registry(scene) -> dict:
    data = getattr(scene, "plugin_data", None)
    if not isinstance(data, dict):
        data = scene.plugin_data = {}
    reg = data.get(REGISTRY_KEY)
    if not isinstance(reg, dict):
        reg = data[REGISTRY_KEY] = {}
    return reg


def _entry(scene, path) -> dict:
    reg = registry(scene)
    key = _norm(path)
    entry = reg.get(key)
    if not isinstance(entry, dict):
        entry = reg[key] = {}
    entry.setdefault("overrides", {})
    return entry


def is_packed(scene, path) -> bool:
    return bool(registry(scene).get(_norm(path), {}).get("packed"))


def stem(path) -> str:
    return Path(path).stem


def tag_name(path, tag: str) -> str:
    """The host's name for the source's tag ``tag``."""
    from core.layers import DEFAULT_LAYER
    return stem(path) + SEP + ("Untagged" if tag in (None, "", DEFAULT_LAYER)
                               else tag)


def link_tags(scene, path) -> list:
    """The host tags that stand for ``path``'s tags."""
    prefix = stem(path) + SEP
    return [ly.key for ly in scene.layers if ly.key.startswith(prefix)
            and ly.key in _entry(scene, path).get("tags", ())]


def source_of_tag(scene, name: str):
    """The source file a host tag belongs to, or None for the host's own."""
    for key, entry in registry(scene).items():
        if isinstance(entry, dict) and name in (entry.get("tags") or ()):
            return key
    return None


# ---- loading a source -----------------------------------------------------------

class LinkError(Exception):
    """A source that cannot be linked: missing, unreadable, or a cycle."""


def locate(link: dict, host_path=None) -> Path | None:
    """Where the source is: next to the host as last saved (``rel``), then
    the absolute path. None when neither exists."""
    rel = link.get("rel")
    if rel and host_path is not None:
        cand = (Path(host_path).parent / rel)
        if cand.is_file():
            return cand.resolve()
    p = Path(link["path"])
    return p if p.is_file() else None


def load_source(path, select=None, *, include_nested=True, link_meshes=None):
    """``(definition, source scene)`` for the .igz at ``path``: the whole
    document as one component definition (``core.insert``), its own links
    resolved first. Raises :class:`LinkError`."""
    from core.scene import Scene
    from formats import igz
    key = _norm(path)
    stack = getattr(_loading, "stack", None)
    if stack is None:
        stack = _loading.stack = []
    if key in stack:
        raise LinkError(f"{Path(path).name} links itself (through "
                        f"{' → '.join(Path(p).name for p in stack)})")
    if not Path(path).is_file():
        raise LinkError(f"{path} was not found")
    stack.append(key)
    try:
        src = Scene()
        igz.load_into(src, Path(path), include_links=include_nested, link_meshes=link_meshes)
        # A cycle further down is this link's failure too; a source of the
        # source that is merely missing is not (its copies show empty).
        for err in (getattr(src, "link_errors", None) or {}).values():
            if "links itself" in err:
                raise LinkError(err)
        # The source's tag table outlives the component (which consumes the
        # scene's meshes, not its tags).
        tags = {"layers": [ly.to_dict() for ly in src.layers],
                "folders": list(getattr(src, "layer_folders", ()) or ()),
                "folder_visibility": dict(src.folder_visibility),
                "folder_names": dict(getattr(src, "layer_folder_names", None)
                                     or {}),
                "parents": dict(getattr(src, "folder_parents", {}) or {})}
        from core import bim_registry, structure
        vocabulary = bim_registry.get(src)
        nodes = vocabulary.get('structure', {}).get('nodes', {})
        if nodes:
            tags['structure'] = {key: {**node, 'recipe': structure.resolve(vocabulary, key)}
                                 for key, node in nodes.items()}
        materials = dict(getattr(src, "materials", {}) or {})
        from core.group import Group
        from core.insert import is_scale_figure
        from core.mesh import Mesh
        from PySide6.QtGui import QMatrix4x4
        validate_select(select)
        groups = [g for g in src.groups if not is_scale_figure(g)]
        mesh = src.mesh
        if select is not None:
            from core.facets import build, evaluate
            wanted = set(select['uids']) if 'uids' in select else set(evaluate(build(src), select['filter']))
            selected = []
            def visit(items, parent):
                for group in items:
                    world = parent * (group.xform or QMatrix4x4())
                    if group.uid in wanted:
                        group.xform = world
                        selected.append(group)
                    else:
                        visit(group.children or (), world)
            visit(groups, QMatrix4x4())
            groups, mesh = selected, Mesh()
            mesh.__dict__['_link_geometry'] = ((str(path), 'subset-root'), mesh.revision)
        comp = Group(mesh, name=stem(path))
        comp.adopt(groups)
    except LinkError:
        raise
    except Exception as exc:  # noqa: BLE001 — any reader failure
        raise LinkError(f"{Path(path).name}: {exc}") from exc
    finally:
        stack.remove(key)
    if comp is None:
        from core.group import Group
        comp = Group(name=stem(path))
    if comp.xform is None:
        from PySide6.QtGui import QMatrix4x4
        comp.xform = QMatrix4x4()
    comp.component = True
    return comp, tags, materials


def _relabel_deferred(mesh, rename, shared=None) -> bool:
    """Relabel a mesh the document left deferred (``core.mesh.DeferredMesh``)
    in its tables — the face-attribute rows and the edge tag names —
    without building it: touching ``mesh.faces`` would build all of them,
    and the 1.6 M faces of Maxence's facilities model took 46 s to link that
    way (2026-11-07). The tables are replaced, never edited in place (other
    meshes may share them), and what was derived from them is dropped: the
    lazy plan, and the drawing cache's key and bake (the key hashes the
    tables, so the bake is rebuilt, never shown stale). False for a mesh
    that is already built.

    ``shared`` (a dict the caller keeps for one ``rename``) hands meshes
    that had the same table the same renamed one: tables are only ever
    replaced, so sharing them is safe, and the facilities model's 4353
    deferred meshes have 2726 distinct tables. It also keeps the original
    tables alive, so their ``id`` stays a valid key while it lives."""
    from core.mesh import DeferredMesh
    if type(mesh) is not DeferredMesh:
        return False
    import numpy as np
    if shared is None:
        shared = {}
    d = mesh.__dict__
    src = dict(d["_deferred_source"])
    meta = dict(src.get("meta") or {})
    untagged = rename(None)
    # Faces: each row of the attribute table, and a row for the faces that
    # had none (they were on the default tag).
    nfaces = len(src.get("counts") if src.get("counts") is not None else ())
    fa = src.get("face_attr")
    fa = (np.full(nfaces, -1, dtype=np.int32) if fa is None
          else np.array(fa, copy=True))
    neg = bool(fa.size and (fa < 0).any())
    old = meta.get("face_attrs") or []
    key = ("f", id(old), neg)
    hit = shared.get(key)
    if hit is None:
        table = [dict(row) for row in old]
        for row in table:
            row["layer"] = rename(row.get("layer"))
        if neg:
            table.append({"layer": untagged})
        hit = shared[key] = (old, table)
    table = hit[1]
    if neg:
        fa[fa < 0] = len(table) - 1
    src["face_attr"] = fa
    meta["face_attrs"] = table
    # Edges: the name table, and a name for the untagged ones.
    nedges = len(src["edges"]) if src.get("edges") is not None else 0
    lay = src.get("layers")
    lay = (np.full(nedges, -1, dtype=np.int32) if lay is None
           else np.array(lay, copy=True))
    neg = bool(lay.size and (lay < 0).any())
    old = meta.get("edge_layers") or []
    key = ("e", id(old), neg)
    hit = shared.get(key)
    if hit is None:
        names = [rename(n) for n in old]
        if neg:
            names.append(untagged)
        hit = shared[key] = (old, names)
    names = hit[1]
    if neg:
        lay[lay < 0] = len(names) - 1
    src["layers"] = lay
    meta["edge_layers"] = names
    src["meta"] = meta

    def relabel(m, rename=rename):
        from core.layers import assign_layer, layer_of
        for f in m.faces:
            assign_layer(f, rename(layer_of(f)))
        for e in m.edges:
            assign_layer(e, rename(layer_of(e)))
    _retable_deferred(mesh, src, relabel)
    return True


def _retable_deferred(mesh, src, after_build) -> None:
    """Give a deferred mesh new source tables ``src`` (a copy: the old ones
    may be shared) and drop what was derived from the old ones: the lazy
    plan, and the drawing cache's key and bake (the key hashes the tables,
    so the bake is rebuilt, never shown stale). The build the loader left
    reads its own copy of the original tables; if the mesh is ever built
    (an edit, a pick into it), ``after_build(mesh)`` brings its faces and
    edges to what the new tables say — the drawing already shows that."""
    d = mesh.__dict__
    # What the loader's build reads (the original tables), for a
    # store-backed first touch that replays build + hooks on its own.
    d.setdefault("_deferred_hook_source", d["_deferred_source"])
    d["_deferred_source"] = src
    for key in ("_deferred_plan", "_drawing_staged", "_drawing_source_key"):
        d.pop(key, None)
    original = d["_deferred"]

    def build(m, original=original, after_build=after_build):
        original(m)
        after_build(m)
    d["_deferred"] = build
    # A store-backed first touch (core/store_mesh.py) does not run
    # ``build``: it builds from the original source with these per-element
    # hooks applied to its table rows and tag names.
    d["_deferred_hooks"] = list(d.get("_deferred_hooks", ())) + [after_build]


def _renamed_paint(attrs, renamed):
    """``attrs`` (a face's, or a row of a face-attribute table) with its
    front and back material names moved through ``renamed``; the same
    object when neither is renamed, else a copy."""
    mat = attrs.get("mat")
    back = attrs.get("back")
    front_hit = mat in renamed
    back_hit = isinstance(back, dict) and back.get("mat") in renamed
    if not (front_hit or back_hit):
        return attrs
    out = dict(attrs)
    if front_hit:
        out["mat"] = renamed[mat]
    if back_hit:
        out["back"] = {**back, "mat": renamed[back["mat"]]}
    return out


def restamp_materials(mesh, renamed) -> None:
    """Move ``mesh``'s faces onto the materials' new names (``renamed``,
    old → new, from :func:`core.insert.merge_materials`). A mesh the
    document left deferred is relabelled in its face-attribute table,
    without building it: on Maxence's ride, three colliding names in the
    linked facilities built every one of its meshes on open (4.8 s of a
    9.5 s load, 2026-11-13). A built mesh has its faces restamped in place
    as before."""
    from core.mesh import DeferredMesh
    if type(mesh) is DeferredMesh:
        d = mesh.__dict__
        src = dict(d["_deferred_source"])
        meta = dict(src.get("meta") or {})
        table = meta.get("face_attrs") or []
        new = [_renamed_paint(row, renamed) for row in table]
        if all(a is b for a, b in zip(new, table)):
            return                     # no face of this mesh is renamed
        meta["face_attrs"] = new
        src["meta"] = meta
        _retable_deferred(mesh, src,
                          lambda m: _restamp_faces(m.faces, renamed))
        return
    _restamp_faces(mesh.faces, renamed)


def _restamp_faces(faces, renamed) -> None:
    for f in faces:
        a = f.attrs
        if not a:
            continue
        if a.get("mat") in renamed:
            a["mat"] = renamed[a["mat"]]
        back = a.get("back")
        if isinstance(back, dict) and back.get("mat") in renamed:
            back["mat"] = renamed[back["mat"]]


def _rename_tags(comp, path, reused=()) -> None:
    """Move the definition's geometry onto the host's names for the
    source's tags (:func:`tag_name`)."""
    from core.group import iter_placements
    from core.layers import assign_layer, layer_of

    # One name per source tag, worked out once: tag_name parses the path
    # each time, and the ride's links asked it ~200k times per open.
    names: dict = {}

    def rename(name):
        out = names.get(name)
        if out is None:
            out = names[name] = tag_name(path, name)
        return out
    seen = set()
    shared: dict = {}          # renamed tables, shared (_relabel_deferred)
    for g, _m in iter_placements(comp):
        if g is not comp:
            assign_layer(g, rename(layer_of(g)))
        if g.mesh in reused or id(g.mesh) in seen:
            continue
        seen.add(id(g.mesh))
        if _relabel_deferred(g.mesh, rename, shared):
            continue
        for f in g.mesh.faces:
            assign_layer(f, rename(layer_of(f)))
        for e in g.mesh.edges:
            assign_layer(e, rename(layer_of(e)))


def _sync_tags(scene, path, tags):
    """Reconcile only when the source table or its mirror in this host moved."""
    key = _norm(path)
    cache = scene.__dict__.setdefault('_link_tag_tables', {})
    def mirror():
        entry = _entry(scene, path)
        wanted = set(entry.get("tags", ()))
        own = stem(path)
        folders = tuple((f, scene.folder_parents.get(f), scene.folder_visibility.get(f),
                         scene.layer_folder_names.get(f), scene.layer_folder_uids.get(f))
                        for f in scene.layer_folders
                        if f in (LINKED_FOLDER, own) or f.startswith(own + SEP))
        return ([ly.to_dict() for ly in scene.layers if ly.key in wanted], folders,
                dict(entry['overrides']))
    old = cache.get(key)
    if old is None or old[0] != tags or old[1] != mirror():
        _apply_tags(scene, path, tags)
        cache[key] = tags, mirror()
        return True
    return False


def _apply_tags(scene, path, tags) -> None:
    """Bring the source's tags into the host (see the module notes): the
    folder of the file inside «Linked files», the source's folders below
    it, each tag as the source shows it unless overridden here. Tags of
    this file the source no longer has are dropped when nothing uses
    them. The ride-model agent (2026-11-09) also found empty mirror
    folders left behind when the source reorganised its tags."""
    from core.layers import (DEFAULT_LAYER, Layer, folders_of, move_folder,
                             remove_folder)
    entry = _entry(scene, path)
    if tags.get('structure'):
        entry['structure'] = tags['structure']
    else:
        entry.pop('structure', None)
    overrides = entry["overrides"]
    # What the source said last time: a tag the host now shows otherwise
    # was switched here — by the Tags panel, Isolate, a saved view, any
    # road — and that switch is the host's override from now on.
    last = entry.setdefault("source_visible", {})

    def folder(name, parent):
        if name not in folders_of(scene):
            scene.layer_folders = folders_of(scene) + [name]
        move_folder(scene, name, parent)
        return name

    top = folder(LINKED_FOLDER, None)
    own = folder(stem(path), top)
    existing = set(folders_of(scene))
    # Older hosts recorded only mirrored tags. Recover their namespaced
    # folder paths before those tags move, without claiming unrelated folders.
    if "folder_uids" in entry:
        owned = set(entry["folder_uids"])
    else:
        owned = set()
        old_tags = set(entry.get("tags", ()))
        for ly in scene.layers:
            f = ly.folder if ly.key in old_tags else None
            chain = set()
            while f in existing and f.startswith(own + SEP) and f not in chain:
                chain.add(f)
                f = scene.folder_parents.get(f)
            if f == own:
                owned.update(scene.layer_folder_uids[name] for name in chain)
    src_folders = {f: own + SEP + f for f in tags.get("folders") or ()}
    added = [name for name in src_folders.values() if name not in existing]
    if added:
        scene.layer_folders = folders_of(scene) + added
        folders_of(scene)  # assign identities before placing children
        owned.update(scene.layer_folder_uids[name] for name in added)
    # The host's folder keys are namespaced by the file; their labels are
    # the source's own (#tags-by-key), so the tree reads as in the source.
    src_labels = tags.get("folder_names") or {}
    labels = dict(getattr(scene, "layer_folder_names", None) or {})
    for f, name in src_folders.items():
        incoming = tags.get("folder_visibility", {}).get(f, True)
        previous = entry.setdefault("source_folder_visible", {})
        if name not in previous or scene.folder_visibility.get(name, True) == previous[name]:
            scene.folder_visibility[name] = incoming
        previous[name] = incoming
        parent = tags.get("parents", {}).get(f)
        move_folder(scene, name, src_folders.get(parent, own))
        label = src_labels.get(f) or f
        if label != name:
            labels[name] = label
    scene.layer_folder_names = labels

    wanted = []
    for raw in tags.get("layers") or ():
        src = Layer.from_dict(raw)
        # Keyed by the source tag's KEY — what the source's entities carry
        # (see _relabel) — and labelled like the source shows it.
        name = tag_name(path, src.key)
        wanted.append(name)
        ly = scene.layer_by_key(name)
        if ly is None:
            ly = Layer(name)
            scene.layers.append(ly)
        elif name in last and name not in overrides \
                and bool(ly.visible) != bool(last[name]):
            overrides[name] = bool(ly.visible)
        if src.key != DEFAULT_LAYER:
            ly.name = src.name
        last[name] = bool(src.visible)
        ly.color = src.color
        ly.linetype = src.linetype
        ly.lineweight = src.lineweight
        ly.opacity = src.opacity
        ly.folder = src_folders.get(src.folder, own)
        ly.visible = overrides.get(name, src.visible)
    old = set(entry.get("tags") or ()) - set(wanted)
    entry["tags"] = wanted
    if old:
        used = _used_tags(scene)
        scene.layers[:] = [ly for ly in scene.layers
                           if ly.key not in old or ly.key in used]
        for name in old:
            overrides.pop(name, None)
            last.pop(name, None)

    # Remove obsolete leaves first so a live layer or host subfolder
    # protects every ancestor. Only folders this link created qualify.
    current = set(src_folders.values())
    candidates = {f for f in folders_of(scene) if f not in current
                  and f.startswith(own + SEP) and scene.layer_folder_uids[f] in owned}
    occupied = {ly.folder for ly in scene.layers}
    while candidates:
        leaves = candidates - occupied - set(scene.folder_parents.values())
        if not leaves:
            break
        for f in leaves:
            remove_folder(scene, f)
        candidates -= leaves
    entry["folder_uids"] = sorted(owned.intersection(scene.layer_folder_uids.values()))


def _used_tags(scene) -> set:
    from core.purge import layer_carriers
    from core.layers import layer_of
    return {layer_of(e) for e in layer_carriers(scene)}


def _install(scene, path, comp) -> None:
    """Every copy of ``path`` shows ``comp``: they share its prototype
    mesh and nested placements, each keeping its own transform, tag,
    paint and name."""
    for g in linked_groups(scene, path):
        _populate(g, comp)
    _entry(scene, path)['loaded_mtime_ns'] = Path(path).stat().st_mtime_ns
    scene.version += 1


def validate_select(select):
    """Only source UIDs or a Navigator expression can define a subset."""
    if select is None:
        return
    if not isinstance(select, dict) or set(select) not in ({'uids'}, {'filter'}):
        raise ValueError('select requires exactly uids or filter')
    if 'uids' in select:
        if not isinstance(select['uids'], list) or any(not isinstance(u, str) for u in select['uids']):
            raise ValueError('uids must be a list of strings')
    else:
        from core.facets import validate_filter
        validate_filter(select['filter'])


# uuid.NAMESPACE_URL's bytes, then the name's prefix: what uuid5 hashes.
_UID5_PREFIX = (b"\x6b\xa7\xb8\x11\x9d\xad\x11\xd1\x80\xb4\x00\xc0\x4f\xd4\x30\xc8"
                + b"ingetrazo:link:v1:")


def stable_uid(placement_uid, source_uid):
    """Versioned, platform-independent UUID5 mapping; paths never participate.

    Exactly ``uuid.uuid5(NAMESPACE_URL, 'ingetrazo:link:v1:' + placement_uid
    + ':' + source_uid).hex[:16]`` — these ids are saved and named by scenes,
    so they may never change — computed with ``hashlib`` directly: building
    a ``UUID`` object per cloned group was a good part of 0.6 s of opening
    the ride. The first 8 bytes of the SHA-1 are kept, with the version
    nibble (13th hex digit) set to 5 as ``uuid5`` does; the variant bits
    fall outside these 16 digits."""
    from hashlib import sha1
    h = sha1(_UID5_PREFIX + (placement_uid + ':' + source_uid).encode()).hexdigest()
    return h[:12] + '5' + h[13:16]


def _clone_shell(group):
    """A shallow copy of ``group``: what ``copy.copy`` gives, without its
    generic reduce/reconstruct route (a few µs per call on 100k groups).
    Slots the original never set stay unset."""
    cls = type(group)
    if cls is not _Group():
        from copy import copy
        return copy(group)
    out = object.__new__(cls)
    for name in _GROUP_SLOTS:
        try:
            value = getattr(group, name)
        except AttributeError:
            continue
        setattr(out, name, value)
    return out


_GROUP_CLASS = None
_GROUP_SLOTS = ()


def _Group():
    global _GROUP_CLASS, _GROUP_SLOTS
    if _GROUP_CLASS is None:
        from core.group import Group
        _GROUP_CLASS, _GROUP_SLOTS = Group, Group.__slots__
    return _GROUP_CLASS


_PLACEMENT_FIELDS = ("name", "layer", "billboard", "context", "text3d", "hidden",
                     "material", "axes", "component", "exploded", "explode_offset", "locked")


def _populate(placement, source, *, reconcile=False, selection=None):
    """Install a source tree, retaining matching host objects on a reload.
    Return the changed mesh bindings if every placement kept its draw state.
    """
    from copy import deepcopy
    from PySide6.QtGui import QMatrix4x4
    from core.purge import iter_groups
    previous = {g.uid: g for g in iter_groups(placement.children)} if reconcile else {}

    swaps = []
    structure_changed = False
    installed = {}
    claimed = set()

    def install_mesh(mesh, old):
        if not reconcile:
            return mesh
        if mesh in installed:
            return installed[mesh]
        stamp = old.__dict__.get('_link_geometry') if old is not None else None
        incoming = mesh.__dict__.get('_link_geometry')
        if (stamp and incoming and stamp[0] == incoming[0]
                and stamp[1] == old.revision and old not in claimed):
            mesh_to_use = old
        else:
            mesh_to_use = mesh
        installed[mesh] = mesh_to_use
        claimed.add(mesh_to_use)
        return mesh_to_use

    def clone(group, parent):
        nonlocal structure_changed
        uid = stable_uid(placement.uid, group.uid)
        result = previous.pop(uid, None)
        old = result.mesh if result is not None else None
        new = result is None
        if new:
            structure_changed = True
            result = _clone_shell(group)
            result.uid = uid
        else:
            for name in _PLACEMENT_FIELDS:
                value = getattr(group, name, None)
                if getattr(result, name, None) != value:
                    structure_changed = True
                    setattr(result, name, deepcopy(value))
        result.mesh = install_mesh(group.mesh, old)
        if old is not None and result.mesh is not old:
            swaps.append((result, id(old)))
        if new or result.ext != group.ext:
            result.ext = deepcopy(group.ext) if group.ext is not None else None
        ifc = group.ifc
        if ifc and ifc.get('openings'):
            ifc = deepcopy(ifc)
            for opening in ifc['openings']:
                if opening.get('group_uid'):
                    opening['group_uid'] = stable_uid(placement.uid, opening['group_uid'])
        if new or result.ifc != ifc:
            result.ifc = deepcopy(ifc) if ifc is not None else None
        if new or result.xform != group.xform:
            structure_changed = True
            result.xform = QMatrix4x4(group.xform) if group.xform is not None else None
        result.owner = parent
        children = [clone(c, result) for c in group.children or ()]
        if result.children != children:
            structure_changed = True
            result.children = children
        return result
    old_mesh = placement.mesh
    placement.mesh = install_mesh(source.mesh, old_mesh)
    if placement.mesh is not old_mesh:
        swaps.append((placement, id(old_mesh)))
    children = [clone(c, placement) for c in source.children or ()]
    if placement.children != children:
        structure_changed = True
        placement.children = children
    if selection is not None:
        for removed in previous.values():
            selection.discard(removed)
    placement.component = True
    return None if structure_changed or previous else swaps


# ---- the entry points --------------------------------------------------------------

def _read_definition(registry, path, select, include_nested, previous=None):
    from core.link_meshes import LinkMeshes
    from core.purge import iter_groups
    from core.insert import merge_materials
    reader = LinkMeshes(path, registry, previous, filtered=select is not None and 'filter' in select)
    comp, tags, materials = load_source(path, select, include_nested=include_nested,
                                        link_meshes=reader)
    _rename_tags(comp, path, reader.reused)
    renamed = merge_materials(materials, registry, comp, reused=reader.reused)
    seen = set()
    for group in iter_groups([comp]):
        mesh = group.mesh
        if mesh in seen or mesh in reader.reused:
            continue
        seen.add(mesh)
        stamp = mesh.__dict__.get('_link_geometry')
        if stamp:
            key = stamp[0]
            if key[0] != str(path):
                key = (str(path), tuple(renamed.items()), 'nested', key)
            mesh.__dict__['_link_geometry'] = key, mesh.revision
    return comp, tags


def make_link(scene, path, host_path=None, select=None, *, include_nested=True):
    """A copy of the .igz at ``path`` ready to place in ``scene`` (by the
    placement tool, like an imported component), its tags brought in.
    Raises :class:`LinkError`."""
    path = Path(path).resolve()
    if host_path is not None and _norm(path) == _norm(host_path):
        raise LinkError("a document cannot link itself")
    comp, tags = _read_definition(scene.materials, path, select, include_nested)
    _sync_tags(scene, path, tags)
    comp.ext = {LINK_KEY: _link_record(path, host_path)}
    if not include_nested:
        comp.ext[LINK_KEY]["include_nested"] = False
    if select is not None:
        from copy import deepcopy
        comp.ext[LINK_KEY]['select'] = deepcopy(select)
    _populate(comp, comp)
    _entry(scene, path)['loaded_mtime_ns'] = path.stat().st_mtime_ns
    comp.name = stem(path)
    return comp


def _link_record(path, host_path) -> dict:
    rec = {"path": str(Path(path).resolve())}
    if host_path is not None:
        try:
            rec["rel"] = os.path.relpath(Path(path).resolve(),
                                         Path(host_path).resolve().parent)
        except ValueError:          # another drive (Windows)
            pass
    return rec


def set_include_nested(scene, group, enabled):
    """Refresh one placement from disk with the chosen nested-link policy."""
    rec = link_of(group)
    path = rec["path"]
    comp, tags = _read_definition(scene.materials, path, rec.get("select"), enabled, group)
    _sync_tags(scene, path, tags)
    _populate(group, comp, reconcile=True, selection=scene.selection)
    rec["include_nested"] = enabled
    scene.version += 1


def reload(scene, path, host_path=None) -> None:
    """Read ``path`` again and show it in every copy. Raises
    :class:`LinkError` (the copies keep what they showed)."""
    # Prepare every subset before changing any placement or host material.
    path = Path(path).resolve()
    materials = dict(scene.materials)
    prepared = []
    definitions = {}
    for group in linked_groups(scene, path):
        rec = link_of(group)
        config = (repr(rec.get('select')), rec.get('include_nested', True))
        if config not in definitions:
            definitions[config] = _read_definition(materials, path, rec.get('select'), config[1], group)
        comp, tags = definitions[config]
        prepared.append((group, comp, tags))
    before = scene.content_version
    swaps = []
    scene.materials.update(materials)
    for group, comp, _tags in prepared:
        changed = _populate(group, comp, reconcile=True, selection=scene.selection)
        if changed is None:
            swaps = None
        elif swaps is not None:
            swaps.extend(changed)
    if prepared and _sync_tags(scene, path, prepared[0][2]):
        swaps = None
    _entry(scene, path)['loaded_mtime_ns'] = Path(path).stat().st_mtime_ns
    scene.version += 1
    # A view that has exactly the previous revision can retain every matrix,
    # paint and pick ID. A missed revision or tree/tag change uses its diff.
    scene._link_mesh_changes = (before, scene.content_version, swaps)


def resolve(scene, host_path=None) -> dict:
    """Fill the linked copies of a document just opened from their sources
    (packed ones keep what the file holds). Returns ``{path: error}`` for
    the sources that could not be read — their copies stay empty
    placements, kept, so a later save does not lose the link."""
    errors: dict = {}
    stack = getattr(_loading, "stack", None)
    if stack is None:
        stack = _loading.stack = []
    host_key = _norm(host_path) if host_path is not None else None
    if host_key is not None and host_key not in stack:
        # The document being opened is part of the chain: a source that
        # links back to it is a cycle.
        stack.append(host_key)
        try:
            return resolve(scene, host_path)
        finally:
            stack.remove(host_key)
    for g in linked_groups(scene):
        link = link_of(g)
        found = locate(link, host_path)
        if found is not None and _norm(found) != _norm(link["path"]):
            link["path"] = str(found)     # the project moved: follow it
    for path in sources(scene):
        if is_packed(scene, path):
            continue
        try:
            reload(scene, path, host_path)
        except LinkError as exc:
            errors[path] = str(exc)
    scene.link_errors = errors
    return errors


def set_packed(scene, path, packed: bool) -> None:
    """Pack (the host file keeps the geometry) or unpack (it is read from
    the source again — reloaded now)."""
    _entry(scene, path)["packed"] = bool(packed)


def note_visibility(scene, name: str, visible: bool) -> bool:
    """The host switched tag ``name``: if it is a linked file's, remember
    it as an override. Returns True when it was."""
    path = source_of_tag(scene, name)
    if path is None:
        return False
    _entry(scene, path)["overrides"][name] = bool(visible)
    return True


def clear_overrides(scene, path, host_path=None) -> None:
    """Show ``path``'s tags as the source has them again."""
    entry = _entry(scene, path)
    entry["overrides"] = {}
    # Back to what the source said, so the reload does not read the old
    # override as a new one.
    for name, vis in (entry.get("source_visible") or {}).items():
        ly = scene.layer(name)
        if ly is not None:
            ly.visible = bool(vis)
    reload(scene, path, host_path)


def refresh_paths(scene, host_path) -> None:
    """Rewrite each link's ``rel`` for the host's new location (Save As)."""
    for g in linked_groups(scene):
        g.ext[LINK_KEY].update(_link_record(link_of(g)["path"], host_path))


def save_as_placeholder(group, scene) -> bool:
    """Whether the .igz writer saves ``group`` as an empty placement (an
    unpacked link: the source holds the geometry)."""
    link = link_of(group)
    return link is not None and not is_packed(scene, link["path"])


def query(scene):
    """JSON metadata for UI and agents, without touching geometry."""
    from core.group import iter_placements
    rows = []
    for group in linked_groups(scene):
        rec = link_of(group)
        path = Path(rec['path'])
        stamp = path.stat().st_mtime_ns if path.is_file() else None
        state = ('packed' if is_packed(scene, path) else 'missing' if stamp is None
                 else 'modified' if stamp != _entry(scene, path).get('loaded_mtime_ns')
                 else 'resolved')
        rows.append(dict(uid=group.uid, path=str(path), status=state,
                         include_nested=rec.get('include_nested', True),
                         select=rec.get('select'), stable_uid_count=sum(1 for _ in iter_placements(group)) - 1))
    return rows
