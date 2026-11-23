# SPDX-License-Identifier: GPL-3.0-or-later
"""Optional Revit reader and deterministic JSON-to-scene mapping (metres, Z up)."""
from __future__ import annotations

from copy import deepcopy
import importlib
import json
import math

from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from core.group import Group
from core.history import SnapshotImport, SetRegistryCommand
from core.i18n import tr
from core.kept_components import place_copy
from core.layers import Layer
from core.materials import Material, register


def read_rfa(path):
    """Decode a file without requiring the native extension at app startup."""
    try:
        reader = importlib.import_module('ingetrazo_rfa')
    except ImportError as exc:
        raise RuntimeError(tr('The Revit family reader is not built. Run scripts/build_native.sh.')) from exc
    return json.loads(reader.read_family(str(path)))


#: The "Clearance zones" tag's key: every .rfa import puts zones on it.
ZONE_TAG_KEY = 'rfa-clearance-zones'


def _triangle_mesh(positions, indices):
    """One Mesh from a part's triangles, welded and with the seams between
    coplanar or gently curved triangles softened, as the glTF import does."""
    import numpy as np
    from core.mesh import Mesh
    from formats.fuse import soften_smooth_edges
    try:
        pts = np.asarray(positions, dtype=np.float64).reshape(-1, 3)
        idx = np.asarray(indices, dtype=np.int64).reshape(-1, 3)
    except (ValueError, TypeError) as exc:
        raise ValueError(tr('Invalid Revit triangle mesh.')) from exc
    if not np.isfinite(pts).all():
        raise ValueError(tr('Invalid Revit triangle mesh.'))
    if len(idx) and (idx.min() < 0 or idx.max() >= len(pts)):
        raise ValueError(tr('Invalid Revit triangle index.'))
    tri = pts[idx]
    # Drop degenerate triangles (Revit's tessellation leaves a few slivers).
    area2 = np.linalg.norm(np.cross(tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0]), axis=1)
    tri = tri[area2 > 1e-12]
    mesh = Mesh()
    if not len(tri):
        return mesh
    vobjs, inverse = mesh.bulk_weld(tri.reshape(-1, 3))
    n = len(tri)
    mesh.add_faces_welded(vobjs, inverse, np.full(n, 3, dtype=np.int64),
                          np.ones(n, dtype=np.int64), [None] * n)
    soften_smooth_edges(mesh)
    return mesh


def _property(parameter):
    value = parameter.get('value', '')
    if parameter.get('kind') == 'Length' and parameter.get('units') == 'mm':
        try:
            number = float(value) / 1000.0
            if math.isfinite(number):
                return {'kind': 'length', 'value': number}
        except (ValueError, TypeError):
            pass
    return {'kind': 'text', 'value': str(value)}


def _placeholder(parameters, warnings, name):
    dims = []
    for axis in ('Width', 'Depth', 'Height'):
        p = next((p for p in parameters if p['name'] == axis), {})
        try:
            value = float(p['value']) / 1000.0
            if not math.isfinite(value) or value <= 0:
                raise ValueError()
        except (KeyError, TypeError, ValueError):
            value = 1.0
            warnings.append(tr('{name}: missing or invalid {axis}; using 1 m.', name=name, axis=axis))
        dims.append(value)
    w, d, h = dims
    g = Group(name=tr('Placeholder'))
    points = [QVector3D(x, y, z) for z in (0, h) for y in (0, d) for x in (0, w)]
    for loop in ((0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4),
                 (2, 6, 7, 3), (0, 4, 6, 2), (1, 3, 7, 5)):
        g.mesh.add_face([points[i] for i in loop])
    g.component = False
    warnings.append(tr('{name}: no decoded mesh; imported a placeholder box.', name=name))
    return g


def apply_payload(scene, payload):
    """Map a parsed contract into scene; no native reader or file I/O.

    Returns report messages. Solid IDs are local to each component placement.
    Parts stay as child groups so mixed visibility/material/tag states within
    one solid remain independently editable. All geometry and translations
    already use the model's internal unit, metres.
    """
    if payload.get('schema') != 'ingetrazo.rfa/1':
        raise ValueError(tr('Unsupported Revit family JSON schema.'))
    if not payload.get('types'):
        raise ValueError(tr('The Revit family contains no types.'))
    warnings = list(payload.get('warnings') or [])
    family, source = payload.get('family', {}), payload.get('source', {})
    materials = {}
    for key, raw in payload.get('materials', {}).items():
        mat = Material(raw['name'], tuple(v / 255.0 for v in (raw.get('rgb') or (180, 180, 180))),
                       opacity=(1.0 - float(raw['transparency'])) if raw.get('transparency') else None)
        materials[key] = scene.materials[register(scene.materials, mat)]
    zone_layer = None

    def clearance():
        nonlocal zone_layer
        if zone_layer is None:
            # Found again by its key, so renaming the tag keeps it in use.
            zone_layer = next((ly for ly in scene.layers if ly.key == ZONE_TAG_KEY), None)
            if zone_layer is None:
                zone_layer = Layer(tr('Clearance zones'), key=ZONE_TAG_KEY)
                scene.layers.append(zone_layer)
        return zone_layer.key

    definitions = []
    for typ in payload['types']:
        definition = Group(name=typ['name'])
        definition.xform = QMatrix4x4()
        parameters = typ.get('parameters', [])
        definition.ext = {'rfa': deepcopy({'family': family, 'source': source,
                           'parameters': parameters, 'geometry_source': typ['geometry_source'],
                           'preview_png_base64': payload.get('preview_png_base64')})}
        record = {'v': 1, 'class': 'IfcBuildingElementProxy', 'name': typ['name'],
                          'psets': {'Revit family': {
                              'Family': family.get('name', ''), 'Category': family.get('category', ''),
                              'Source file': source.get('file_name', '')},
                              'Revit parameters': {p['name']: _property(p) for p in parameters}}}
        if family.get('omniclass'):
            record['classifications'] = [{'system': 'omniclass', 'code': family['omniclass']}]
        definition.ifc = record
        solids = []
        if typ['geometry_source'] == 'none':
            solids.append(_placeholder(parameters, warnings, typ['name']))
        for component in typ.get('components', []):
            geometry = payload['geometries'][component['geometry']]
            matrix = component['transform']
            if len(matrix) != 16 or not all(math.isfinite(v) for v in matrix):
                raise ValueError(tr('Invalid Revit component transform.'))
            transform = QMatrix4x4(*matrix)
            grouped = {}
            for part in geometry['parts']:
                mat = materials.get(part.get('material'))
                if part.get('zone'):
                    zone = Material((mat.name if mat else 'Revit') + ' — Clearance',
                                    mat.color if mat else (0.3, 0.6, 0.9), opacity=0.2)
                    mat = scene.materials[register(scene.materials, zone)]
                g = Group(name=mat.name if mat else geometry.get('label', 'Solid'))
                g.component = False
                g.hidden = bool(part.get('hidden'))
                g.material = mat.face_attrs() if mat else None
                if part.get('zone'):
                    g.layer = clearance()
                g.ext = {'rfa': {'solid': part['solid'], 'style': deepcopy(payload.get('styles', {}).get(part.get('style')))}}
                g.mesh = _triangle_mesh(part['positions'], part['indices'])
                grouped.setdefault(part['solid'], []).append(g)
            for solid_id, parts in grouped.items():
                solid = Group(name=parts[0].name)
                solid.adopt(parts)
                solid.xform = QMatrix4x4(transform)
                solid.component = False
                solid.hidden = all(p.hidden for p in parts)
                if all(p.layer for p in parts):
                    solid.layer = parts[0].layer
                solid.ext = {'rfa': {'solid': solid_id, 'geometry': component['geometry']}}
                solids.append(solid)
        definition.adopt(solids)
        definitions.append(definition)
    scene.kept_components.extend(definitions)
    scene.groups.append(place_copy(definitions[0]))
    scene.version += 1
    return warnings


class RfaImport(SnapshotImport):
    """Keep templates, paints and classification registry in the undo step."""
    _TRACKED = SnapshotImport._TRACKED + (('kept_components', 'added_templates', 'removed_templates'),)

    def do(self, scene):
        from core.bim_registry import get
        first = self.after is None
        if first:
            self.materials_before = dict(scene.materials)
        try:
            super().do(scene)
            if first:
                self.materials_after = dict(scene.materials)
                registry = get(scene)
                registry.setdefault('classification_systems', {}).setdefault('omniclass', {'name': 'OmniClass'})
                self.registry_command = SetRegistryCommand(registry)
            scene.materials = dict(self.materials_after)
            self.registry_command.do(scene)
        except Exception:
            scene.materials = dict(self.materials_before)
            raise

    def undo(self, scene):
        super().undo(scene)
        scene.materials = dict(self.materials_before)
        if hasattr(self, 'registry_command'):
            self.registry_command.undo(scene)
