# SPDX-License-Identifier: GPL-3.0-or-later
"""Topology edits and their inverses against recorded object-mesh states."""
import gzip
import hashlib
import json
from pathlib import Path
import random

import pytest
from ingetrazo_core import Vec3
from core.mesh import Mesh, turn_face_over
from tests.test_store_mesh import fingerprint


def history(seed):
    rng = random.Random(seed)
    mesh = Mesh()
    snapshots = []
    removed_faces = []
    removed_edges = []

    def point():
        return Vec3(rng.randrange(6), rng.randrange(6), 0)

    for _ in range(96):
        op = rng.randrange(14)
        faces = list(mesh.faces)
        edges = list(mesh.edges)
        if op == 0 and faces:
            face = rng.choice(faces)
            mesh.remove_face(face)
            removed_faces.append(face)
        elif op == 1 and removed_faces:
            mesh.relink_face(removed_faces.pop())
        elif op == 2 and edges:
            edge = rng.choice(edges)
            if not edge.faces:
                mesh.remove_edge(edge)
                removed_edges.append(edge)
        elif op == 3 and removed_edges:
            edge = removed_edges.pop()
            if mesh.find_edge(edge.v0, edge.v1) is None:
                mesh.relink_edge(edge)
        elif op == 4 and faces:
            face = rng.choice(faces)
            turn_face_over(face)
        elif op == 5 and edges:
            edge = rng.choice(edges)
            if not edge.faces and edge.v0 != edge.v1:
                mesh.split_edge(edge, (edge.a + edge.b) * 0.5)
        elif op == 6 and faces:
            face = rng.choice(faces)
            if face.hole_loops:
                mesh.remove_hole(face, face.hole_loops[0])
            else:
                origin = face.vertices[0]
                mesh.add_hole(face, [origin + Vec3(x, y, 0) for x, y in
                                     ((0.2, 0.2), (0.2, 0.4), (0.4, 0.2))])
        elif op == 7:
            snapshots.append(mesh.capture_state())
        elif op == 8 and snapshots:
            mesh.restore_state(rng.choice(snapshots))
            removed_faces.clear()
            removed_edges.clear()
        elif op == 9:
            mesh.purge_debris()
        elif op == 10:
            mesh.dedupe_faces()
        elif op == 11 and edges:
            edge = rng.choice(edges)
            edge.soft = not edge.soft
        elif op == 12:
            mesh.add_edge(point(), point() + Vec3(0.5, 0, 0))
        else:
            origin = point()
            face = mesh.add_face([origin + Vec3(x, y, 0) for x, y in
                                  ((0, 0), (1, 0), (1, 1), (0, 1))])
            face.attrs['row'] = rng.randrange(5)
        payload = json.dumps(fingerprint(mesh), sort_keys=True, separators=(',', ':'))
        yield hashlib.sha256(payload.encode()).hexdigest()


@pytest.mark.parametrize('seed', range(32))
def test_topology_history(seed):
    with gzip.open(Path(__file__).parent / 'fixtures' / 'topology_history.json.gz', 'rt') as src:
        expected = json.load(src)[seed]
    for step, (actual, prior) in enumerate(zip(history(seed), expected, strict=True)):
        assert actual == prior, (seed, step)
