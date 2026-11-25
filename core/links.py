# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Linked files: another .igz placed in this one as a live reference.

A linked file is a component whose definition is not the host's: it is the
source document, loaded from disk. The source stays a document of its own —
open it, edit it, save it, and every host that links it shows the change
(the window watches the files). Inside the host the linked part moves,
turns and scales like any component, but it cannot be opened for editing
there: entering it offers to open the source.

What is stored where
--------------------
* Each placed copy carries ``Group.ext["link"] = {"path": …}`` — the source,
  absolute — and ``"rel"``, the same path relative to the host when it was
  last saved, tried first so a project folder can move as a whole.
* The host's links registry (``scene.link_registry``, a root table of the
  document; the file stores it as ``plugin_data["links"]``) keeps, per
  source, the tag overrides (below) and whether the source is *packed*.
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
that tag no longer reach it.

The logic is ``rust/ingetrazo_core/src/model/links.rs`` (``link_*``): the
queries, the registry, the definition renamed as it is read, the tags
mirrored and released, the copies of a source's tree. What stays here
reads the files: a source loaded into a definition, a link made, reloaded,
resolved on open and filled again.
"""

from __future__ import annotations

import os
import threading
from pathlib import Path

import ingetrazo_core as _rs

# The plugin surface (docs/model-port.md).
link_of = _rs.link_of
fill_link = _rs.link_fill
is_linked = _rs.link_is_linked
linked_groups = _rs.link_linked_groups
sources = _rs.link_sources
link_state = _rs.link_state

LINK_KEY = "link"                 # Group.ext key on each placed copy
REGISTRY_KEY = "links"            # the file's plugin_data key for the registry
LINKED_FOLDER = "Linked files"    # the tag folder all links' folders sit in
SEP = " / "                       # between the file's name and a tag's

_loading = threading.local()      # the sources being loaded (cycle guard)


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


def load_source(path, select=None, *, include_nested=True, link_meshes=None, progress=None):
    """``(definition, source tags, source materials)`` for the .igz at
    ``path``: the whole document as one component definition, its own links
    resolved first. Raises :class:`LinkError`."""
    from core.scene import Scene
    from formats import igz
    key = _rs.link_norm(path)
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
        igz.load_into(src, Path(path), progress=progress, include_links=include_nested, link_meshes=link_meshes)
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
        from ingetrazo_core import Matrix4 as QMatrix4x4
        _rs.link_validate_select(select)
        groups = [g for g in src.groups if not is_scale_figure(g)]
        mesh = src.mesh
        if select is not None:
            from core.facets import build, evaluate
            from collections import Counter
            from core.link_structure import row_id
            refs = select.get('uids')
            wanted = {ref for ref in refs or () if isinstance(ref, str)}
            targets = {src.resolve_occurrence(ref) for ref in refs or () if isinstance(ref, list)}
            requested_paths = targets.copy()
            if refs is None:
                index = build(src)
                targets.update(index.groups[key] for key in evaluate(index, select['filter']))
            selected = []

            def visit(items, parent):
                for group in items:
                    world = parent * (group.xform or QMatrix4x4())
                    if group.uid in wanted or group in targets:
                        selected.append((group, world))
                    else:
                        visit(group.children or (), world)

            def detached(group):
                copy = _rs.link_clone_shell(group)
                copy.children = [detached(c) for c in group.children]
                return copy

            visit(groups, QMatrix4x4())
            counts = Counter(group.uid for group, _ in selected)
            repeated = src.repeated_uids() if requested_paths else set()
            groups, mesh = [], Mesh()
            for group, world in selected:
                part = detached(group)
                if counts[group.uid] > 1 or (group in requested_paths and group.uid in repeated):
                    # Two equal leaf UIDs can become siblings in a flat subset.
                    part.uid = row_id(src.occurrence_path(group))
                part.xform = world
                groups.append(part)
            mesh.__dict__['_link_geometry'] = ((str(path), 'subset-root'), mesh.revision)
        comp = Group(mesh, name=Path(path).stem)
        comp.adopt(groups)
    except LinkError:
        raise
    except Exception as exc:  # noqa: BLE001 — any reader failure
        raise LinkError(f"{Path(path).name}: {exc}") from exc
    finally:
        stack.remove(key)
    if comp.xform is None:
        from ingetrazo_core import Matrix4 as QMatrix4x4
        comp.xform = QMatrix4x4()
    comp.component = True
    return comp, tags, materials


def _read_definition(registry, path, select, include_nested, previous=None, *, progress=None):
    """The source at ``path`` read into a definition renamed into the host:
    its materials registered in ``registry`` (renamed where a name is the
    host's already), its tags and paint on the host's names."""
    from core.link_meshes import LinkMeshes
    from core.materials import register
    reader = LinkMeshes(path, registry, previous, filtered=select is not None and 'filter' in select)
    comp, tags, materials = load_source(path, select, include_nested=include_nested,
                                        link_meshes=reader, progress=progress)
    renamed = {}
    for old, mat in list(materials.items()):
        final = register(registry, mat)
        if final != old:
            renamed[old] = final
    _rs.link_install_definition(comp, path, reader.reused, renamed)
    return comp, tags


def make_link(scene, path, host_path=None, select=None, *, include_nested=True):
    """A copy of the .igz at ``path`` ready to place in ``scene`` (by the
    placement tool, like an imported component), its tags brought in.
    Raises :class:`LinkError`."""
    path = Path(path).resolve()
    if host_path is not None and _rs.link_norm(path) == _rs.link_norm(host_path):
        raise LinkError("a document cannot link itself")
    comp, tags = _read_definition(scene.materials, path, select, include_nested)
    _rs.link_sync_tags(scene, path, tags)
    comp.ext = {LINK_KEY: _link_record(path, host_path)}
    if not include_nested:
        comp.ext[LINK_KEY]["include_nested"] = False
    if select is not None:
        from copy import deepcopy
        comp.ext[LINK_KEY]['select'] = deepcopy(select)
    _rs.link_populate(comp, comp)
    _rs.link_entry(scene, path)['loaded_mtime_ns'] = path.stat().st_mtime_ns
    comp.name = path.stem
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
    _rs.link_sync_tags(scene, path, tags)
    _rs.link_populate(group, comp, reconcile=True, selection=scene.selection)
    rec["include_nested"] = enabled
    scene.version += 1


def reload(scene, path, host_path=None, *, progress=None) -> None:
    """Read ``path`` again and show it in every copy. Raises
    :class:`LinkError` (the copies keep what they showed)."""
    # Prepare every subset before changing any placement or host material.
    path = Path(path).resolve()
    materials = dict(scene.materials)
    prepared = []
    definitions = {}
    roots = {}
    for match in linked_groups(scene, path):
        group = match
        # A nested copy wears every enclosing source's tag namespace and
        # UID mapping. Its outer definition also owns the packed policy.
        while (parent := _rs.link_linked_ancestor(group.owner)) is not None:
            group = parent
        source_path = Path(link_of(group)['path']).resolve()
        if group is match or not _rs.link_is_packed(scene, source_path):
            roots[group] = source_path
    tags_by_path = {}
    for group, source_path in roots.items():
        rec = link_of(group)
        nested = rec.get('include_nested', True)
        config = (source_path, repr(rec.get('select')), nested)
        if config not in definitions:
            definitions[config] = _read_definition(materials, source_path, rec.get('select'), nested,
                                                   group, progress=progress)
        comp, tags = definitions[config]
        prepared.append((group, comp))
        tags_by_path[source_path] = tags
    if not prepared:
        return
    before = scene.content_version
    swaps = []
    scene.materials.update(materials)
    for group, comp in prepared:
        changed = _rs.link_populate(group, comp, reconcile=True, selection=scene.selection)
        if changed is None:
            swaps = None
        elif swaps is not None:
            swaps.extend(changed)
    for source_path, tags in tags_by_path.items():
        if _rs.link_sync_tags(scene, source_path, tags):
            swaps = None
        _rs.link_entry(scene, source_path)['loaded_mtime_ns'] = source_path.stat().st_mtime_ns
    scene.version += 1
    # A view that has exactly the previous revision can retain every matrix,
    # paint and pick ID. A missed revision or tree/tag change uses its diff.
    scene._link_mesh_changes = (before, scene.content_version, swaps)


def resolve(scene, host_path=None, *, progress=None) -> dict:
    """Fill the linked copies of a document just opened from their sources
    (packed ones keep what the file holds). Returns ``{path: error}`` for
    the sources that could not be read — their copies stay empty
    placements, kept, so a later save does not lose the link."""
    errors: dict = {}
    stack = getattr(_loading, "stack", None)
    if stack is None:
        stack = _loading.stack = []
    host_key = _rs.link_norm(host_path) if host_path is not None else None
    if host_key is not None and host_key not in stack:
        # The document being opened is part of the chain: a source that
        # links back to it is a cycle.
        stack.append(host_key)
        try:
            return resolve(scene, host_path, progress=progress)
        finally:
            stack.remove(host_key)
    for g in linked_groups(scene):
        # A packed copy keeps what the file holds, and is not read again:
        # its parts read-only all the same.
        _rs.link_adopt_parts(g)
        link = link_of(g)
        found = locate(link, host_path)
        if found is not None and _rs.link_norm(found) != _rs.link_norm(link["path"]):
            link["path"] = str(found)     # the project moved: follow it
    paths = [path for path in sources(scene) if not _rs.link_is_packed(scene, path)]
    for index, path in enumerate(paths):
        def report(fraction, phase):
            progress((index + fraction) / len(paths), f"{Path(path).name} ({index + 1}/{len(paths)}) · {phase}")

        if progress:
            report(0.0, "Reading linked document…")
        try:
            reload(scene, path, host_path, progress=report if progress else None)
        except LinkError as exc:
            errors[path] = str(exc)
    scene.link_errors = errors
    return errors


def clear_overrides(scene, path, host_path=None) -> None:
    """Show ``path``'s tags as the source has them again."""
    entry = _rs.link_entry(scene, path)
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


_SET_REGISTRY = None


def set_registry_command(value):
    """An undoable step that replaces the links registry with ``value``
    (a copy, through JSON both ways, as ``SetPluginDataCommand`` copies).
    The registry is a document value: the step's transaction undoes it."""
    global _SET_REGISTRY
    if _SET_REGISTRY is None:
        import json
        from core.history import Command

        def copy(v):
            return json.loads(json.dumps(v))

        class SetLinkRegistryCommand(Command):
            doc_undo = True

            def __init__(self, value) -> None:
                self.value = copy(value)
                self._before = None

            def do(self, scene) -> None:
                self._before = copy(scene.link_registry)
                scene.link_registry = copy(self.value)
                scene.version += 1

            def undo(self, scene) -> None:
                scene.link_registry = copy(self._before)
                scene.version += 1

        _SET_REGISTRY = SetLinkRegistryCommand
    return _SET_REGISTRY(value)
