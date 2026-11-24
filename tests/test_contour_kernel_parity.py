# SPDX-License-Identifier: GPL-3.0-or-later
"""Golden outputs of the Python contour planner before the kernel port."""
import hashlib
import json
import math
import random

import pytest
from ingetrazo_core import Matrix4, Vec3
from core import mesh as mesh_api
from core.mesh import Mesh
from core.snap import curve_centers_of_face


def contour_results(store, monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE", str(int(store)))
    monkeypatch.setattr(mesh_api, "_CURVE_COUNTER", 1000)
    tags, centers = [], []
    for seed in range(36):
        rng = random.Random(seed)
        mesh = Mesh()
        n = rng.randint(12, 36)
        x = 2**20 if seed % 2 else 0
        points = [Vec3(x + 2*math.cos(math.tau*i/n), 2*math.sin(math.tau*i/n), 3) for i in range(n)]
        face = mesh.add_face(points)
        edges = list(mesh.edges)
        # Bare imported circles, soft arcs crossing the array boundary, and
        # multiple separate drawn arcs sharing a face exercise cyclic order.
        if seed % 4 == 1:
            for edge in edges[-5:] + edges[:4]:
                edge.soft = True
        elif seed % 4 == 2:
            mesh.tag_curve(points, closed=True)
        elif seed % 4 == 3:
            mesh.tag_curve(points[-5:] + points[:4], closed=False)
            mesh.tag_curve(points[5:10], closed=False)
        placement = Matrix4()
        placement.translate(17, -3, 5)
        placement.scale(1.5, 1.5, 2)
        centers.append([[(tuple(round(v, 7) for v in p.toTuple()), round(r, 7), k)
                         for p, r, k in curve_centers_of_face(face, xf)]
                        for xf in (None, placement)])
        first = mesh.tag_curve(points, closed=bool(seed % 3))
        for i in rng.sample(range(n), 5):
            v = points[i]
            attach = Vec3(0, 0, 1) if i % 2 else Vec3(0.2, -0.12, 0)
            mesh.add_edge(v, v + attach)
        mesh.resplit_curves()
        tags.append([first, [e.curve for e in edges], mesh_api._CURVE_COUNTER])
        mesh.resplit_curves()
        tags[-1].append([e.curve for e in edges])
    return tags, centers


def digest(rows):
    return hashlib.sha256(json.dumps(rows, separators=(",", ":")).encode()).hexdigest()


@pytest.mark.parametrize("store", [False, True])
def test_contour_tags_and_centers(store, monkeypatch):
    tags, centers = contour_results(store, monkeypatch)
    assert digest(tags) == "b054b6b9cf167799f937ade02ae265d7acdff7c0193f283bfaeaac13e7757077"
    assert digest(centers) == "fcdbfb7a728115f565309ba22dea410c60b4966b7ba8f988d28f102863e20411"


@pytest.mark.parametrize("store", [False, True])
def test_generator_can_reserve_curve_id_before_tagging(store, monkeypatch):
    monkeypatch.setenv("INGETRAZO_STORE", str(int(store)))
    monkeypatch.setattr(mesh_api, "_CURVE_COUNTER", 1)
    mesh = Mesh()
    a, b = Vec3(), Vec3(1, 2, 0)
    edge = mesh.add_edge(a, b)

    def path():
        mesh.next_curve_id()
        yield a
        yield b

    assert mesh.tag_curve(path(), closed=False) == edge.curve == 2
