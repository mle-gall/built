"""Adapters between tool edge references, renderer visibility and the f64 query."""
from ingetrazo_core import Vec3 as QVector3D
from ingetrazo_core import SnapQuery, PointVisibility, edge_tag, edge_columns, point_inputs


def make_query(edges, projector, cursor):
    projection = getattr(projector, 'snap_projection', None)
    args = (None, None, cursor, projector) if projection is None else (*projection, cursor)
    if hasattr(edges, 'snap_columns'):
        return SnapQuery(*edges.snap_columns(), *args)
    return SnapQuery.from_objects(edges, *args)


def point_result(query, edges, radius, hidden, *, midpoints=False, quadrants=(),
                 centers=(), line=None, hidden_many=None):
    from core.snap import (SnapResult, COLOR_ENDPOINT, COLOR_MIDPOINT, COLOR_ORIGIN,
                           COLOR_ON_FACE)
    quads, extras = point_inputs(quadrants, centers)
    occluded = PointVisibility(hidden, hidden_many or None) if hidden else None
    choice = query.points(radius, midpoints, quads, extras, occluded, line)
    if choice is not None:
        point, index, kind = choice
        context = (getattr(edges[index], 'context', None) if index >= 0 else
                   centers[-2-index][1] if index <= -2 else None)
        colors = {'midpoint': COLOR_MIDPOINT, 'quadrant': COLOR_MIDPOINT,
                  'arc_midpoint': COLOR_MIDPOINT, 'face_center': COLOR_ON_FACE,
                  'origin': COLOR_ORIGIN, 'component_origin': COLOR_ORIGIN}
        return SnapResult(QVector3D(*point), kind, colors.get(kind, COLOR_ENDPOINT),
                          context=context)
    return None
