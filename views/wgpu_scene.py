"""Pack resident GPU arrays without creating faces for deferred geometry.

The ABI is little endian: 28-byte vertices, 32-byte prototype bounds,
80-byte instances and 20-byte indexed indirect commands. Alpha zero marks
unpainted faces so container tint never overwrites explicit face colour.
Alpha 1..255 is rounded effective opacity (clamped to at least 1).
Both-side paint lives in the existing material corner records, never alpha.
Instance info[2]: bit 0 suppresses ordinary drawing; bit 3 is billboard;
bit 4 marks an object hidden by Hide (including descendants). Reveal bit 4
only when bit 5 is clear: bit 5 means tag/navigator/edit filtering also hides
it. Hidden commands use the same vertex/index buffers and COMMAND dtype.
Materials use 16-byte prototype headers followed by 32-byte corner records
only for prototypes with textures or backs that differ from their fronts.
"""
import numpy as np
from core import _native
from core.geometry_arrays import arrays_of
from core.lazy_mesh import plan_of
from views.array_bake import _attrs_table
from ingetrazo_core import (COMMAND, INSTANCE, OBJECT_HIDDEN, VERTEX, VISIBILITY_FILTERED, PackedScene,
                            PlacementState as _placement_state, baked_paint as _baked_paint,
                            doc_worlds as _doc_worlds, native_records as _native_records, open_focus,
                            pack_scene, pbr_entry as _pbr_entry, placement_records as _placement_records,
                            proxy_records as _proxy_records, source_arrays as _source_arrays)


def bake_mesh(mesh):
    """Reuse the array bake's kernel outputs, including hole triangulation."""
    plan = plan_of(mesh)
    if plan is not None:
        arrays = plan
        src = mesh.__dict__['_deferred_source']
        table = _attrs_table(src)
        attr = np.asarray(src['face_attr'], dtype=np.int64)
        pos = plan.pos
    else:
        arrays = arrays_of(mesh)
        pos = arrays.pos
        if type(mesh).__name__ == 'StoreMesh':
            from core.store_mesh import attr_columns
            table, attr = attr_columns(mesh)
            attr = np.asarray(attr, np.int64)
        else:
            table = [f.attrs for f in mesh.faces]
            attr = np.arange(len(table))
    # Gather once through the attribute table; masking every face for each
    # distinct colour is quadratic on large, per-face-painted prototypes.
    palette = np.full((len(table)+1, 4), 255, np.uint8)
    palette[:, :3] = 210
    palette[:, 3] = 0
    hidden_table = np.zeros(len(table)+1, bool)
    for k, own in enumerate(table):
        hidden_table[k+1] = bool(own.get('hidden'))
        if own.get('color') is not None:
            palette[k+1, :3] = np.clip(np.array(own['color'][:3])*255, 0, 255)
            palette[k+1, 3] = 255
    colors = palette[attr+1]
    hidden = hidden_table[attr+1]
    native = _native.available()
    return native.bake_vertices(
        np.asarray(pos, np.float32), arrays.corners, arrays.ring_sizes,
        arrays.ring_counts, colors, hidden, _native.qt_hypot()).view(VERTEX)



def _texture_array(paths, *, device_limit=8192, memory_budget=256*1024*1024):
    """Bottom-up RGBA8 array, complete mips, stable 1-based texture IDs.

    Common layer dimensions retain source resolution up to device_limit (8192
    conservative default; caller may pass the adapter's actual 2D limit).
    Halve both dimensions until *all* layers including white layer zero and
    every mip fit the 256 MiB GPU budget. CPU source decoding is not budgeted.
    Array textures require equal layer sizes; changing that needs a new ABI.
    """
    from ingetrazo_render import material_texture_array
    return material_texture_array(paths, device_limit, memory_budget)
