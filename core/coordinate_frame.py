"""Floating origins shared by the Metal buffers and its Qt geometry planners."""
from contextlib import contextmanager

import numpy as np

ZERO = (0., 0., 0.)
# At 512 m a float32 step is 0.061 mm, below the 0.1 mm weld tolerance.
REBASE_DISTANCE = 512.


def choose_origin(center, previous=ZERO):
    if np.max(np.abs(np.subtract(center, previous))) <= REBASE_DISTANCE:
        return tuple(previous)
    return tuple(map(float, np.rint(center)))


def point_in_frame(point, origin):
    from PySide6.QtGui import QVector3D
    return QVector3D(*(v - o for v, o in zip(point, origin)))


@contextmanager
def mesh_frame(mesh, origin):
    """Run a Qt tool on local coordinates. Its document and history keep model keys."""
    st = mesh.__dict__.get('_store')
    if st is None or tuple(st.frame_origin) == tuple(origin):
        yield
        return
    d = mesh.__dict__
    parent = d.get('_frame_parent')
    cached = d.get('_tool_frame_cache')
    if parent is not None and tuple(parent.frame_origin) == tuple(origin):
        local = parent
        if st.revision != parent.revision:
            local._publish_frame(st)
    elif cached and cached[:3] == (st, st.revision, tuple(origin)):
        local = cached[3]
    else:
        local = st.in_frame(origin)
    start = local.revision
    d['_frame_parent'] = st
    d['_store'] = local
    try:
        yield
    finally:
        local = d.get('_store')
        if local is None:
            # A compatibility command explicitly requested Python objects.
            # Its graph and tombstones leave the tool in the caller's frame.
            from core.mesh import Vertex, _key
            from PySide6.QtGui import QVector3D
            delta = np.subtract(origin, st.frame_origin)
            holders = list(d["_registry"].values())
            vertices = {v.key: v for v in d.get("_objmap", {}).values() if isinstance(v, Vertex)}
            vertices.update((v.key, v) for v in mesh.vertices)
            for v in vertices.values():
                v.position = QVector3D(*(np.asarray(v.position.toTuple()) + delta))
            d["_registry"] = {_key(v.position): v for v in holders}
            d["_displaced"] = {}
            for v in mesh.vertices:
                cell = _key(v.position)
                if d["_registry"].setdefault(cell, v).key != v.key:
                    d["_displaced"].setdefault(cell, []).append(v)
        else:
            if local.revision != start and st.revision != local.revision:
                st._publish_frame(local)
            d['_store'] = st
        if parent is None:
            d.pop('_frame_parent', None)
        else:
            d['_frame_parent'] = parent
        if local is not None:
            d['_tool_frame_cache'] = (st, st.revision, tuple(origin), local)
        else:
            d.pop('_tool_frame_cache', None)


@contextmanager
def planned_points(mesh, points):
    """Keep supplied or calculated doubles behind Qt points used to plan topology."""
    if not points:
        yield
        return
    previous = mesh.__dict__.get('_planned_points')
    mesh._planned_points = points
    try:
        yield
    finally:
        if previous is None:
            mesh.__dict__.pop('_planned_points', None)
        else:
            mesh._planned_points = previous


def drawing_point(mesh, point):
    """Resolve a snapped endpoint back to its stored doubles in the tool frame."""
    vertex = mesh.vertex_at(point)
    return np.array(vertex.position_tuple() if vertex is not None else point.toTuple())
