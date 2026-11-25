# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Large saves retain the exact legacy JSON/ZIP bytes with bounded scratch reads."""
import ingetrazo_core
import io
import json
import tracemalloc
import zipfile

import pytest

from core.scene import Scene
from formats import igz
from ingetrazo_core import SaveStream
import tempfile
from tests.test_igz_format3 import _scene


def _old_container(path, doc, blobs, fast=False, deflated=None):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as archive:
        for member, data, compression in [
            (igz._DOC_ENTRY, doc, zipfile.ZIP_DEFLATED),
            *((m, deflated[m], zipfile.ZIP_DEFLATED) for m in sorted(deflated or ())),
            *((m, blobs[m], zipfile.ZIP_STORED) for m in sorted(blobs)),
        ]:
            info = zipfile.ZipInfo(member, date_time=igz._ZIP_EPOCH)
            info.compress_type = compression
            archive.writestr(info, data, compresslevel=1 if fast else None)
    path.write_bytes(buf.getvalue())


def test_streamed_save_matches_retained_document_and_round_trips(tmp_path, monkeypatch):
    from core import layers, links
    from core.composition import Composicion
    from core.materials import Material
    from core.saved_views import SavedView
    from tests.test_igz_format3 import _canonical

    scene = _scene(tmp_path)
    source = tmp_path / "source.igz"
    igz.save_scene(_scene(tmp_path), source)
    scene.groups.append(links.make_link(scene, source))
    folder = ingetrazo_core.tag_edits.add_folder(scene, "Finishes")
    scene.layers.append(layers.Layer("Wood", key="wood-key", folder=folder))
    scene.saved_views.append(SavedView("Plan", hidden_layers=["wood-key"]))
    scene.compositions.append(Composicion(name="Sheet 1"))
    scene.plugin_data["unicode"] = {"key": ["é", -0.0, 1e-20]}
    scene._home_thumbnail_png = b"thumbnail bytes"
    recipe = {"roughness_map": {"path": str(tmp_path / "wood.png")}}
    scene.mesh.faces[0].attrs.update(pbr=recipe, layer="wood-key")
    scene.materials["Finish"] = Material("Finish", pbr=recipe)
    before, after = tmp_path / "before.igz", tmp_path / "after.igz"
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    result = igz.save_scene(scene, after)

    # Retain all mesh blocks, then encode the whole payload and archive as trunk did.
    steps = igz.prepare_scene_steps(scene)
    with pytest.raises(StopIteration) as done:
        while True:
            next(steps)
    with monkeypatch.context() as m:
        m.setattr(igz, "_write_container", _old_container)
        assert igz.write_prepared(done.value.value, before) == result
    assert before.read_bytes() == after.read_bytes()
    loaded = Scene()
    igz.load_into(loaded, after)
    reference = Scene()
    igz.load_into(reference, before)
    assert _canonical(loaded) == _canonical(reference)
    assert loaded.groups[0].mesh is loaded.groups[1].mesh
    assert links.link_of(loaded.groups[-1])["rel"] == source.name
    assert loaded.layer("wood-key").folder == folder
    assert loaded.saved_views[-1].hidden_layers == ["wood-key"]
    assert loaded.compositions[-1].name == "Sheet 1"
    assert loaded.materials["Finish"].pbr == loaded.mesh.faces[0].attrs["pbr"]
    assert "path" in scene.mesh.faces[0].attrs["pbr"]["roughness_map"]


def test_json_fragments_match_stdlib_and_can_read_their_own_spool():
    leaf = {7: [None, True, False, -0.0, 1e-200, "é\\\"\n"], None: "null key"}
    with SaveStream() as stream:
        fragment = stream.json(leaf)
        actual = stream.document({"nested": [fragment, {"again": fragment}]})
        expected = {"nested": [leaf, {"again": leaf}]}
        assert b"".join(actual.chunks()) == json.dumps(
            expected, separators=(",", ":"), allow_nan=False).encode()


def test_many_completed_meshes_do_not_accumulate_encoded_bytes():
    tracemalloc.start()
    try:
        with SaveStream() as stream:
            fragments = [stream.json(["x" * 1024] * 1024) for _ in range(20)]
            assert sum(f.size for f in fragments) > 20_000_000
            _current, peak = tracemalloc.get_traced_memory()
            assert peak < 5_000_000
            assert all(len(chunk) <= 1024 * 1024
                       for chunk in fragments[0].chunks())
    finally:
        tracemalloc.stop()


@pytest.mark.parametrize("error", [OSError, RuntimeError, KeyboardInterrupt])
@pytest.mark.parametrize("native", [False, True])
def test_write_failure_preserves_old_file_and_closes_scratch(native, tmp_path, monkeypatch, error):
    monkeypatch.setenv("INGETRAZO_RUST_IO", "1" if native else "0")
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)
    path = tmp_path / "old.igz"
    path.write_bytes(b"original document")
    opened = []
    original = tempfile.TemporaryFile

    def temporary_file(*args, **kwargs):
        result = original(*args, **kwargs)
        opened.append(result)
        return result

    def fail(*args, **kwargs):
        raise error("injected write failure")

    monkeypatch.setattr(tempfile, "TemporaryFile", temporary_file)
    monkeypatch.setattr(zipfile._ZipWriteFile, "write", fail)
    if native and hasattr(igz.native_io(), "igz_write_zip"):
        original_writer = igz.native_io().igz_write_zip
        def interrupted(fd, members, fast):
            original_writer(fd, members[:1], fast)
            raise error("injected write failure")
        monkeypatch.setattr(igz.native_io(), "igz_write_zip", interrupted)
    with pytest.raises(error, match="injected"):
        igz.save_scene(_scene(tmp_path), path)
    assert path.read_bytes() == b"original document"
    assert not list(tmp_path.glob("*.part"))
    assert opened and all(f.closed for f in opened)


def test_missing_texture_is_counted_once_across_spooled_meshes(tmp_path, monkeypatch):
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    scene = _scene(tmp_path)
    for group in scene.groups:
        group.mesh.faces[0].attrs["texture"] = {"path": "/absent-r5c.png"}
    assert igz.save_scene(scene, tmp_path / "missing.igz") == {"embedded": 1, "missing": 1}


def test_nonfinite_fragment_preserves_existing_document(tmp_path, monkeypatch):
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    scene = _scene(tmp_path)
    scene.groups[-1].mesh.vertices[0].position.setX(float("nan"))
    path = tmp_path / "old.igz"
    path.write_bytes(b"original")
    with pytest.raises(igz.NonFiniteDocumentError):
        igz.save_scene(scene, path)
    assert path.read_bytes() == b"original"
    assert not list(tmp_path.glob("*.part"))
