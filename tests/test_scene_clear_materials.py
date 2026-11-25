# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #759: File ▸ New kept the previous document's materials.

``Scene.clear()`` reset the geometry, layers, georef, styles… but not
``scene.materials``, and ``save_scene`` writes every registered material,
used or not — so the previous document's material library survived
File ▸ New and was saved into the next project."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import zipfile

import pytest
from built_render import SurveyRaster
from built_core import Vec3 as QVector3D


from built_core.materials import Material                                  # noqa: E402
from built_core.scene import Scene                                         # noqa: E402
from formats import igz as igz_format                                # noqa: E402


def _quad(mesh, z=0.0):
    return mesh.add_face([QVector3D(0, 0, z), QVector3D(2, 0, z),
                          QVector3D(2, 2, z), QVector3D(0, 2, z)])


def test_clear_drops_the_material_registry():
    """Materials alone are document content too: a scene holding only
    registered materials is not already empty, so clear() must drop
    them and bump the version like any other reset."""
    scene = Scene()
    scene.materials["Marble"] = Material("Marble", color=(0.9, 0.9, 0.88))
    scene.materials["Concrete"] = Material("Concrete",
                                           color=(0.6, 0.6, 0.6))
    version = scene.version
    scene.clear()
    assert scene.materials == {}
    assert scene.version > version


def test_a_new_document_does_not_save_the_previous_materials(tmp_path):
    """The #759 round trip: textured model ▸ File ▸ New ▸ a plain box ▸
    Save. The new file must carry none of the old materials, and none
    of their textures may be embedded in the container."""
    img = tmp_path / "marble.png"
    assert SurveyRaster(4, 4, bytes((0, 0, 0, 255)) * 16).save(str(img))

    scene = Scene()
    tex = {"path": str(img), "sw": 1.0, "sh": 1.0}
    mat = Material("Marble", texture=dict(tex))
    scene.materials[mat.name] = mat
    face = _quad(scene.mesh)
    face.attrs.update(mat.face_attrs())

    scene.clear()                                    # File ▸ New
    assert scene.materials == {}

    _quad(scene.mesh)                                # the new, plain box
    path = tmp_path / "box.igz"
    igz_format.save_scene(scene, path)
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as zf:
            assert not any(n.startswith("textures/")
                           for n in zf.namelist())

    loaded = Scene()
    igz_format.load_into(loaded, path)
    assert loaded.materials == {}
    assert len(loaded.mesh.faces) == 1
