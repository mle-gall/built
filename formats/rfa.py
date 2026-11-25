# SPDX-License-Identifier: GPL-3.0-or-later
"""Optional Revit reader and deterministic JSON-to-scene mapping (metres, Z up)."""
from __future__ import annotations

import math
from functools import partialmethod


from core.history import SnapshotImport, SetRegistryCommand
from core.i18n import tr


from ingetrazo_core import rfa_history_do, rfa_history_undo, rfa_read as read_rfa, rfa_triangle_mesh as _triangle_mesh, rfa_apply as apply_payload


#: The "Clearance zones" tag's key: every .rfa import puts zones on it.
ZONE_TAG_KEY = 'rfa-clearance-zones'


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


def _family_record(typ, family, source, parameters):
    record = {'v': 1, 'class': 'IfcBuildingElementProxy', 'name': typ['name'],
                      'psets': {'Revit family': {
                          'Family': family.get('name', ''), 'Category': family.get('category', ''),
                          'Source file': source.get('file_name', '')},
                          'Revit parameters': {p['name']: _property(p) for p in parameters}}}
    if family.get('omniclass'):
        record['classifications'] = [{'system': 'omniclass', 'code': family['omniclass']}]
    return record


class RfaImport(SnapshotImport):
    """Keep templates, paints and classification registry in the undo step."""
    _TRACKED = SnapshotImport._TRACKED + (('kept_components', 'added_templates', 'removed_templates'),)

    do = partialmethod(rfa_history_do)
    undo = partialmethod(rfa_history_undo)
