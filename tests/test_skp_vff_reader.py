# SPDX-License-Identifier: GPL-3.0-or-later
"""Checks at the VFF member and applied-document boundaries."""
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
from types import SimpleNamespace
import zipfile

import pytest

from built_core import skp_vff_model


DATA = Path(__file__).parent / "data"
PINS = DATA / "skp_import_reference"


@pytest.mark.parametrize("reference", sorted(PINS.glob("*.json")), ids=lambda p: p.stem)
def test_document_open_matches_pinned_import(reference, tmp_path):
    # The snapshot helper also makes generated group/tag IDs deterministic.
    source = DATA / reference.stem
    env = dict(os.environ, HOME=str(tmp_path), CFFIXED_USER_HOME=str(tmp_path),
               BUILT_SETTINGS_DIR=str(tmp_path / "settings"), QT_QPA_PLATFORM="offscreen")
    root = DATA.parents[1]
    result = subprocess.run(
        [sys.executable, str(root / "scripts/skp_fixture_snapshot.py"), "--worker", str(source)],
        cwd=root, env=dict(env, PYTHONPATH=str(root)), capture_output=True,
        check=True, text=True, timeout=60)
    actual = json.loads(result.stdout)
    expected = json.loads(reference.read_text())
    # A folder's generated UUID is independent of its persisted tree identity.
    for key in actual.get("layer_folder_uids", {}):
        actual["layer_folder_uids"][key] = "<generated>"
    from tests.platform_golden import matches
    assert matches(actual, expected)


def _repack(path, *, physical, zip64=False):
    source = DATA / "skp_instances_2021.skp"
    raw = source.read_bytes()
    prefix = raw[:raw.index(b"PK\x03\x04")]
    stream = io.BytesIO()
    if physical:
        stream.write(prefix)
    with zipfile.ZipFile(source) as old, zipfile.ZipFile(stream, "w") as new:
        for member in old.infolist():
            with new.open(member.filename, "w", force_zip64=zip64) as writer:
                writer.write(old.read(member))
    path.write_bytes((b"" if physical else prefix) + stream.getvalue())


@pytest.mark.parametrize("physical", [False, True])
@pytest.mark.parametrize("zip64", [False, True])
def test_modern_model_reads_both_member_offset_conventions(tmp_path, monkeypatch, physical, zip64):
    path = tmp_path / "cube.skp"
    _repack(path, physical=physical, zip64=zip64)
    monkeypatch.setitem(sys.modules, "openskp", None)
    model = skp_vff_model(path, SimpleNamespace())
    assert [ins.ref_idx for ins in model.root.instances] == [18, 18]
    assert len(model.definitions[18].faces) == 6
    from formats.skp import parse_skp
    assert len(parse_skp(path)["protos"][0]["instances"]) == 2


def test_damaged_member_fails_before_any_model_is_published(tmp_path):
    path = tmp_path / "bad.skp"
    _repack(path, physical=True)
    with zipfile.ZipFile(path) as source:
        offset = source.getinfo("model.dat").header_offset
    raw = bytearray(path.read_bytes())
    name, extra = struct.unpack_from("<HH", raw, offset + 26)
    raw[offset + 30 + name + extra] ^= 1
    path.write_bytes(raw)
    with pytest.raises(OSError, match=r"\.skp ZIP"):
        skp_vff_model(path, SimpleNamespace())


@pytest.mark.parametrize("relief", [False, True], ids=["flat", "faceted"])
def test_drape_detection_requires_shared_uv_seams(relief):
    from built_core import skp_mark_drapes
    import numpy as np

    # Eight triangles on a shared square grid. Raising the middle vertex
    # changes each incident face's projection plane, while plan UVs still meet.
    points = [(x + 5, y + 8, 2 if relief and x == y == 1 else 0)
              for y in range(3) for x in range(3)]
    vertices = {i: SimpleNamespace(x=x, y=y, z=z) for i, (x, y, z) in enumerate(points)}
    edges, faces = {}, {}
    for origin in (0, 1, 3, 4):
        for ring in ((origin, origin + 1, origin + 4), (origin, origin + 4, origin + 3)):
            loop = []
            for a, b in zip(ring, ring[1:] + ring[:1]):
                loop.append((len(edges), 1))
                edges[len(edges)] = SimpleNamespace(v1_id=a, v2_id=b)
            a, b, c = (np.asarray(points[i]) for i in ring)
            normal = np.cross(b - a, c - a)
            faces[len(faces)] = SimpleNamespace(
                loops=[loop], normal=tuple(normal / np.linalg.norm(normal)),
                material_id=1, uv_transform=(1, 0, 0, 0, 1, 0, 0, 0, 1))
    skp_mark_drapes(SimpleNamespace(vertices=vertices, edges=edges, faces=faces),
                    {1: {"texture": {"sw": .0254, "sh": .0254}}})
    assert all(getattr(face, "_projected", False) == relief for face in faces.values())
