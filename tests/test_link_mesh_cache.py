"""Cache keys and archive reuse captured from the Python reader at 336938628."""
import io
import json
import zipfile
from pathlib import Path

import numpy as np
import pytest

from core.group import Group
from built_core.materials import Material
from built_core import Vec3
from built_core.link_meshes import LinkMeshes
from built_core.mesh import Mesh


def test_keys_include_only_the_material_renames_used_by_the_geometry():
    reader = LinkMeshes("fixtures/site.igz", {"Red": Material("Red", color=(1, 0, 0))})
    payload = {"groups": [{"children": [{"uid": uid, "proto": 4, "xform": []}
                                        for uid in ("z", "a")]}],
               "kept_components": [{"uid": "b", "proto": 4, "xform": []}]}
    reader.prepare({"Red": Material("Red", color=(0, 0, 1))}, payload)
    faces = [{"mat": "Red", "back": {"mat": "Other"}}]
    blocks = [
        {"edges": [], "faces": faces},
        {"_arrays": {"xyz": np.arange(24, dtype="<f8").reshape(4, 6)[:, ::2],
                     "empty": np.empty((0, 3), dtype=np.uint32), "meta": {"face_attrs": faces}}},
        {"geom": {"member": "models/geo.bin", "face_attrs": faces}},
    ]
    hashes = json.loads((Path(__file__).parent / "fixtures/link_mesh_digests.json").read_text())
    for block, expected in zip(blocks, hashes, strict=True):
        path, renames, placement, digest = reader.key(block, {"models/geo.bin": b"geometry"}, ("proto", 4))
        assert (path, renames, placement) == ("fixtures/site.igz", (("Red", "Red (2)"),), ("proto", "a"))
        assert digest.hex() == expected


@pytest.mark.parametrize("method", [zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED])
def test_archive_reuse_requires_every_occurrence_to_use_the_cached_mesh(method):
    contents = io.BytesIO()
    with zipfile.ZipFile(contents, "w", compression=method) as out:
        out.writestr("geometry.bin", b"geometry" * 123)
    block = {"geom": {"member": "geometry.bin"}}
    previous = Group(Mesh(), name="Cache")
    with zipfile.ZipFile(contents) as archive:
        first = LinkMeshes("unit.igz", {})
        first.prepare({}, block)
        assert not first.unchanged_members(archive, block)
        mesh = previous.mesh
        mesh.__dict__["_link_geometry"] = first.key(block, {}, ("loose",)), mesh.revision
        for filtered, extra, expected in [
                (False, {}, {"geometry.bin"}), (True, {}, set()),
                (False, {"groups": [dict(block, uid="new")]}, set())]:
            reader = LinkMeshes("unit.igz", {}, previous, filtered=filtered)
            payload = dict(block, **extra)
            reader.prepare({}, payload)
            assert reader.unchanged_members(archive, payload) == expected

        mesh.add_edge(Vec3(), Vec3(1, 0, 0))
        edited = LinkMeshes("unit.igz", {}, previous)
        edited.prepare({}, block)
        assert not edited.unchanged_members(archive, block)
