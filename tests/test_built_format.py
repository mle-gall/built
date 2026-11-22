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
    if _native.available() is None:
        pytest.skip("native store unavailable")
    scene = Scene()
    empty = np.empty(0, dtype=np.int64)
    src = dict(pos=POINTS, edges=[[0, 1]], flags=None, counts=empty,
               sizes=empty, rings=empty)
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
