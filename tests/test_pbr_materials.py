"""PBR recipes at document, glTF and renderer boundaries."""
from tests.raster_samples import solid_raster
from dataclasses import replace
from pathlib import Path
import zipfile

import numpy as np
from tests.shell_isolation import restore_shell_patches  # noqa: F401
import pytest
from built_render import SurveyRaster
from built_core import Vec3 as V

from core.group import Group
from core.history import RestampMaterialCommand
from core.materials import Material, register
from core.scene import Scene
from formats import igz
from formats.glb import load_glb, parse_container
from formats.gltf import save_glb
from built_core import source_arrays
from built_render import material_texture_array


def triangle(mesh):
    return mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])


def image(path, rgb):
    tile = solid_raster(2, 3, rgb)
    assert tile.save(str(path))
    return str(path)


def test_igz_embeds_pbr_in_registry_faces_and_nested_paint(tmp_path, monkeypatch):
    monkeypatch.setenv("BUILT_TEXTURE_CACHE", str(tmp_path / "cache"))
    path = image(tmp_path / "tile.png", (15, 29, 41))
    src = Path(path).read_bytes()
    recipe = {key: {"path": path} for key in
              ("roughness_map", "normal_map", "metallic_map", "orm_map", "ao_map")}
    recipe["normal_map"]["flip_y"] = True
    mat = Material("Tile", pbr=recipe)
    scene = Scene()
    register(scene.materials, mat)
    assert register(scene.materials, replace(mat, pbr={"metallic": 1.})) == "Tile (2)"
    group = Group(name="Nested")
    group.material = mat.face_attrs()
    group.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])
    scene.groups.append(group)
    face = triangle(scene.loose_mesh)
    face.attrs.update(mat.face_attrs())
    face.attrs["back"] = mat.face_attrs()
    doc = tmp_path / "test.igz"
    igz.save_scene(scene, doc)
    with zipfile.ZipFile(doc) as z:
        assert len([n for n in z.namelist() if n.startswith("textures/")]) == 1
    assert face.attrs["pbr"] == group.material["pbr"] == recipe
    Path(path).unlink()
    restored = Scene()
    igz.load_into(restored, doc)
    front = restored.loose_mesh.faces[0].attrs
    for pbr in (front["pbr"], front["back"]["pbr"], restored.groups[0].material["pbr"],
                restored.materials["Tile"].pbr):
        assert pbr["normal_map"]["flip_y"]
        for key in recipe:
            assert Path(pbr[key]["path"]).read_bytes() == src


@pytest.mark.parametrize("packed", [True, False])
def test_glb_round_trip_channels_uv_and_factors(tmp_path, packed):
    paths = {key: image(tmp_path / (key + ".png"), rgb) for key, rgb in (
        ("normal_map", (110, 37, 244)), ("roughness_map", (50, 7, 9)),
        ("metallic_map", (202, 33, 19)), ("ao_map", (78, 11, 0)),
        ("orm_map", (78, 50, 202)))}
    recipe = {k: {"path": paths[k]} for k in
              (("orm_map",) if packed else ("roughness_map", "metallic_map", "ao_map"))}
    recipe.update(normal_map={"path": paths["normal_map"], "flip_y": True},
                  normal_scale=.52, metallic=.75, roughness=.42, ao_strength=.66)
    scene = Scene()
    face = triangle(scene.loose_mesh)
    face.attrs.update(Material("Alloy", (.5, .4, .3), pbr=recipe).face_attrs())
    source = tmp_path / "source.glb"
    save_glb(scene, source)
    restored = Scene()
    load_glb(restored, source)
    actual = restored.materials["Alloy"].pbr
    assert SurveyRaster.open(actual["normal_map"]["path"]).rgba()[1] == 218
    assert actual["normal_scale"] == .52
    assert restored.groups[0].mesh.faces[0].attrs["pbr"] == actual
    out = tmp_path / "again.glb"
    save_glb(restored, out)
    js, buffers = parse_container(out)
    mat = js["materials"][0]
    mr = mat["pbrMetallicRoughness"]
    assert mr["baseColorFactor"] == [.5, .4, .3, 1.]
    assert (mr["metallicFactor"], mr["roughnessFactor"]) == (.75, .42)
    assert mat["occlusionTexture"]["strength"] == .66
    assert mat["normalTexture"]["scale"] == .52
    assert "TEXCOORD_0" in js["meshes"][0]["primitives"][0]["attributes"]
    def pixel(texture):
        idx = js["textures"][texture["index"]]["source"]
        view = js["bufferViews"][js["images"][idx]["bufferView"]]
        start = view["byteOffset"]
        return SurveyRaster.decode(buffers[0][start:start+view["byteLength"]]).rgba()[:4]
    orm = pixel(mr["metallicRoughnessTexture"])
    assert (orm[1], orm[2]) == (50, 202)
    assert pixel(mat["occlusionTexture"])[0] == 78


def test_restamp_keeps_uv_and_legacy_corner_bytes(tmp_path):
    scene = Scene()
    face = triangle(scene.loose_mesh)
    tex = {"path": image(tmp_path / "base.png", (18, 62, 111))}
    mat = Material("Paint", texture=tex)
    scene.materials[mat.name] = mat
    face.attrs.update(mat.face_attrs())
    face.attrs["texture"] = {**tex, "uvw": [2, 0, 0, .25, 0, 3, 0, .75]}
    def pack():
        ids = {}
        *_, maps, paints = source_arrays(scene.loose_mesh, texture_ids=ids)
        return maps.tobytes(), paints.tobytes(), material_texture_array(ids)
    before = pack()
    assert before[0] == np.array([*face.attrs["texture"]["uvw"], *([0]*8)], '<f4').tobytes()
    assert before[1] == np.array([0, 1, 0], '<u4').tobytes()
    assert len(before[2]) == 4
    cmd = RestampMaterialCommand("Paint", replace(mat, pbr={"normal_map": tex}))
    cmd.do(scene)
    maps, _, textures = pack()
    assert maps == before[0]
    assert textures[2] == 2  # white + one image shared by base and normal
    cmd.undo(scene)
    assert pack() == before


def test_cross_document_paste_registers_recipe_and_undoes_collision(tmp_path):
    from formats.clip_payload import encode, decode
    from tests.test_paste import _VP, _paste_at
    from core.history import History
    from tests.controller_surface import ControllerSurface as Viewport
    first = Scene()
    mat = Material("Metal", pbr={"normal_map": {"path": image(tmp_path / "n.png", (128, 128, 255))}})
    first.materials[mat.name] = mat
    f = triangle(first.mesh)
    f.attrs.update(mat.face_attrs())
    first.selection.add(f)
    source = _VP(first, History(first), None)
    assert Viewport.copy_selection(source, publish=False)
    clip = decode(encode(source.clipboard))
    second = Scene()
    second.materials["Metal"] = Material("Metal")
    target = _VP(second, History(second), clip)
    _paste_at(target, 5, 0)
    attrs = second.mesh.faces[0].attrs
    assert attrs["mat"] == "Metal (2)"
    assert attrs["pbr"] == second.materials[attrs["mat"]].pbr
    assert target.history.undo()
    assert set(second.materials) == {"Metal"}


def test_linked_recipe_on_nested_group_with_name_collision(tmp_path):
    from core import links
    source = Scene()
    mat = Material("Steel", pbr={"metallic": .9})
    source.materials[mat.name] = mat
    nested = Group(name="nested")
    nested.material = mat.face_attrs()
    triangle(nested.mesh)
    root = Group(name="assembly")
    root.adopt([nested])
    source.groups = [root]
    path = tmp_path / "link.igz"
    igz.save_scene(source, path)
    host = Scene()
    host.materials["Steel"] = Material("Steel", pbr={"roughness": .7})
    imported = links.make_link(host, path)
    paint = imported.children[0].children[0].material
    assert paint["mat"] == "Steel (2)"
    assert paint["pbr"] == host.materials[paint["mat"]].pbr == mat.pbr


def test_edit_pbr_in_the_inspector_and_undo_each_change(native_host, tmp_path, monkeypatch):
    from built_render import wgpu_library as library
    win = native_host.window
    scene = win.viewport.scene
    mat = Material("Editor alloy", (.5, .5, .5))
    scene.materials[mat.name] = mat
    face = triangle(scene.mesh)
    face.attrs.update(mat.face_attrs())
    path = image(tmp_path / "normal.png", (128, 128, 255))
    monkeypatch.setattr(library, "_pick_image", lambda *_: path)
    library.edit_field(win, win.view, "percent_metallic:" + mat.name, "77")
    library.action(win, win.view, "map:m:" + mat.name + ":normal_map")
    assert scene.materials[mat.name].pbr["metallic"] == .77
    assert face.attrs["pbr"]["normal_map"]["path"] == path
    assert win.viewport.history.undo() and "normal_map" not in face.attrs["pbr"]
    assert win.viewport.history.undo()
    assert "pbr" not in face.attrs and scene.materials[mat.name].pbr is None
