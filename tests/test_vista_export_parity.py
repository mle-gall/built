"""End-to-end face fills recorded before moving the export walk to Rust."""
import json
from pathlib import Path

import numpy as np
import pytest

from built_core.scene import Scene
from built_core import vista_caras as vista
from tests.test_export_view_svg import _camera, _slab, _sunny


def observation(perspective):
    scene = Scene()
    _slab(scene.loose_mesh, 0, 0, 0, 3, 2, 1)
    _slab(scene.loose_mesh, 1, 0.5, 1, 1, 1, 1.5)
    for face in scene.loose_mesh.faces:
        face.attrs["color"] = (0.7, 0.55, 0.3)
        face.attrs["texture"] = {"path": "brick.png", "sw": 0.25, "sh": 0.5}
    _sunny(scene)
    camera = _camera(scene)
    camera.perspective = perspective
    return {"rect": vista.view_rect(camera), "faces": [
        {"layer": f.layer, "rgb": f.rgb, "depth": f.depth, "back": f.back,
         "opacity": f.opacity, "alpha": f.shadow_alpha, "texture": f.texture,
         "rings": [r.tolist() for r in f.rings],
         "shadow": [r.tolist() for r in (f.shadow or [])]}
        for f in vista.view_faces(scene, camera, shadows=True)]}


@pytest.mark.parametrize("perspective", [False, True])
def test_face_export_matches_python(perspective):
    expected = json.loads((Path(__file__).parent / "fixtures" /
                           "vista_export_python.json").read_text())[int(perspective)]
    actual = observation(perspective)
    # BLAS can reorder dot products. Allow 2 nm in projected rings and pins.
    def compare(a, b):
        if isinstance(b, dict):
            assert a.keys() == b.keys()
            for key in b:
                compare(a[key], b[key])
        elif isinstance(b, list):
            assert len(a) == len(b)
            for left, right in zip(a, b):
                compare(left, right)
        elif isinstance(b, float):
            np.testing.assert_allclose(a, b, atol=2e-9, rtol=0)
        else:
            assert a == b
    compare(actual, expected)
