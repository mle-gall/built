# SPDX-License-Identifier: GPL-3.0-or-later
"""Emission in Built's material contract: a linear colour, a strength and
an optional map in ``Material.pbr`` (``emission``, ``emission_strength``,
``emission_map``). Absent means black and 0, so existing materials and
files are unchanged; it round-trips through .igz and glTF (emissiveFactor,
emissiveTexture, KHR_materials_emissive_strength) and the editor shows it."""
from __future__ import annotations

import zipfile
from pathlib import Path

import pytest
from tests.raster_samples import solid_raster
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from built_core import Vec3 as V
from built_core.history import History
from built_core.materials import Material, register
from built_core.protocol import commands as C
from built_core.scene import Scene
from formats import igz
from formats.glb import load_glb, parse_container
from formats.gltf import save_glb


def _image(path, rgb):
    assert solid_raster(2, 3, rgb).save(str(path))
    return str(path)


def _triangle(scene, mat):
    face = scene.loose_mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
    face.attrs.update(mat.face_attrs())
    return face


def _gltf_material(path):
    js, _ = parse_container(path)
    return js, js["materials"][0]


def test_without_emission_nothing_changes(tmp_path):
    scene = Scene()
    _triangle(scene, Material("Plain", (.5, .4, .3), pbr={"roughness": .4}))
    save_glb(scene, tmp_path / "a.glb")
    js, mat = _gltf_material(tmp_path / "a.glb")
    assert "emissiveFactor" not in mat and "emissiveTexture" not in mat
    assert "KHR_materials_emissive_strength" not in js.get("extensionsUsed", [])


def test_set_material_pbr_takes_a_colour_and_a_strength_and_keeps_the_finish():
    scene = Scene()
    h = History(scene)
    register(scene.materials, Material("Lamp", (.9, .9, .9), finish="gloss"))
    C.run(h, "set_material_pbr", name="Lamp", emission=[1.2, .5, -1], emission_strength=4)
    pbr = scene.materials.get("Lamp").pbr
    assert pbr["emission"] == [1.0, .5, 0.0] and pbr["emission_strength"] == 4
    # The finish's look stays: its roughness and metallic came along.
    assert "roughness" in pbr and "metallic" in pbr
    with pytest.raises(ValueError):
        C.run(h, "set_material_pbr", name="Lamp", emission=[1, 1])
    C.run(h, "set_material_pbr", name="Lamp", emission=None, emission_strength=None)
    assert "emission" not in (scene.materials.get("Lamp").pbr or {})
    h.undo()
    h.undo()
    assert not scene.materials.get("Lamp").pbr


def test_emission_alone_is_not_a_pbr_surface(tmp_path):
    scene = Scene()
    glow = Material("Glow", (.2, .2, .2), finish="gloss", pbr={"emission": [1, .5, 0], "emission_strength": 1.0})
    register(scene.materials, glow)          # the finish is the registry's
    _triangle(scene, glow)
    save_glb(scene, tmp_path / "g.glb")
    _, mat = _gltf_material(tmp_path / "g.glb")
    assert mat["emissiveFactor"] == [1, .5, 0]
    assert mat.get("extras", {}).get("ingetrazo_finish") == "gloss"


def test_igz_keeps_emission_and_packs_its_map(tmp_path, monkeypatch):
    monkeypatch.setenv("BUILT_TEXTURE_CACHE", str(tmp_path / "cache"))
    path = _image(tmp_path / "glow.png", (200, 120, 10))
    src = Path(path).read_bytes()
    mat = Material("Sign", (.1, .1, .1), pbr={"emission": [1, .6, .1], "emission_strength": 2.5,
                                              "emission_map": {"path": path}})
    scene = Scene()
    register(scene.materials, mat)
    _triangle(scene, mat)
    doc = tmp_path / "sign.igz"
    igz.save_scene(scene, doc)
    with zipfile.ZipFile(doc) as z:
        assert [n for n in z.namelist() if n.startswith("textures/")]
    Path(path).unlink()
    restored = Scene()
    igz.load_into(restored, doc)
    for pbr in (restored.materials["Sign"].pbr, restored.loose_mesh.faces[0].attrs["pbr"]):
        assert pbr["emission"] == [1, .6, .1] and pbr["emission_strength"] == 2.5
        assert Path(pbr["emission_map"]["path"]).read_bytes() == src


def test_gltf_round_trip(tmp_path):
    path = _image(tmp_path / "glow.png", (250, 200, 0))
    scene = Scene()
    _triangle(scene, Material("Neon", (.3, .3, .3), pbr={
        "roughness": .3, "emission": [.9, .2, .1], "emission_strength": 6.0, "emission_map": {"path": path}}))
    save_glb(scene, tmp_path / "n.glb")
    js, mat = _gltf_material(tmp_path / "n.glb")
    assert mat["emissiveFactor"] == pytest.approx([.9, .2, .1])
    assert "emissiveTexture" in mat
    assert mat["extensions"]["KHR_materials_emissive_strength"]["emissiveStrength"] == 6.0
    assert "KHR_materials_emissive_strength" in js["extensionsUsed"]
    restored = Scene()
    load_glb(restored, tmp_path / "n.glb")
    pbr = restored.materials["Neon"].pbr
    assert pbr["emission"] == pytest.approx([.9, .2, .1]) and pbr["emission_strength"] == 6.0
    assert Path(pbr["emission_map"]["path"]).is_file()


def test_strength_one_needs_no_extension(tmp_path):
    scene = Scene()
    _triangle(scene, Material("Dim", (.3, .3, .3), pbr={"emission": [.5, .5, .5], "emission_strength": 1.0}))
    save_glb(scene, tmp_path / "d.glb")
    js, mat = _gltf_material(tmp_path / "d.glb")
    assert mat["emissiveFactor"] == [.5, .5, .5] and "extensions" not in mat
    restored = Scene()
    load_glb(restored, tmp_path / "d.glb")
    assert restored.materials["Dim"].pbr["emission_strength"] == 1.0


def test_the_editor_shows_emission_with_a_map_slot():
    from built_core import material_inspector
    _, sections = material_inspector(Material("Plain", (.5, .5, .5)), 0)
    rows = {label: value for _, items in sections for label, value in items}
    assert rows["Emission"].endswith("#000000") and "\x1dcolor\x1d" in rows["Emission"]
    assert rows["Strength"].endswith("\x1d0")
    actions = [label for title, items in sections if title == "Actions" for _, label in items]
    assert "Emission map…" in actions
    # Emission alone does not switch the PBR surface on.
    _, sections = material_inspector(Material("Glow", (.5, .5, .5), pbr={
        "emission": [1, 0, 0], "emission_strength": 2.0, "emission_map": {"path": "/x/glow.png"}}), 0)
    rows = {label: value for _, items in sections for label, value in items}
    assert rows["Emission"].endswith("#ff0000") and rows["Strength"].endswith("\x1d2")
    assert rows["Emission map"] == "glow.png" and "Metallic" not in rows
    assert rows["PBR surface"].endswith("\x1d0")
