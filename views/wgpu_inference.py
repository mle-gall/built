"""Resident inference API for P1 (no GL bake or viewport construction).

``prepare(packed, scene=None) -> ResidentInference`` runs in the existing
background preparation job. Pass scene to retain loose hidden/soft edges;
packed.groups supplies placement meshes. Reprepare on hidden-geometry/layer
policy changes, as for the render pack. ``update(ids, packed, changed_codes=())``
retains unchanged BVHs; call after placement, geometry and flag changes, including
billboards and edit-rest flags (64 = rest, 128 = hidden rest). No document-sized BVH rebuild on hover.

``nearby_edges(index, packed, camera, width, height, x, y, *, radius_px=48,
cap=48, rows=None, edit_group=None)`` returns nearest world-space pseudo edges
(a, b, row, edge_id, distance, group, in_group, context). Use uid_rows to supply
an optional row subset; use group/edit_group to split loose/group facade calls.
These are inference records, NOT editable topology Edges. edge_id is the filtered
prototype edge ordinal, not a mesh edge ID. Resolve topology before editing.
``reference_edge(index, view, x, y, radius_px=None)`` returns one visible pseudo
edge or None for Tape/Protractor/Rotate. ``tangent_direction(index, packed, row,
point)`` returns a world unit direction at a curve endpoint or None.
``scene_edges(view)`` returns guide/point/image-border pseudo edges (source is
the original record). Image records implement border_edges() and locked; scene
supplies entity_selectable(). ``snap(index, row, view, x, y, **options)`` returns
GL's SnapResult, even with row=None. All compute_snap lock/reference/acquisition
keywords pass through; view supplies scene, packed, camera, width/height,
_pixel_to_ray, _world_to_pixel and optionally tools.work_point/tool and GL pixel
tolerances. P1 must call snap without a picked face too. ``surface(index, row,
origin, direction)`` also accepts row arrays and returns a world point or None.

Kinds/priorities are owned by core.snap (endpoint, midpoint, on_edge, on_face,
intersection, center, parallel, perpendicular, reference, axis, from_point,
etc.). Arc tools consume tangent_direction to produce their tangent preview;
tangent is not a new competing compute_snap rule. Defaults retain the previous
12 px snap tolerance when the GL thresholds are not supplied by the facade.
"""
from ingetrazo_core import resident_snap as snap, resident_prepare as prepare, resident_rebase as rebase, tangent_direction
from ingetrazo_core import (Vec3 as QVector3D, context_hit, eligible_rows,
                            packed_surface, screen_boxes, screen_rows as _screen_rows, surface, surface_hit)
from ingetrazo_core import (acquire as _acquire, acquire_rim as _acquire_rim, face_centers_near,
                            face_crossing, front_at as _front_at, nearby_edges, occluded as _occluded,
                            resident_occluded_many as _occluded_many, on_surface, placement_context,
                            reference_edge, scene_edges, snap_context, start_edges)
