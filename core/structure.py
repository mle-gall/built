# SPDX-License-Identifier: GPL-3.0-or-later
"""Project filing (2026-11-09). Plan all projections before executing any command."""
from collections import Counter, defaultdict
from copy import deepcopy

from core import bim_registry, links
from core.history import (AssignLayerCommand, Command, CompoundCommand, EnsureLayersCommand,
                          SetRecordCommand, SetRegistryCommand, SetStructureNodeCommand)
from core.layers import DEFAULT_LAYER, layer_of
from core.group import new_uid
from core.record import typed


def _nodes(registry):
    return registry.get('structure', {}).get('nodes', {})


def _lineage(registry, key):
    from ingetrazo_core import structure_lineage
    return structure_lineage(_nodes(registry), key)


def tree(registry):
    """Children by parent key, including roots under None; ties retain authored order."""
    children = {None: []}
    nodes = _nodes(registry)
    for key, node in nodes.items():
        children.setdefault(node['parent'], []).append(key)
        children.setdefault(key, [])
    for siblings in children.values():
        siblings.sort(key=lambda k: nodes[k]['order'])
    return children


def path(registry, key):
    return [_nodes(registry)[k]['name'] for k in _lineage(registry, key)]


def resolve(registry, key):
    nodes = _nodes(registry)
    names = []
    recipe, classifications, systems, psets = {}, {}, [], {}
    for k in _lineage(registry, key):
        node = nodes[k]
        names.append(node['name'])
        local = node.get('source_recipe', {}) if 'source' in node else node.get('recipe', {})
        recipe.update({f: deepcopy(v) for f, v in local.items()
                       if f not in ('classifications', 'systems', 'psets')})
        for item in local.get('classifications', []):
            classifications[item['system'], item['code']] = deepcopy(item)
        systems.extend(local.get('systems', []))
        for name, props in local.get('psets', {}).items():
            psets.setdefault(name, {}).update(deepcopy(props))
    recipe.setdefault('tag', names[-1])
    folder = recipe.get('folder', names[:-1])
    recipe['folder'] = ([folder] if folder else []) if isinstance(folder, str) else folder
    return {**recipe, 'classifications': list(classifications.values()),
            'systems': list(dict.fromkeys(systems)), 'psets': psets}


def node_of(group):
    return ((group.ext or {}).get('structure') or {}).get('node')


def _placements(scene):
    from ingetrazo_core import StructurePlacements
    return StructurePlacements(scene)


def _targets(scene, uids):
    from ingetrazo_core import structure_targets
    return structure_targets(scene, uids)


def _patch(group, recipe, registry):
    old = group.ifc or {}
    cls = recipe.get('class')
    allowed = recipe.get('classes_allowed')
    if allowed is not None and old.get('class') is not None:
        if old['class'] not in allowed:
            raise ValueError(f"{group.uid}: {old['class']} outside classes_allowed")
        if cls is not None:
            cls = old['class']
    if cls is None:
        return {}
    predefined = recipe.get('predefined_type')
    if predefined:
        from core.ifc_tables import TABLES, predefined_types
        if not any(predefined in predefined_types(cls, s) for s in TABLES[cls]['schemas']):
            raise ValueError(f'{group.uid}: {predefined} is invalid for {cls}')
    codes = deepcopy(recipe['classifications'])
    namespaces = {c['system'] for c in codes}
    for c in codes:
        c.setdefault('name', registry['classification_systems'][c['system']]['codes'][c['code']]['name'])
    foreign = [c for c in old.get('classifications', []) if c.get('system') not in namespaces]
    psets = {name: {k: typed(v) for k, v in props.items()}
             for name, props in recipe['psets'].items()}
    for name, props in old.get('psets', {}).items():
        merged = psets.setdefault(name, {})
        for prop, value in props.items():
            stored = value.get('value') if isinstance(value, dict) else value
            if stored is not None or prop not in merged:
                merged[prop] = value
    fields = {'class': cls, 'predefined_type': predefined, 'type': recipe.get('type'),
              'object_type': recipe.get('object_type', ''), 'systems': recipe['systems'],
              'classifications': foreign + codes, 'psets': psets}
    # Missing optional defaults already express the same value in record v1.
    return {k: v for k, v in fields.items() if old.get(k) != v and (v or k in old)}


class _EnsureStructureLayers(EnsureLayersCommand):
    def do(self, scene):
        from ingetrazo_core import structure_folder_state, apply_folder_parents
        self._folders = structure_folder_state(scene)
        super().do(scene)
        apply_folder_parents(scene, self._template)
        scene.version += 1

    def undo(self, scene):
        from ingetrazo_core import restore_structure_folders
        super().undo(scene)
        restore_structure_folders(scene, self._folders)


class _MoveTag(Command):
    doc_undo = True

    def __init__(self, layer, folder):
        self.layer, self.folder = layer, folder

    def do(self, scene):
        from ingetrazo_core import definition_edits
        self.before = definition_edits.set_placement_field(self.layer, "folder", self.folder)
        scene.version += 1

    def undo(self, scene):
        self.layer.folder = self.before
        scene.version += 1


def _filing(scene, groups, registry, key, changes):
    from ingetrazo_core import plan_filing
    return plan_filing(scene, groups, registry, key, changes)


def _check_paths(scene, registry, batches):
    from ingetrazo_core import check_filing_paths
    return check_filing_paths(scene, registry, batches)


def file(scene, uids, key):
    groups = _targets(scene, uids)
    registry = bim_registry.get(scene)
    _check_paths(scene, registry, {key: groups})
    return CompoundCommand(_filing(scene, groups, registry, key, {}))


def file_instances(scene, uid, key):
    """File all local placements sharing the selected definition mesh."""
    group, = _targets(scene, [uid])
    return file(scene, [g.uid for g, src, _ in _placements(scene) if not src and g.mesh is group.mesh], key)


def unfile(scene, uids):
    return SetStructureNodeCommand(_targets(scene, uids), None)


def _mount_index(registry):
    # A source's stable node keys belong to its own namespace. Older documents
    # have only mirrored tags; those provide virtual membership without a write.
    mounts, sources, tags = {}, {}, {}
    for key, node in _nodes(registry).items():
        if 'source' in node:
            src = node['source']
            if 'source_node' in node:
                sources[src, node['source_node']] = key
            elif 'source_tag' in node:
                tags[src, node['source_tag']] = key
            elif node.get('mount'):
                mounts[src] = key
    return mounts, sources, tags


def _membership(group, source, mount_index):
    key = node_of(group)
    if not source:
        return key
    mounts, sources, tags = mount_index
    if links.is_linked(group):
        return mounts.get(source)
    return sources.get((source, key), tags.get((source, group.layer)))


def _memberships(scene, registry):
    mount_index = _mount_index(registry)
    for g, src, root in _placements(scene):
        yield g, src, root, _membership(g, src, mount_index)


def members(scene, key, recursive=True):
    registry = bim_registry.get(scene)
    children = tree(registry)
    if key not in _nodes(registry):
        raise ValueError('Unknown node: ' + str(key))
    keys, stack = set(), [key]
    while stack:
        current = stack.pop()
        keys.add(current)
        if recursive:
            stack.extend(children[current])
    return [g.uid for g, _, _, k in _memberships(scene, registry) if k in keys]


def resync(scene, keys=None):
    """Return (one History command, changed field names per uid), including descendants."""
    return _resync(scene, bim_registry.get(scene), keys)


def _resync(scene, registry, keys):
    from ingetrazo_core import resync_filing
    return resync_filing(scene, registry, keys)


def unsorted(scene):
    registry = bim_registry.get(scene)
    recipes = {k: resolve(registry, k) for k in _nodes(registry)}
    return [g.uid for g, src, root, k in _memberships(scene, registry)
            if _is_unsorted(g, src, root, k, recipes)]


def _is_unsorted(group, source, root, key, recipes):
    if source:
        return False
    if key not in recipes:
        return root or key is not None
    recipe = recipes[key]
    return bool(group.ifc and group.ifc.get('class') not in
                recipe.get('classes_allowed', [recipe.get('class')]))


def _has_status(record):
    for name, props in record.get('psets', {}).items():
        if name.endswith('Common'):
            value = props.get('Status')
            if isinstance(value, dict):
                value = value.get('value')
            if value:
                return True
    return False


def completeness(scene, registry):
    """Roll placement counts up the tree, counting missing Status on records only."""
    from core.structure_ids import load_requirements, requirement_hits
    nodes = _nodes(registry)
    stats = {k: dict(members=0, without_record=0, missing_status=0, ids_hits=[]) for k in nodes}
    requirements = load_requirements(scene, registry)
    lineages = {k: _lineage(registry, k) for k in nodes}
    for group, _, _, key in _memberships(scene, registry):
        if key not in nodes:
            continue
        rec = group.ifc or {}
        status = _has_status(rec)
        hits = requirement_hits(group, registry, requirements)
        for k in lineages[key]:
            row = stats[k]
            row['members'] += 1
            row['without_record'] += not rec
            row['missing_status'] += bool(rec) and not status
            row['ids_hits'].extend(hits)
    return stats


def _infer(groups, key, conflicts):
    """Infer from placed records, preserving first-seen order when a vote ties."""
    records = [g.ifc or {} for g in groups]
    classes = Counter(r['class'] for r in records if r.get('class'))
    recipe = {'class': classes.most_common(1)[0][0] if classes else None}
    missing = len(groups) - sum(classes.values())
    if classes and 2 * sum(classes.values()) < len(groups):
        conflicts.append(dict(node=key, reason='recorded under null class',
                              placements=[{'uid': g.uid, 'class': g.ifc['class']}
                                          for g in groups if (g.ifc or {}).get('class')]))
        return {'class': None}
    if len(classes) > 1:
        recipe['classes_allowed'] = list(classes)
        conflicts.append(dict(node=key, reason='mixed classes', counts=dict(classes)))
    if classes and missing:
        conflicts.append(dict(node=key, reason='would newly classify', count=missing))
    if not records or not classes:
        return recipe

    first = records[0]
    for field in ('predefined_type', 'object_type', 'type'):
        val = first.get(field)
        if all(r.get(field) == val for r in records):
            if val is not None:
                recipe[field] = val
        else:
            conflicts.append(dict(node=key, reason='varying ' + field))
    for field in ('systems', 'classifications'):
        def identity(value):
            return (value['system'], value['code']) if field == 'classifications' else value
        common = set(map(identity, first.get(field, [])))
        for record in records[1:]:
            common.intersection_update(map(identity, record.get(field, [])))
        values = [deepcopy(v) for v in first.get(field, []) if identity(v) in common]
        if values:
            recipe[field] = values
        if any(set(map(identity, r.get(field, []))) != common for r in records):
            conflicts.append(dict(node=key, reason='varying ' + field))
    if recipe.get('predefined_type') == 'USERDEFINED' and not recipe.get('object_type'):
        # A shared enum without a shared user-defined label is not a valid default.
        recipe.pop('predefined_type')
    return recipe


def bootstrap(scene, registry):
    """Propose one node per tag. When at least half its members are recorded,
    the majority recorded class wins; below half, the class is null. Report
    both kinds of residue. Null-class filing never edits existing records.
    Linked placements get virtual membership and are never filed by the host.
    """
    from core import doc_model
    if doc_model.ENABLED:
        from ingetrazo_core import structure_proposal
        return structure_proposal(scene, registry)
    nodes, conflicts, filing = {}, [], {}
    by_tag = defaultdict(list)
    source_paths = {}
    for g, src, root in _placements(scene):
        if src:
            source_paths[links.stem(src)] = src
        if g.layer and g.layer != DEFAULT_LAYER:
            by_tag[g.layer].append((g, src))
        elif root and not src:
            conflicts.append(dict(uid=g.uid, name=g.name, reason='untagged placement'))
    folders = list(dict.fromkeys([*scene.layer_folders, *(ly.folder for ly in scene.layers if ly.folder)]))
    folder_keys = {f: ('link:' + f if f in source_paths else
                       'folder:' + scene.layer_folder_uids.get(f, f)) for f in folders}
    orders = Counter()

    def add(key, name, parent, **extra):
        nodes[key] = dict(name=name, parent=parent, order=orders[parent], **extra)
        orders[parent] += 1

    source_trees = {path: entry['structure'] for path, entry in
                    scene.plugin_data.get(links.REGISTRY_KEY, {}).items() if entry.get('structure')}
    folder_sources = {}
    for f in folders:
        source = source_paths.get(f) or source_paths.get(f.partition(links.SEP)[0])
        if source:
            folder_sources[f] = source
        mount = f in source_paths
        if source and not mount and source in source_trees:
            continue
        parent = folder_keys.get(scene.folder_parents.get(f))
        from core.layers import folder_label
        label = folder_label(scene, f)
        add(folder_keys[f], label.split(links.SEP, 1)[-1] if source else label, parent,
            **({'source': source, 'mount': mount} if source else {}))
    # Only the host's own links get a mount: a link inside a linked file
    # comes with that file's structure (core/link_structure.py), never as
    # a top-level node of the host.
    from core.link_structure import walk
    top = {links.stem(inner[-1][1]) for _g, chain, inner, _r in walk(scene) if inner != chain and not chain}
    for stem, src in source_paths.items():
        key = 'link:' + stem
        if key not in nodes and stem not in top:
            continue
        if key not in nodes:
            add(key, stem, None, source=src, mount=True)
        # Authored source keys and effective recipes survive differing host vocabularies.
        for sk, sn in source_trees.get(src, {}).items():
            parent = key if sn['parent'] is None else key + '/' + sn['parent']
            nodes[key + '/' + sk] = dict(name=sn['name'], parent=parent, order=sn['order'],
                                         source=src, source_node=sk, source_recipe=deepcopy(sn['recipe']))
    for layer in scene.layers:
        if layer.key == DEFAULT_LAYER:
            continue
        src = folder_sources.get(layer.folder)
        if src in source_trees:
            continue
        key = 'layer:' + layer.uid
        pairs = by_tag.pop(layer.key, [])
        groups = [g for g, source in pairs if source == src and not links.is_linked(g)]
        recipe = _infer(groups, key, conflicts)
        recipe['tag'] = layer.key
        name = layer.name.split(links.SEP, 1)[-1] if src else layer.name
        if name.casefold().startswith('_to review'):
            conflicts.append(dict(node=key, reason='review layer', count=len(groups),
                                  placements=[dict(uid=g.uid, name=g.name) for g in groups]))
        add(key, name, folder_keys.get(layer.folder),
            **(dict(source=src, source_tag=layer.key, source_recipe=recipe) if src else dict(recipe=recipe)))
        if groups and not src:
            filing[key] = [g.uid for g in groups]
    for tag, pairs in by_tag.items():
        for g, src in pairs:
            if not src:
                conflicts.append(dict(uid=g.uid, name=g.name, reason='missing layer', tag=tag))
    proposed = {'nodes': nodes}
    for message in bim_registry._structure_errors({**registry, 'structure': proposed}):
        conflicts.append(dict(reason='registry validation', message=message))
    return dict(structure=proposed, filing=filing, conflicts=conflicts)


def apply_bootstrap(scene, proposal):
    """Preflight the complete proposal; the registry and placements share one undo step."""
    from ingetrazo_core import apply_structure_bootstrap
    return apply_structure_bootstrap(scene, proposal)
