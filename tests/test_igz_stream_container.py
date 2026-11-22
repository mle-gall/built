# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Direct-to-file containers keep the deterministic ZIP and atomic save contract."""
import io
import zipfile

import pytest

from formats import igz


@pytest.mark.parametrize("fast", [False, True])
def test_direct_container_matches_buffered_zip_bytes(tmp_path, fast):
    doc = '{"name":"Español", "faces":[]}'
    geometry = {"geom/z.bin": bytes(range(256)) * 100, "geom/a.bin": b"zeros" * 1000}
    textures = {"textures/z.png": b"PNG data", "textures/a.jpg": b"JPEG data"}
    expected = io.BytesIO()
    with zipfile.ZipFile(expected, "w") as archive:
        for name, data, compression in (
            [(igz._DOC_ENTRY, doc, zipfile.ZIP_DEFLATED)]
            + [(key, geometry[key], zipfile.ZIP_DEFLATED) for key in sorted(geometry)]
            + [(key, textures[key], zipfile.ZIP_STORED) for key in sorted(textures)]
        ):
            info = zipfile.ZipInfo(name, date_time=igz._ZIP_EPOCH)
            info.compress_type = compression
            archive.writestr(info, data, compresslevel=1 if fast else None)
    path = tmp_path / "scene.igz"
    igz._write_container(path, doc, textures, fast=fast, deflated=geometry)
    assert path.read_bytes() == expected.getvalue()
    assert not list(tmp_path.glob("*.part"))


@pytest.mark.parametrize("failure", [OSError, RuntimeError, KeyboardInterrupt])
def test_failed_archive_leaves_existing_document_intact(tmp_path, monkeypatch, failure):
    path = tmp_path / "scene.igz"
    path.write_bytes(b"previous document")
    original = zipfile.ZipFile.writestr
    calls = []

    def fail_after_document(self, *args, **kwargs):
        calls.append(1)
        if len(calls) > 1:
            raise failure("write interrupted")
        return original(self, *args, **kwargs)

    monkeypatch.setattr(zipfile.ZipFile, "writestr", fail_after_document)
    with pytest.raises(failure, match="write interrupted"):
        igz._write_container(path, "{}", {"textures/image.png": b"image"})
    assert path.read_bytes() == b"previous document"
    assert not list(tmp_path.glob("*.part"))
