"""Floating origins shared by the Metal buffers and its Qt geometry planners."""
from contextlib import ExitStack, contextmanager
from contextvars import ContextVar

import numpy as np

ZERO = (0., 0., 0.)
_active = ContextVar("geometry_frames", default=())
# At 512 m a float32 step is 0.061 mm, below the 0.1 mm weld tolerance.
REBASE_DISTANCE = 512.


def choose_origin(center, previous=ZERO):
    if not np.isfinite(center).all() or np.max(np.abs(np.subtract(center, previous))) <= REBASE_DISTANCE:
        return tuple(previous)
    return tuple(map(float, np.rint(center)))


def point_in_frame(point, origin):
    from ingetrazo_core import Vec3 as QVector3D
    return QVector3D(*(v - o for v, o in zip(point, origin)))


@contextmanager
def mesh_frame(mesh, origin, *, matrix=...):
    """Run a Qt tool on local coordinates. Its document and history keep model keys."""
    st = mesh.__dict__.get('_store')
    origin = tuple(origin)
    if matrix is ...:
        matrix = st.frame_matrix if st is not None else None
    matrix = None if matrix is None else tuple(matrix)
    frame = (origin, matrix)
    def matches(store):
        placement = store.frame_matrix
        return (tuple(store.frame_origin), tuple(placement) if placement is not None else None) == frame

    if st is None or matches(st):
        yield
        return
    d = mesh.__dict__
    definition = d.get('_instance_definition')
    if definition is not None:
        # Start at the definition's f64 positions even if the zero-origin
        # view has already rounded a tiny offset beside a large translation.
        view = definition._store._instance_view(definition, matrix, origin)
        d['_store'] = view
        active = tuple(m for m in _active.get() if m is not mesh)
        token = _active.set(active + (mesh,) if any(origin) else active)
        try:
            yield
        finally:
            _active.reset(token)
            d['_store'] = st
        return
    parent = d.get('_frame_parent')
    cached = d.get('_tool_frame_cache')
    if parent is not None and matches(parent):
        local = parent
        if st.revision != parent.revision:
            local._publish_frame(st)
    elif cached and cached[:3] == (st, st.revision, frame):
        local = cached[3]
    else:
        local = (st._in_model_frame(origin) if matrix is None
                 else st._in_instance_frame(matrix, origin))
    entry = local
    start = local.revision
    active = tuple(m for m in _active.get() if m is not mesh)
    token = _active.set(active + (mesh,) if any(origin) or matrix is not None else active)
    d['_frame_parent'] = st
    d['_store'] = local
    try:
        yield
    finally:
        _active.reset(token)
        local = d.get('_store')
        if local is None:
            # A compatibility command explicitly requested Python objects.
            # Its graph and tombstones leave the tool in the caller's frame.
            from core.mesh import Vertex, _key
            from ingetrazo_core import Vec3 as QVector3D
            holders = list(d["_registry"].values())
            vertices = {v.key: v for v in d.get("_objmap", {}).values() if isinstance(v, Vertex)}
            vertices.update((v.key, v) for v in mesh.vertices)
            for v in vertices.values():
                v.position = QVector3D(*entry._convert_point(st, v.position.toTuple()))
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
            d['_tool_frame_cache'] = (st, st.revision, frame, local)
        else:
            d.pop('_tool_frame_cache', None)


@contextmanager
def model_frame():
    active = _active.get()
    if not active:
        yield
        return
    with ExitStack() as frames:
        for mesh in active:
            frames.enter_context(mesh_frame(mesh, ZERO))
        yield


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
