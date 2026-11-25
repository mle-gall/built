# SPDX-License-Identifier: GPL-3.0-or-later
"""Large documents are written compact and deflated fast (#158); small
ones stay indented. Either way the document reads back the same."""
from __future__ import annotations

from tests.raster_samples import solid_raster
import json
import zipfile

from core.scene import Scene
from formats import igz


def _doc(tmp_path, n_faces: int, texture=None):
    faces = []
    for i in range(n_faces):
        f = {"vertices": [[i, 0, 0], [i + 1, 0, 0], [i + 1, 1, 0]]}
        if texture:
            f["texture"] = {"path": str(texture), "sw": 1.0, "sh": 1.0}
        faces.append(f)
    src = tmp_path / f"src{n_faces}.igz"
    src.write_text(json.dumps({"igz_format": 1,
                               "scene": {"edges": [], "faces": faces}}))
    scene = Scene()
    igz.load_into(scene, src)
    return scene


def _shape(scene):
    return sorted(
        tuple(sorted((round(v.x(), 6), round(v.y(), 6), round(v.z(), 6))
                     for v in f.vertices))
        for f in scene.mesh.faces)


def _text(path) -> str:
    raw = path.read_bytes()
    if raw.startswith(b"PK"):
        with zipfile.ZipFile(path) as zf:
            return zf.read("document.json").decode("utf-8")
    return raw.decode("utf-8")


def test_a_small_document_stays_indented(tmp_path):
    scene = _doc(tmp_path, 3)
    out = tmp_path / "small.igz"
    igz.save_scene(scene, out)
    assert "\n  " in _text(out)


def test_a_large_document_is_compact_and_reads_back_the_same(
        tmp_path, monkeypatch):
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 10)
    scene = _doc(tmp_path, 30)
    out = tmp_path / "large.igz"
    igz.save_scene(scene, out)
    text = _text(out)
    assert "\n" not in text and ", " not in text
    again = Scene()
    igz.load_into(again, out)
    assert _shape(again) == _shape(scene)


def test_a_large_textured_container_reads_back_the_same(
        tmp_path, monkeypatch):
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 10)
    monkeypatch.setenv("BUILT_TEXTURE_CACHE", str(tmp_path / "cache"))
    png = tmp_path / "t.png"
    img = solid_raster(4, 4, (54, 137, 230))
    img.save(str(png))
    scene = _doc(tmp_path, 30, texture=png)
    out = tmp_path / "large-tex.igz"
    assert igz.save_scene(scene, out)["embedded"] == 1
    assert out.read_bytes().startswith(b"PK")
    assert "\n" not in _text(out)
    again = Scene()
    igz.load_into(again, out)
    assert _shape(again) == _shape(scene)


def test_the_count_includes_the_loose_mesh_and_each_prototype_once():
    from core.group import Group
    from core.mesh import Mesh
    from built_core import Matrix4 as QMatrix4x4, Vec3 as V
    scene = Scene()
    scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])   # 1 + 3
    proto = Mesh()
    proto.add_face([V(0, 0, 0), V(1, 0, 0), V(0, 1, 0)])        # 1 + 3
    for _ in range(3):                    # placed three times, written once
        g = Group(proto)
        g.xform = QMatrix4x4()
        scene.groups.append(g)
    assert igz._live_element_count(scene) == 4 + 4


def test_native_format_choice_uses_known_face_count_before_building_edges(tmp_path, monkeypatch):
    import pytest
    from core.mesh import DeferredMesh

    if not hasattr(igz.native_io(), "igz_large_document"):
        pytest.skip("native format classifier unavailable")
    scene = _doc(tmp_path, 5)
    if type(scene.mesh) is not DeferredMesh or scene._root is None:
        pytest.skip("requires deferred geometry in the Rust document")
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 4)
    assert igz._large_document(scene)
    assert "_deferred_plan" not in vars(scene.mesh)
    exact = igz._live_element_count(scene)
    for limit in (exact - 1, exact, exact + 1):
        monkeypatch.setattr(igz, "_LARGE_DOCUMENT", limit)
        assert igz._large_document(scene) == (exact > limit)
