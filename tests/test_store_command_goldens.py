"""Ordered store geometry and paint captured from the pre-port history commands."""
import hashlib
import json
import random

import pytest
from ingetrazo_core import MeshStore
from core.scene import Scene
from tests.test_store_mesh import fingerprint
from tests.test_store_pieces_explode import _explode_scene, _translate


def rooms(seed):
    rng = random.Random(seed)
    scene = Scene()
    for index in range(2 + seed % 5):
        x, y = rng.randrange(6), rng.randrange(5)
        if seed % 2:
            x += 650_000
        w, h = rng.randrange(1, 5), rng.randrange(1, 5)
        loop = [(x, y, 0), (x+w, y, 0), (x+w, y+h, 0), (x, y+h, 0)]
        if index % 3 == 2:
            loop.reverse()
        scene.mesh.add_face(loop).attrs["mat"] = str(index)
    return scene


def golden_state(mesh):
    # Float arithmetic from affine placements can differ below model precision.
    def rounded(value):
        if isinstance(value, float):
            return round(value, 8) or 0.0
        if isinstance(value, (tuple, list)):
            return [rounded(item) for item in value]
        if isinstance(value, dict):
            return {key: rounded(item) for key, item in value.items()}
        return value
    return rounded(fingerprint(mesh))


@pytest.mark.parametrize("operation,expected", [
    ("rooms", "2187f59a8b376a00058359d3d084380ca193ad9bfc811cd683a015a8aa24e970"),
    ("placed", "5448b8fb93e4eb67f0be84fbf0c4b002deb7234a3239a887379fe3b5f9be74d5"),
])
def test_store_commands_keep_historical_meshes(monkeypatch, operation, expected):
    monkeypatch.setenv("INGETRAZO_STORE", "1")
    states = []
    for seed in range(16):
        if operation == "rooms":
            scene = rooms(seed)
            if seed % 3:
                faces = list(scene.mesh.faces[1:]) if seed % 2 else None
                keys = [f.key for f in faces] if faces is not None else None
                MeshStore.command_rebuild_plane(scene.mesh, (0.,0.,0.), (0.,0.,1.), keys)
            else:
                MeshStore.command_rebuild_planar(scene.mesh)
        else:
            scene, group, _ = _explode_scene(True)
            matrix = _translate((seed % 5) * .5, (seed % 3) * .5, 0., 90. * (seed % 4))
            MeshStore.command_explode_geometry(scene.mesh, group.mesh, matrix.data(), group.material)
        states.append(golden_state(scene.mesh))
    encoded = json.dumps(states, sort_keys=True, separators=(",", ":")).encode()
    assert hashlib.sha256(encoded).hexdigest() == expected
