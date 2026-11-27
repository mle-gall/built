# SPDX-License-Identifier: GPL-3.0-or-later
from pathlib import Path

import pytest

from built_core.scene import Scene
from formats import step


def test_cylinder_seam_and_cap_survive_exchange(tmp_path):
    src = Path(__file__).parent / "fixtures" / "step" / "capped-cylinder.step"
    prepared = step.prepare_step(src)
    assert (prepared["exact"], prepared["faceted"], prepared["problems"]) == (1, 0, [])
    scene = Scene()
    step.apply_step(scene, prepared)
    mesh = scene.groups[0].mesh
    assert mesh._store.native_exact
    assert (len(mesh.vertices), len(mesh.edges), len(mesh.faces)) == (2, 3, 3)
    path = tmp_path / "roundtrip.step"
    step.save_step(scene, path)
    doc = step.prepare_step(path)
    assert doc["exact"] == 1 and not doc["problems"]
    text = path.read_text()
    assert text.count("= CYLINDRICAL_SURFACE(") == 1
    assert text.count("= CIRCLE(") == 2


def test_partial_import_requires_acceptance():
    scene = Scene()
    with pytest.raises(ValueError, match="confirmation"):
        step.apply_step(scene, {"problems": ["unsupported body"], "groups": []})
    assert not scene.groups
