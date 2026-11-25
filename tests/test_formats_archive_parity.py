"""Archive compatibility across the three historical IGZ layouts."""
import hashlib
import itertools
import json
import uuid
import zipfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
CASES = (
    "tests/fixtures/links/five-parts.igz",
    "examples/banca-pergola-yanque.igz",
    "tests/fixtures/nested_rings_0_1.igz",
)


def archive_results(source, folder, monkeypatch):
    from core import group, layers
    from core.scene import Scene
    from formats import igz

    ids = itertools.count()
    guids = itertools.count()
    def uid():
        return f"archive-id-{next(ids):05d}"
    monkeypatch.setattr(group, "new_uid", uid)
    monkeypatch.setattr(layers, "new_uid", uid)
    monkeypatch.setattr(uuid, "uuid4", lambda: uuid.UUID(int=next(guids)))
    def content(path):
        if zipfile.is_zipfile(path):
            with zipfile.ZipFile(path) as archive:
                return {k: archive.read(k) for k in archive.namelist()}
        return {"document.json": path.read_bytes()}
    def digest(path):
        members = content(path)
        doc = json.loads(members.pop("document.json"))
        # Track scene content across writer versions.
        scene_bytes = json.dumps(doc["scene"], separators=(",", ":"), sort_keys=True).encode()
        return {"format": doc["igz_format"], "scene": hashlib.sha256(scene_bytes).hexdigest(),
                "members": {k: hashlib.sha256(v).hexdigest() for k,v in members.items()}}
    input_doc = json.loads(content(source)["document.json"])
    result = {"original_format": input_doc["igz_format"], "original_version": input_doc["app_version"]}
    for key in ("first_save", "reopened_save"):
        scene = Scene()
        igz.load_into(scene, source)
        source = folder / (key + ".igz")
        igz.save_scene(scene, source, backup=False)
        result[key] = digest(source)
    return result


@pytest.mark.parametrize("name", CASES)
def test_old_archive_round_trip(name, tmp_path, monkeypatch):
    expected = json.loads((ROOT / "tests/fixtures/formats_archive_python.json").read_text())
    assert archive_results(ROOT / name, tmp_path, monkeypatch) == expected[name]


@pytest.mark.parametrize("first", [
    "formats.igz", "ingetrazo_core.formats.igz", "formats.ifc",
    "georef.points", "ingetrazo_core.georef.points",
])
def test_plugin_can_import_each_namespace_first(first):
    import subprocess
    import sys

    probe = """
from importlib import import_module
import sys
import_module(sys.argv[1])
for name in ('formats.igz', 'georef.points'):
    assert import_module(name) is import_module('ingetrazo_core.' + name)
assert import_module('formats.ifc').__file__.endswith('__init__.py')
"""
    subprocess.run([sys.executable, "-c", probe, first], cwd=ROOT, check=True)
