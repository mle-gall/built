"""Built document identity and precision at the Qt/model boundary."""
from pathlib import Path

BUILT_SUFFIX = ".built"
DOCUMENT_SUFFIXES = (BUILT_SUFFIX, ".igz")
DOCUMENT_FILTER = f"Documents (*{BUILT_SUFFIX} *.igz)"
BUILT_FILTER = f"Built document (*{BUILT_SUFFIX})"
WARNING_MM = 0.0254


def is_built(path):
    return Path(path).suffix.lower() == BUILT_SUFFIX


def position(vertex):
    read = getattr(vertex, "position_tuple", None)
    return read() if read else vertex.position.toTuple()


def point_array(points, sink):
    import numpy as np
    from formats.igz import NonFiniteDocumentError

    points = np.asarray(points).reshape(-1, 3)
    compact = points.astype("<f4", copy=False)
    if not np.isfinite(points).all():
        raise NonFiniteDocumentError("geometry")
    if getattr(sink, "built", False) and not np.array_equal(points, compact):
        return points.astype("<f8", copy=False)
    if not np.isfinite(compact).all():
        raise NonFiniteDocumentError("geometry")
    if getattr(sink, "compatibility", False) and points.size:
        mm = displacement_mm(points, compact, sink.current_placements)
        sink.displacement_mm = max(sink.displacement_mm, float(mm))
    return compact


def placement(values):
    """Qt consumes float32. Retain the source doubles on its projection."""
    from PySide6.QtGui import QMatrix4x4
    import math
    values = tuple(map(float, values))
    if len(values) != 16 or not all(map(math.isfinite, values)):
        raise ValueError("Invalid Built placement transform")
    matrix = QMatrix4x4(*(values[c * 4 + r] for r in range(4) for c in range(4)))
    matrix._built_source = values, tuple(matrix.data())
    return matrix


def placement_values(matrix):
    current = matrix.data()
    saved = getattr(matrix, "_built_source", None)
    if saved:
        values, projected = saved
        return [v if p == c else c for v, p, c in zip(values, projected, current)]
    return list(current)


def python_vertices(mesh, points, sequential):
    """Build on the model coordinates while Qt displays their f32 projection."""
    from core.lazy_mesh import weld
    from core.mesh import _KEY_SCALE
    from formats.built_vertex import BuiltVertex
    positions, inverse = weld(points, sequential, precise=True)
    for point in positions.tolist():
        vertex = BuiltVertex(point, key=next(mesh._keys))
        mesh._registry[tuple(round(x * _KEY_SCALE) for x in point)] = vertex
        mesh.vertices.append(vertex)
    return mesh.vertices, inverse


def export_placements(scene):
    """For each serialized mesh, its placements before and after f32 export."""
    import numpy as np
    from core import links

    by_mesh = {}
    identity = np.eye(4)
    mesh = getattr(scene, "mesh", scene)
    by_mesh[id(mesh)] = [(identity, identity)]

    def visit(group, world, exported):
        if group.xform is not None:
            if links.save_as_placeholder(group, scene):
                return
            source = np.array(placement_values(group.xform)).reshape(4, 4).T
            target = np.array(group.xform.data()).reshape(4, 4).T
            world, exported = world @ source, exported @ target
        by_mesh.setdefault(id(group.mesh), []).append((world, exported))
        for child in group.children:
            visit(child, world, exported)

    for groups in (getattr(scene, "groups", ()), getattr(scene, "kept_components", ())):
        for group in groups or ():
            visit(group, identity, identity)
    return by_mesh


def displacement_mm(points, compact, placements):
    import numpy as np
    equal_points = np.array_equal(points, compact)
    maximum = 0.0
    for source, target in placements:
        equal_transform = np.array_equal(source, target)
        if equal_points and equal_transform:
            continue
        for start in range(0, len(points), 65536):
            p = points[start:start + 65536].astype(np.float64, copy=False)
            q = compact[start:start + 65536].astype(np.float64)
            # Model placements are affine. Taking the difference first avoids
            # subtracting two large translated world positions.
            delta = (p - q) @ source[:3, :3].T
            if not equal_transform:
                change = source - target
                delta += q @ change[:3, :3].T + change[:3, 3]
            maximum = max(maximum, float(np.linalg.norm(delta, axis=1).max()))
    return maximum * 1000
