# SPDX-License-Identifier: GPL-3.0-or-later
"""Shared material/style identities and presentation layers for each export."""
import built_core
from collections import defaultdict
from .step import _s, _f


def ref(node):
    return '#' + str(node)


def refs(nodes):
    return '(' + ','.join(map(ref, nodes)) + ')'


class Appearance:
    def __init__(self, scene, writer, relation):
        self.scene, self.w, self.relation = scene, writer, relation
        self.materials, self.styles = {}, {}
        self.layers = defaultdict(set)

    def material(self, name):
        if name not in self.materials:
            self.materials[name] = self.w.entity('IfcMaterial', Name=_s(name))
        return self.materials[name]

    def apply(self, record, group, faces, node, guid, representation, item,
              attributes=None, face_areas=None):
        w = self.w
        areas = defaultdict(float)
        if attributes is None:
            attributes = [face.attrs for face in faces]
            face_areas = [face.area() for face in faces]
        for attrs, area in zip(attributes, face_areas):
            if attrs.get('mat'):
                areas[attrs['mat']] += area
        paint = getattr(group, 'material', None) or {}
        material = record.get('material') or (max(areas, key=areas.get) if areas else None) or paint.get('mat')
        name = material.get('name', '') if isinstance(material, dict) else material
        if name:
            mat = self.material(name)
            if isinstance(material, dict) and material.get('layers'):
                key = (name, tuple((layer['name'], layer['thickness']) for layer in material['layers']))
                if key not in self.materials:
                    layers = [w.entity('IfcMaterialLayer', Material=ref(self.material(layer['name'])),
                                       LayerThickness=_f(layer['thickness'])) for layer in material['layers']]
                    self.materials[key] = w.entity('IfcMaterialLayerSet', MaterialLayers=refs(layers), LayerSetName=_s(name))
                mat = self.materials[key]
            self.relation('IfcRelAssociatesMaterial', (guid, 'material'), RelatedObjects=refs([node]), RelatingMaterial=ref(mat))
        recipe = self.scene.materials.get(name)
        color = getattr(recipe, 'color', None)
        opacity = getattr(recipe, 'opacity', None)
        if color is None:
            source = next((a for a in attributes if a.get('mat') == name and a.get('color')), paint)
            color, opacity = source.get('color'), source.get('opacity')
        if color is not None and item is not None:
            alpha = opacity if opacity is not None else color[3] if len(color) > 3 else 1.
            key = (name, tuple(color[:3]), alpha)
            if key not in self.styles:
                rgb = w.entity('IfcColourRgb', Red=_f(color[0]), Green=_f(color[1]), Blue=_f(color[2]))
                render = w.entity('IfcSurfaceStyleRendering', SurfaceColour=ref(rgb), Transparency=_f(1-alpha), ReflectanceMethod='.NOTDEFINED.')
                self.styles[key] = w.entity('IfcSurfaceStyle', Name=_s(name or 'Colour'), Side='.BOTH.', Styles=refs([render]))
            for occurrence in item if isinstance(item, list) else [item]:
                w.entity('IfcStyledItem', Item=ref(occurrence), Styles=refs([self.styles[key]]))
        if representation is not None:
            from built_core.layers import DEFAULT_LAYER
            names = {a.get('layer') or getattr(group, 'layer', None) or DEFAULT_LAYER for a in attributes}
            if group is not None:
                names.add(group.layer or DEFAULT_LAYER)
            for layer in self.scene.layers:
                if layer.key in names:
                    # The folder path: unique and readable, where a bare
                    # label may repeat and a key may be a uid.
                    self.layers[(built_core.tag_edits.path_label(self.scene, layer.key),
                                 layer.uid)].add(representation)

    def finish(self):
        for (name, uid), representations in self.layers.items():
            self.w.entity('IfcPresentationLayerAssignment', Name=_s(name), Identifier=_s(uid), AssignedItems=refs(sorted(representations)))


def junction_connections(inventory, products, root):
    """Write each child's connection even though the assembly is flat."""
    for uid, row in inventory.rows.items():
        members = inventory.members[uid]
        if not members or members[0] not in products:
            continue
        main = members[0]
        parts = [p for p in inventory.realizers[uid] if p in products]
        detail = row.get('detail') or {}
        name = detail.get('name') or 'Custom junction'
        if row.get('meeting') == 'hosted' and not any(
                products[p][3] in ('IfcMechanicalFastener', 'IfcFastener') for p in parts):
            continue
        for other in dict.fromkeys(members[1:]):
            if other == main or other not in products:
                continue
            attrs = dict(RelatingElement=ref(products[main][0]), RelatedElement=ref(products[other][0]))
            if {'main', 'joining'} <= row['roles'].keys() and all(products[p][3] == 'IfcWall' for p in (main, other)):
                roles = row['roles']
                ends = {'start': '.ATSTART.', 'end': '.ATEND.'}
                tee = roles['main'].get('connection') == 'path' or detail.get('uid') == 'a2ff384d2a3b97b3'
                attrs.update(RelatingPriorities='()', RelatedPriorities='()',
                             RelatingConnectionType='.ATPATH.' if tee else ends.get(roles['main'].get('end'), '.ATPATH.'),
                             RelatedConnectionType=ends.get(roles['joining'].get('end'), '.ATPATH.'))
                cls = 'IfcRelConnectsPathElements'
            elif parts:
                attrs.update(RealizingElements=refs([products[p][0] for p in parts]), ConnectionType=_s(name))
                cls = 'IfcRelConnectsWithRealizingElements'
            else:
                cls = 'IfcRelConnectsElements'
            root(cls, ('junction', uid, main, other), **attrs)
