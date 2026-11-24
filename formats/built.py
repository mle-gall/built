"""Built document identity and precision at the Qt/model boundary."""
from pathlib import Path

from ingetrazo_core import (
    built_position as position,
    built_placement as placement,
    built_placement_values as placement_values,
    built_point_array as point_array,
    built_compact_values as compact_values,
    built_python_vertices as python_vertices,
    built_export_placements as export_placements,
)

BUILT_SUFFIX = ".built"
DOCUMENT_SUFFIXES = (BUILT_SUFFIX, ".igz")
DOCUMENT_FILTER = f"Documents (*{BUILT_SUFFIX} *.igz)"
BUILT_FILTER = f"Built document (*{BUILT_SUFFIX})"
WARNING_MM = 0.0254


def is_built(path):
    return Path(path).suffix.lower() == BUILT_SUFFIX


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
