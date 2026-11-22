"""Built v1 precision through the store, both archive readers and recovery."""
import json
import zipfile

import numpy as np
import pytest

from core import _native, store_mesh
from core.group import Group
from core.scene import Scene
from formats import built, igz
from formats.igz_snapshot import snapshot_steps

POINTS = np.array([[650000.01, 6860000.02, 0], [650000.02, 6860001.02, 0]])


def precise_scene():
    from unittest.mock import patch
    if _native.available() is None:
        pytest.skip("native store unavailable")
    scene = Scene()
    empty = np.empty(0, dtype=np.int64)
    src = dict(pos=POINTS, edges=[[0, 1]], flags=None, counts=empty,
               sizes=empty, rings=empty)
    with patch.object(_native, "native", _native.available()):
        store, table = store_mesh.store_from_source(src, precise=True)
    store_mesh.become_store(scene.mesh, store, table)
    return scene


def manifest(path):
    with zipfile.ZipFile(path) as archive:
        return json.loads(archive.read("document.json"))


@pytest.mark.parametrize("engine", [False, True])
def test_precise_scene(tmp_path, monkeypatch, engine):
    scene = precise_scene()
    matrix = np.eye(4).flatten()
    matrix[12:15] = POINTS[0]
    group = Group()
    group.xform = built.placement(matrix)
    scene.groups.append(group)
    path = tmp_path / "site.built"
    igz.save_scene(scene, path)
    monkeypatch.setattr(_native, "native", _native.available() if engine else None)
    monkeypatch.setattr(store_mesh, "enabled", lambda: engine)
    monkeypatch.setattr(igz, "_NATIVE_JSON_MIN", 0)
    data = manifest(path)
    assert data["built_format"] == 1 and "igz_format" not in data
    assert data["scene"]["geom"]["arrays"]["pos"][0] == "<f8"
    with zipfile.ZipFile(path) as z:
        members = igz._read_geometry_members(z, path, data["scene"], built_format=True)
        assert bool(members.checked) == engine
    restored = Scene()
    igz.load_into(restored, path)
    np.testing.assert_array_equal([built.position(v) for v in restored.mesh.vertices], POINTS)
    np.testing.assert_array_equal(built.placement_values(restored.groups[0].xform), matrix)
    # Recovery goes through a detached snapshot and a background serializer.
    steps = snapshot_steps(restored)
    with pytest.raises(StopIteration) as result:
        while True:
            next(steps)
    slot = tmp_path / "autosave.built"
    igz.write_prepared(result.value.value, slot, background=True)
    recovered = Scene()
    igz.load_into(recovered, slot)
    np.testing.assert_array_equal([built.position(v) for v in recovered.mesh.vertices], POINTS)


def test_export_warns_once_before_replacing(tmp_path):
    scene = precise_scene()
    path = tmp_path / "copy.igz"
    assert igz.export_igz(scene, path, lambda mm: False) is None
    assert not path.exists()
    warnings = []
    igz.export_igz(scene, path, lambda mm: warnings.append(mm) or True)
    rounded = POINTS.astype("f").astype("d")
    assert warnings == pytest.approx([np.linalg.norm(POINTS - rounded, axis=1).max() * 1000])
    data = manifest(path)
    assert data["igz_format"] == 3
    assert data["scene"]["geom"]["arrays"]["pos"][0] == "<f4"
    reopened = Scene()
    igz.load_into(reopened, path)
    np.testing.assert_array_equal([built.position(v) for v in reopened.mesh.vertices], rounded)


@pytest.mark.parametrize("marker", [{"igz_format": 3}, {"built_format": 2},
                                    {"built_format": 1, "igz_format": 3}])
def test_f64_is_scoped_to_built_v1(tmp_path, marker):
    path = tmp_path / "input.built"
    igz.save_scene(precise_scene(), path)
    with zipfile.ZipFile(path) as z:
        members = {name: z.read(name) for name in z.namelist()}
    data = json.loads(members["document.json"])
    del data["built_format"]
    data.update(marker)
    members["document.json"] = json.dumps(data)
    path = path.with_suffix(".igz")
    with zipfile.ZipFile(path, "w") as z:
        for name, value in members.items():
            z.writestr(name, value)
    with pytest.raises(ValueError):
        igz.load_into(Scene(), path)


def test_export_includes_placement_rounding_and_scale(tmp_path):
    scene = precise_scene()
    group = Group(scene.mesh)
    matrix = np.diag([2., 1., 1., 1.]).T.flatten()
    matrix[12] = 650000.01
    group.xform = built.placement(matrix)
    scene = Scene()
    scene.groups.append(group)
    warnings = []
    igz.export_igz(scene, tmp_path / "copy.igz", lambda mm: warnings.append(mm) or True)
    rounded = POINTS.astype("f").astype("d")
    delta = (POINTS - rounded) * [2, 1, 1]
    delta[:, 0] += matrix[12] - float(np.float32(matrix[12]))
    assert warnings == pytest.approx([np.linalg.norm(delta, axis=1).max() * 1000])


def test_python_reader_handles_distinct_points_in_one_float32_cell(tmp_path, monkeypatch):
    monkeypatch.setattr(_native, "native", None)
    monkeypatch.setenv("INGETRAZO_RUST", "0")
    points = np.array([[650000.01, 6860000.02, 0], [650000.02, 6860000.02, 0]], dtype="<f8")
    assert np.array_equal(*points.astype("<f4"))
    # A minimal independently constructed archive, usable without the engine.
    arrays = {
        "pos": ["<f8", [2, 3], 0], "edges": ["<i4", [1, 2], 48],
        "edge_flags": ["|u1", [1], 56],
        **{key: ["<i4", [0], 64] for key in ("rings", "ring_sizes", "ring_counts", "face_attr")},
    }
    path = tmp_path / "fixture.built"
    member = "geom/0.bin"
    data = {"built_format": 1, "scene": {"geom": {"member": member, "arrays": arrays}}}
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("document.json", json.dumps(data))
        archive.writestr(member, points.tobytes() + np.array([0, 1], dtype="<i4").tobytes() + bytes(8))
    scene = Scene()
    igz.load_into(scene, path)
    assert len(scene.mesh.edges) == 1
    np.testing.assert_array_equal([built.position(v) for v in scene.mesh.vertices], points)
    saved = tmp_path / "roundtrip.built"
    igz.save_scene(scene, saved)
    assert manifest(saved)["scene"]["geom"]["arrays"]["pos"][0] == "<f8"


def test_legacy_float32_weld_threshold_is_preserved(monkeypatch):
    mod = _native.available()
    if mod is None:
        pytest.skip("native store unavailable")
    monkeypatch.setattr(_native, "native", mod)
    # Squaring these binary32 components in binary64 puts the second point
    # just outside the weld radius. Original IGZ readers weld this edge shut.
    points = np.array([[0, 0, 0], [3.0003e-8, 1e-4, 0]], dtype="f")
    empty = np.empty(0, dtype=np.int64)
    source = dict(pos=points, edges=[[0, 1]], sizes=empty, counts=empty, rings=empty)
    legacy, _ = store_mesh.store_from_source(source)
    precise, _ = store_mesh.store_from_source(source, precise=True)
    assert legacy.counts() == (1, 0, 0)
    assert precise.counts() == (2, 1, 0)
