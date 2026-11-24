# SPDX-License-Identifier: GPL-3.0-or-later
"""Shared registry ownership; documents keep a portable reference and fallback.

The project is a ``.builtproject`` manifest (docs/builtproject-format.md);
its I/O, pointer resolution and merges are Rust. Readers refresh from disk.
A registry write is a three-way merge against what this document last read,
so other people's edits to other entries survive; an entry both changed
keeps ours and is reported (``scene.project_warning`` and a RuntimeWarning).
"""
from copy import deepcopy
import json
from pathlib import Path
import uuid
import warnings

from formats import project as storage


def path_for(scene, host_path=None):
    """The project the document points at: an old ``X.igzproj`` pointer
    finds its ``X.builtproject`` sibling. A pointer that finds nothing still
    names its absolute path (callers check that it exists)."""
    ref = scene.plugin_data.get('project')
    if not isinstance(ref, dict):
        return None
    host = host_path or getattr(scene, '_project_host_path', None)
    from ingetrazo_core import project_resolve_pointer
    pointer = {k: ref[k] for k in ('path', 'rel') if isinstance(ref.get(k), str)}
    found = project_resolve_pointer(Path(host) if host else None, json.dumps(pointer))
    if found is not None:
        return Path(found)
    return Path(ref['path']) if ref.get('path') else None


def _observed(scene, manifest, value):
    scene._project_mtime_ns = manifest.stat().st_mtime_ns
    scene._project_registry_base = deepcopy(value['bim_registry'])


def read(scene):
    path = path_for(scene)
    if path is None or not path.is_file():
        return None
    manifest = storage.open_path(path)
    value = storage.load(manifest)
    _observed(scene, manifest, value)
    return value


def attach(scene, path, host_path=None):
    path = storage.open_path(Path(path)).resolve()
    value = storage.load(path)
    scene.plugin_data['project'] = {'path': str(path)}
    refresh_reference(scene, host_path)
    _observed(scene, path, value)
    scene.plugin_data['ifc_identity'] = deepcopy(value['ifc_identity'])
    scene.version += 1
    return value


def refresh_reference(scene, host_path):
    """On a normal save: the pointer names the resolved manifest, absolute
    and relative to the model (an old ``.igzproj`` pointer moves on here)."""
    path = path_for(scene)
    if path is None or not host_path:
        return
    if path.is_file():
        path = storage.open_path(path)
    from ingetrazo_core import project_relative
    host_path = Path(host_path).resolve()
    scene.plugin_data['project'] = {'path': str(path),
                                    'rel': project_relative(path, host_path.parent)}
    scene._project_host_path = str(host_path)


def create(scene, path, host_path=None):
    from core.bim_registry import get
    from core.ifc_identity import stable_guid
    path = Path(path)
    if path.suffix.lower() != storage.SUFFIX:
        path = path.with_suffix(storage.SUFFIX)
    identity = deepcopy(scene.plugin_data.get('ifc_identity', {}))
    identity.setdefault('namespace', uuid.uuid4().hex)
    for key in ('project', 'site', 'building', 'storey'):
        identity.setdefault(key, stable_guid(identity['namespace'], key))
    value = storage.create(path, get(scene), identity)
    attach(scene, path, host_path)
    sync_files(scene, host_path)
    return value


def write_registry(scene, registry, base=None):
    """Merge ``registry`` into the shared file. ``base`` is what it was
    derived from (default: this document's last read); undo passes the
    command's after-value so it reverts only what the command changed."""
    path = path_for(scene)
    if path is None:
        return False
    # Missing project files must never be silently overwritten with a fallback.
    manifest = storage.open_path(path)
    if base is None:
        base = getattr(scene, '_project_registry_base', None)
    if base is None:
        base = storage.load(manifest)['bim_registry']
    from ingetrazo_core import project_write_registry
    conflicts = project_write_registry(manifest, json.dumps(base), json.dumps(registry))
    if conflicts:
        message = ('Project entries also changed elsewhere; yours kept: '
                   + ', '.join(conflicts) + ' (' + str(manifest) + ')')
        scene.project_warning = message
        warnings.warn(message, RuntimeWarning, stacklevel=2)
    value = storage.load(manifest)
    _observed(scene, manifest, value)
    scene.plugin_data['bim_registry'] = deepcopy(value['bim_registry'])
    return True


def sync_files(scene, host_path):
    """Saving a host: its file entry, its linked sources and its own link
    edges; other hosts' edges and every other field are kept."""
    path = path_for(scene)
    if path is None or not path.is_file() or not host_path:
        return
    manifest = storage.open_path(path)
    from core.links import linked_groups, link_of
    from ingetrazo_core import project_relative, project_sync_host

    def relative(p):
        return project_relative(Path(p).resolve(), manifest.parent)
    host = relative(host_path)
    files, edges = [dict(path=host, role='host')], []
    for group in linked_groups(scene):
        rec = link_of(group)
        source = relative(rec['path'])
        files.append(dict(path=source, role='linked'))
        edge = dict(host=host, source=source, placement_uid=group.uid,
                    select=deepcopy(rec.get('select')))
        if not rec.get('include_nested', True):
            edge['include_nested'] = False
        edges.append(edge)
    project_sync_host(manifest, host, json.dumps(files), json.dumps(edges))
    scene._project_mtime_ns = manifest.stat().st_mtime_ns


# ---- per-person session (the classic worksession uses these) ----------------
def session(path, settings_dir=None):
    from ingetrazo_core import ProjectSession
    return ProjectSession(Path(path), settings_dir)


def save_worksession(path, files, active_file):
    """This person's tabs and editable file, in their session file; the
    shared manifest is not touched."""
    s = session(path)
    s.set_editable(Path(active_file) if active_file else None)
    s.record([(Path(e['path']), json.dumps(e['camera']) if e.get('camera') is not None else None,
               e.get('scene')) for e in files], keep_planned=False)
    return json.loads(s.query())


def worksession(path):
    """Resolve portable paths; absent state opens the project's host documents."""
    s = session(path)
    rows = [dict(path=p, role=role, camera=json.loads(camera) if camera else None, scene=scene)
            for p, role, camera, scene, _exists in s.plan()]
    return dict(project=str(s.path), files=rows,
                active_file=s.editable or (rows[0]['path'] if rows else None))
