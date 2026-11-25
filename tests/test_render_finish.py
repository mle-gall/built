# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Finishes for Render with Blender (#181; Marco: «¿qué hay de los
materiales como transparencias y el agua?»). A material's finish — matte,
satin, gloss, metal, glass, water — is guessed from its name and picture
or chosen by hand, saved with the document, sent in the GLB as standard
PBR plus extras, and made into a real material inside Blender."""
from __future__ import annotations

import json
import struct
import subprocess
from pathlib import Path

from tests.shell_isolation import restore_shell_patches  # noqa: F401
import pytest
from ingetrazo_core import Vec3 as V

from core import finish as fin  # noqa: E402
from core import render_blender as rb  # noqa: E402
from core.history import History, SetMaterialFinishCommand  # noqa: E402
from core.materials import Material  # noqa: E402
from core.scene import Scene  # noqa: E402
from formats import igz  # noqa: E402
from formats.gltf import save_glb  # noqa: E402


@pytest.mark.parametrize("name, picture, expected", [
    # The names in Marco's own Yanque models.
    ("water_calm", None, "water"),
    ("[Water Pool Light]2", None, "water"),
    ("Metal_10_1K", None, "metal"),
    ("metal_dark", None, "metal"),
    ("Marble_07_1K", None, "gloss"),
    ("[Granite Light Gray]", None, "gloss"),
    ("[Wood Floor]", None, "satin"),
    ("Beech", "beech_wood.jpg", "satin"),
    ("Concrete_06_1K", None, "matte"),
    ("[Grass Dark Green]", None, "matte"),
    # Spanish and Portuguese.
    ("Vidrio templado", None, "glass"),
    ("Acero inoxidable", None, "metal"),
    ("Agua de piscina", None, "water"),
    ("Madeira", None, "satin"),
    ("mat3", "piscina_azul.png", "water"),
])
def test_the_finish_is_guessed_from_the_name(name, picture, expected):
    assert fin.guess(name, picture) == expected


def test_translucent_paint_reads_as_glass_and_a_choice_wins():
    assert fin.guess("mat7", None, opacity=0.35) == "glass"
    assert fin.resolve("gloss", "water_calm") == "gloss"
    assert fin.resolve(None, "water_calm") == "water"
    assert fin.resolve("nonsense", "Concrete") == "matte"


def test_choosing_a_finish_is_undoable_and_saved(tmp_path):
    scene = Scene()
    scene.materials["Porcelanato"] = Material("Porcelanato",
                                              color=(0.9, 0.9, 0.9))
    hist = History(scene)
    hist.execute(SetMaterialFinishCommand("Porcelanato", "metal"))
    assert scene.materials["Porcelanato"].finish == "metal"
    path = tmp_path / "doc.igz"
    igz.save_scene(scene, path)
    again = Scene()
    igz.load_into(again, path)
    assert again.materials["Porcelanato"].finish == "metal"
    hist.undo()
    assert scene.materials["Porcelanato"].finish is None


def _glb_json(path):
    data = path.read_bytes()
    n = struct.unpack_from("<I", data, 12)[0]
    return json.loads(data[20:20 + n])


def _scene_with(named):
    scene = Scene()
    for i, (name, attrs) in enumerate(named):
        f = scene.mesh.add_face([V(i * 2, 0, 0), V(i * 2 + 1, 0, 0),
                                 V(i * 2 + 1, 1, 0), V(i * 2, 1, 0)])
        f.attrs.update(attrs)
        f.attrs["mat"] = name
        scene.materials[name] = Material(name, color=attrs.get("color"),
                                         opacity=attrs.get("opacity"))
    return scene


def test_the_glb_carries_the_finish(tmp_path):
    scene = _scene_with([("water_calm", {"color": (0.1, 0.3, 0.8)}),
                         ("Vidrio", {"color": (0.8, 0.9, 1.0),
                                     "opacity": 0.3}),
                         ("Acero", {"color": (0.6, 0.6, 0.6)}),
                         ("Tarrajeo", {"color": (0.9, 0.9, 0.85)})])
    out = tmp_path / "m.glb"
    save_glb(scene, out)
    js = _glb_json(out)
    mats = {m["name"]: m for m in js["materials"]}
    assert mats["water_calm"]["extras"]["ingetrazo_finish"] == "water"
    assert mats["water_calm"]["extensions"]["KHR_materials_ior"]["ior"] == 1.33
    glass = mats["Vidrio"]
    assert glass["extras"]["ingetrazo_finish"] == "glass"
    assert glass["extensions"]["KHR_materials_transmission"][
        "transmissionFactor"] == 1.0
    assert mats["Acero"]["pbrMetallicRoughness"]["metallicFactor"] == 1.0
    assert mats["Tarrajeo"]["pbrMetallicRoughness"]["roughnessFactor"] == 0.9
    assert set(js["extensionsUsed"]) == {"KHR_materials_transmission",
                                         "KHR_materials_ior"}


@pytest.mark.parametrize("kind, needle", [
    ("windows", "winget install"),
    ("linux", "snap install blender"),
    ("flatpak", "flatpak install flathub org.blender.Blender"),
])
def test_each_package_says_how_to_get_blender(kind, needle):
    cmds = [c for _t, c in rb.install_steps(kind, host_allowed=False) if c]
    assert any(needle in c for c in cmds)


def test_the_flatpak_asks_for_the_permission_only_when_missing():
    with_perm = rb.install_steps("flatpak", host_allowed=True)
    without = rb.install_steps("flatpak", host_allowed=False)
    assert all(c != rb.FLATPAK_PERMISSION for _t, c in with_perm)
    assert any(c == rb.FLATPAK_PERMISSION for _t, c in without)
    assert all(c is None for _t, c in rb.install_steps("snap"))


@pytest.mark.slow
@pytest.mark.blender
def test_blender_turns_the_finishes_into_materials(tmp_path):
    from core.camera import OrbitCamera
    scene = _scene_with([("water_calm", {"color": (0.1, 0.3, 0.8)}),
                         ("Vidrio", {"color": (0.8, 0.9, 1.0),
                                     "opacity": 0.3}),
                         ("Acero", {"color": (0.6, 0.6, 0.6)})])
    cam = OrbitCamera()
    cam.set_aspect(160, 90)
    lo, hi = scene.bounds()
    cam.fit_to(lo, hi)
    job = rb.write_job(scene, cam, tmp_path, engine="eevee", quality=0,
                       width=160, height=90)
    run = subprocess.run(rb.command(rb.find_blender(), job),
                         capture_output=True, text=True, env=rb.clean_env(),
                         timeout=600)
    assert run.returncode == 0, run.stdout[-2000:]
    line = next(l for l in run.stdout.splitlines() if "finishes" in l)
    for word in ("glass 1", "metal 1", "water 1"):
        assert word in line
    assert Path(tmp_path / "render.png").is_file()


def test_the_material_inspector_sets_and_clears_the_finish(native_host):
    from ingetrazo_core import material_inspector
    from ingetrazo_render import wgpu_library
    win = native_host.window
    scene = win.viewport.scene
    name = "Mármol travertino"
    mat = scene.materials[name] = Material(name, color=(0.9, 0.85, 0.8))

    def picker():
        _, sections = material_inspector(scene.materials[name], 0)
        encoded = dict(dict(sections)["Look"])["Finish"]
        kind, field, data = encoded.lstrip("\x1d").split("\x1d")
        assert (kind, field) == ("picker", "libmat:finish:" + name)
        return json.loads(data)

    at, options = picker()
    assert [key for key, _ in options] == ["auto", *fin.FINISHES]
    assert options[at][0] == "auto" and fin.LABELS["gloss"] in options[at][1]
    before = win.viewport.history.revision
    wgpu_library.edit_field(win, win.view, "finish:" + name, "auto")
    with pytest.raises(ValueError, match="Unknown render finish"):
        wgpu_library.edit_field(win, win.view, "finish:" + name, "unknown")
    assert win.viewport.history.revision == before
    for key, expected in [("water", "water"), ("auto", None)]:
        wgpu_library.edit_field(win, win.view, "finish:" + name, key)
        assert scene.materials[name].finish == expected
        assert options[picker()[0]][0] == key
    assert win.viewport.history.undo() and scene.materials[name].finish == "water"
    assert win.viewport.history.undo() and scene.materials[name].finish is None
    assert fin.resolve(None, mat.name) == "gloss"
