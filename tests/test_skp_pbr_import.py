# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
""".skp 2025+ PBR materials come through a .skp import
(docs/skp-pbr-format.md).

``tests/data/skp_pbr_2025.skp`` was written with the .skp C SDK
(scripts/skp_pbr_fixture), and ``skp_pbr_2025.truth.json`` is what the SDK
reads back from it: the factors, flags and each map's decoded pixels. The
import must land on the same values, in the ``Material.pbr`` contract."""
from __future__ import annotations

import hashlib
import io
import json
import re
import zipfile
from pathlib import Path
from types import SimpleNamespace as NS

import pytest

from formats import openskp_compat
from formats import skp_openskp

DATA = Path(__file__).parent / "data"
FIXTURE = DATA / "skp_pbr_2025.skp"
TRUTH = json.loads((DATA / "skp_pbr_2025.truth.json").read_text())

pytest.importorskip("openskp")


def _pixels(path) -> str:
    from PySide6.QtGui import QImage
    img = QImage(str(path)).convertToFormat(QImage.Format_RGBA8888)
    assert not img.isNull(), path
    return hashlib.sha256(
        bytes(img.constBits())[:img.sizeInBytes()]).hexdigest()


@pytest.fixture
def cache(tmp_path, monkeypatch):
    root = tmp_path / "cache"
    monkeypatch.setenv("INGETRAZO_TEXTURE_CACHE", str(root))
    return root


def _materials(path):
    payload = skp_openskp.parse(path)
    return payload, {m["name"]: m for m in payload["materials"]}


def _rewrite(src: Path, dst: Path, edit) -> Path:
    """Copy the .skp ``src`` to ``dst`` with its ZIP entries passed through
    ``edit(name, data) -> data | None`` (None drops the entry), plus any
    extra entries ``edit.extra`` names. The VFF header is kept as is."""
    raw = src.read_bytes()
    head = raw[:raw.find(b"PK\x03\x04")]
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(raw[len(head):])) as zin, \
            zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zout:
        for info in zin.infolist():
            data = edit(info.filename, zin.read(info.filename))
            if data is not None:
                zout.writestr(info.filename, data)
        for name, data in getattr(edit, "extra", {}).items():
            zout.writestr(name, data)
    dst.write_bytes(head + out.getvalue())
    return dst


def _expected(t: dict):
    """The contract dict the SDK's view of a material maps to (paths
    aside), or None for a classic one."""
    if t["workflow"] != 1:
        return None
    out = {}
    tex = t["textures"]
    if t["metalness_enabled"]:
        out["metallic"] = t["metallic_factor"]
        if "metallic" in tex:
            out["metallic_map"] = tex["metallic"]
    if t["roughness_enabled"]:
        out["roughness"] = t["roughness_factor"]
        if "roughness" in tex:
            out["roughness_map"] = tex["roughness"]
    if t["normal_enabled"] and "normal" in tex:
        out["normal_scale"] = t["normal_scale"]
        out["normal_map"] = tex["normal"]
    if t["ao_enabled"] and "ao" in tex:
        out["ao_strength"] = t["ao_strength"]
        out["ao_map"] = tex["ao"]
    return out or None


def test_fixture_covers_the_three_pbr_shapes():
    # Guard the ground truth itself: every map, factors only, normal-only
    # DirectX, plus a classic control.
    assert TRUTH["PBR All Maps"]["normal_style"] == 0          # OpenGL
    assert set(TRUTH["PBR All Maps"]["textures"]) == {
        "base", "metallic", "roughness", "normal", "ao"}
    assert TRUTH["PBR Factors Only"]["textures"] == {}
    assert TRUTH["PBR Normal DirectX"]["normal_style"] == 1    # DirectX
    assert TRUTH["Classic Plain"]["workflow"] == 0


def test_factors_and_channels_match_the_sdk(cache):
    _payload, mats = _materials(FIXTURE)
    assert set(mats) == set(TRUTH)
    for name, t in TRUTH.items():
        want = _expected(t)
        got = mats[name].get("pbr")
        if want is None:
            assert got is None, name
            continue
        assert set(got) == set(want), name
        for key, value in want.items():
            if key.endswith("_map"):
                continue
            assert got[key] == pytest.approx(value, abs=1e-6), (name, key)


def test_maps_are_extracted_pixel_identical_beside_the_base_texture(cache):
    _payload, mats = _materials(FIXTURE)
    base = Path(mats["PBR All Maps"]["texture"]["path"])
    assert _pixels(base) == TRUTH["PBR All Maps"]["textures"]["base"][
        "pixels_sha256"]
    for name, t in TRUTH.items():
        pbr = mats[name].get("pbr") or {}
        for key, tex in t["textures"].items():
            if key == "base":
                continue
            entry = pbr[f"{key}_map"]
            path = Path(entry["path"])
            assert path.parent == base.parent          # the .skp's own cache
            assert path.parent.parent == cache / "skp"
            assert path.name == tex["file_name"]
            assert _pixels(path) == tex["pixels_sha256"], (name, key)


def test_normal_map_style_sets_flip_y_only_for_directx(cache):
    _payload, mats = _materials(FIXTURE)
    assert "flip_y" not in mats["PBR All Maps"]["pbr"]["normal_map"]
    assert mats["PBR Normal DirectX"]["pbr"]["normal_map"]["flip_y"] is True


def test_painted_faces_carry_the_pbr_recipe(cache):
    payload, mats = _materials(FIXTURE)
    seen = {}
    for group in payload["groups"]:
        for _outer, _holes, attrs in group["faces"]:
            if attrs and "mat" in attrs:
                seen[attrs["mat"]] = attrs
    assert set(seen) == set(TRUTH)
    for name in TRUTH:
        assert seen[name].get("pbr") == mats[name].get("pbr"), name
    assert "pbr" not in seen["Classic Plain"]


def test_reimport_reuses_the_extracted_maps(cache):
    _materials(FIXTURE)
    files = sorted(p.name for p in cache.rglob("*") if p.is_file())
    _materials(FIXTURE)
    assert sorted(p.name for p in cache.rglob("*") if p.is_file()) == files
    assert len(files) == 6        # 1 base + 4 maps + 1 DirectX normal


def _invert_roughness(name, data):
    # The .skp 2026 writer adds <mat:roughness_texture_invert> to every
    # pbrMR block and a thumbnail.jpg to every material folder.
    if name == "materials/PBR All Maps/material.xml":
        data = data.replace(
            b"<mat:enable_normal>",
            b"<mat:roughness_texture_invert>1</mat:roughness_texture_invert>"
            b"\n      <mat:enable_normal>")
    return data


_invert_roughness.extra = {
    "materials/PBR All Maps/thumbnail.jpg": b"\xff\xd8not-really-a-jpeg"}


def test_2026_glossiness_flag_writes_an_inverted_roughness_map(
        cache, tmp_path):
    from PySide6.QtGui import QImage
    skp = _rewrite(FIXTURE, tmp_path / "pbr_2026_style.skp", _invert_roughness)
    _payload, mats = _materials(skp)
    pbr = mats["PBR All Maps"]["pbr"]
    path = Path(pbr["roughness_map"]["path"])
    assert path.name == "all_rough_inverted.png"
    with zipfile.ZipFile(FIXTURE) as zf:
        src = QImage.fromData(zf.read("materials/PBR All Maps/all_rough.png"))
    inv = QImage(str(path))
    assert (inv.width(), inv.height()) == (src.width(), src.height())
    for x, y in ((0, 0), (5, 3), (15, 15)):
        assert inv.pixelColor(x, y).red() == 255 - src.pixelColor(x, y).red()
    # The thumbnail never stands in for the base image.
    assert _pixels(mats["PBR All Maps"]["texture"]["path"]) == TRUTH[
        "PBR All Maps"]["textures"]["base"]["pixels_sha256"]
    assert pbr["metallic"] == pytest.approx(0.25)


def test_a_pbr_material_with_every_channel_off_gets_no_recipe(cache, tmp_path):
    # 2025+ files mark many materials workflow="1" with nothing enabled; an
    # empty recipe would render with glTF defaults (fully metallic).
    def all_off(name, data):
        if name == "materials/PBR All Maps/material.xml":
            data = re.sub(rb"(<mat:enable_\w+>)1(</mat:enable_\w+>)",
                          rb"\g<1>0\g<2>", data)
        return data

    skp = _rewrite(FIXTURE, tmp_path / "off.skp", all_off)
    _payload, mats = _materials(skp)
    assert "pbr" not in mats["PBR All Maps"]
    assert "texture" in mats["PBR All Maps"]
    assert mats["PBR Factors Only"]["pbr"]["metallic"] == pytest.approx(0.9)


def test_a_map_missing_from_the_container_drops_only_that_map(cache, tmp_path):
    skp = _rewrite(FIXTURE, tmp_path / "missing.skp", lambda n, d: None
                   if n == "materials/PBR All Maps/all_normal_gl.png" else d)
    _payload, mats = _materials(skp)
    pbr = mats["PBR All Maps"]["pbr"]
    assert "normal_map" not in pbr and "normal_scale" not in pbr
    assert {"metallic_map", "roughness_map", "ao_map"} <= set(pbr)


@pytest.mark.parametrize("name", ["skp_arcs_2021.skp",
                                  "skp_instances_2021.skp"])
def test_classic_files_import_unchanged(cache, name):
    # These 2025-written files carry a pbrMR block on workflow="0"
    # materials: still no recipe.
    payload = skp_openskp.parse(DATA / name)
    assert all("pbr" not in m for m in payload.get("materials", []))


def test_an_upstream_material_pbr_is_used_as_is(tmp_path, monkeypatch):
    # Once OpenSKP's Material carries `pbr` itself, the importer reads that
    # (same field names as openskp_compat.read_pbr).
    monkeypatch.setenv("INGETRAZO_TEXTURE_CACHE", str(tmp_path / "c"))
    rough = NS(filename="r.png", data=b"\x89PNGrough", width=1.0, height=1.0)
    pbr = NS(workflow=1, metalness_enabled=True, metallic_factor=1.5,
             metallic_texture=None, roughness_enabled=True,
             roughness_factor=0.3, roughness_texture=rough,
             roughness_texture_inverted=False, normal_enabled=False,
             normal_texture=None, normal_scale=1.0, normal_style=0,
             ao_enabled=False, ao_texture=None, ao_strength=1.0)
    mat = NS(name="Steel", color=(90, 90, 90, 255), transparency=1.0,
             texture=None, pbr=pbr)
    model = NS(materials=[mat], materials_by_id={7: mat})
    skp = tmp_path / "m.skp"
    skp.write_bytes(b"")
    attrs, registry = skp_openskp._material_attrs(model, skp)
    got = attrs[7]["pbr"]
    assert got["metallic"] == 1.0                      # clamped to 0..1
    assert got["roughness"] == pytest.approx(0.3)
    assert Path(got["roughness_map"]["path"]).read_bytes() == b"\x89PNGrough"
    assert registry[0]["pbr"] == got
    # attach_pbr leaves a material that already has one alone.
    assert openskp_compat.attach_pbr(model) == 0 and mat.pbr is pbr
