# SPDX-License-Identifier: GPL-3.0-or-later
"""Time materialisation, nonempty bulk welding and face linking on a 100×100 grid.

Run with PYTHONPATH pointing at the checkout under test and that checkout's
release extension installed. Each phase reports milliseconds.
"""
import gc
import json
from time import perf_counter

import numpy as np

from core.store_mesh import empty_store_mesh


def measure(operation):
    start = perf_counter()
    value = operation()
    return (perf_counter() - start) * 1000, value


def main():
    side = 100
    x, y = np.indices((side, side))
    points = np.zeros((side, side, 4, 3))
    for i, (dx, dy) in enumerate(((0, 0), (1, 0), (1, 1), (0, 1))):
        points[:, :, i, 0] = x + dx
        points[:, :, i, 1] = y + dy
    points = points.reshape(-1, 3)
    sizes = np.full(side * side, 4)
    counts = np.ones(side * side, dtype=int)
    mesh = empty_store_mesh()
    mesh.add_faces_bulk(points, sizes, counts)
    assert len(mesh.faces) == side * side

    gc.collect()
    materialise, _ = measure(mesh._materialise)
    gc.collect()
    weld, (vertices, indices) = measure(lambda: mesh.bulk_weld(points + [0., 0., 12.]))
    assert len(vertices) == (side + 1) ** 2
    faces, _ = measure(lambda: mesh.add_faces_welded(vertices, indices, sizes, counts))
    assert len(mesh.faces) == 2 * side * side
    print(json.dumps(dict(materialise=materialise, weld=weld, faces=faces)))


if __name__ == "__main__":
    main()
