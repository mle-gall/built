"""Adapters between tool edge references, renderer visibility and the f64 query."""
from PySide6.QtGui import QVector3D
import numpy as np

from core import _native


def edge_tag(edge):
    attributes = 8 if getattr(edge, 'figure', False) else 0
    if getattr(edge, 'soft', False) or getattr(edge, 'hidden', False) or getattr(edge, 'curve', None) is not None:
        attributes |= 16
    for tag, name in ((2, 'center'), (3, 'component_origin'), (4, 'arc_midpoint'), (1, 'guide')):
        if getattr(edge, name, False):
            return attributes | tag
    return attributes


def edge_columns(edges):
    return (np.array([e.a.toTuple() for e in edges], dtype=float).reshape(-1, 3),
            np.array([e.b.toTuple() for e in edges], dtype=float).reshape(-1, 3),
            [edge_tag(e) for e in edges],
            [getattr(e, 'context', None) is not None for e in edges])


def make_query(edges, projector, cursor):
    projection = getattr(projector, 'snap_projection', None)
    if projection is None or not hasattr(_native.native, 'SnapQuery'):
        return None
    columns = edges.snap_columns() if hasattr(edges, 'snap_columns') else edge_columns(edges)
    return _native.native.SnapQuery(*columns, *projection, cursor)


def point_result(query, edges, radius, hidden, *, midpoints=False, quadrants=(),
                 centers=(), line=None, hidden_many=None):
    from core.snap import (SnapResult, COLOR_ENDPOINT, COLOR_MIDPOINT, COLOR_ORIGIN,
                           COLOR_ON_FACE)
    extras = [(p.toTuple(), context is not None) for p, context in centers]
    quads = [p.toTuple() for p in quadrants]
    visibility = {}

    def occluded(points):
        keys = [QVector3D(*p).toTuple() for p in points]
        missing = list(dict.fromkeys(k for k in keys if k not in visibility))
        if missing:
            vectors = [QVector3D(*k) for k in missing]
            mask = (hidden_many(vectors) if hidden_many and len(vectors) > 1
                    else map(hidden, vectors))
            visibility.update(zip(missing, mask))
        return [visibility[k] for k in keys]

    choice = query.points(radius, midpoints, quads, extras, occluded if hidden else None, line)
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
