# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""GeoTIFF orthomosaics as georeferenced reference images (Marco,
2026-09-14: «quiero importar una imagen tif de un ortomosaico»). The
reader uses NumPy and native JPEG tiles: the header's pixel →
UTM transform places the picture on the datum at its true size, the
pixels decode from every scheme the exporters use, and big mosaics are
reduced while streaming."""
from __future__ import annotations

from pathlib import Path

import numpy as np
import pytest

from georef.geotiff import (GeoTiffError, read_info, read_rgba,
                            unsupported_reason)
from tests.geotiff_fixture import write_geotiff

from tests.shell_isolation import restore_shell_patches  # noqa: F401
DATA = Path(__file__).parent / "data"


def _picture(h=40, w=56, seed=1):
    rng = np.random.default_rng(seed)
    rgba = rng.integers(0, 256, (h, w, 4), dtype=np.uint8)
    rgba[..., 3] = 255
    rgba[:5, :5, 3] = 0                     # a no-data corner
    return rgba


def test_header_places_the_raster_in_utm(tmp_path):
    p = tmp_path / "orto.tif"
    write_geotiff(p, _picture(), origin=(720106.4, 8289766.2), scale=(0.1, 0.1),
                  epsg=32718)
    info = read_info(p)
    assert (info.width, info.height, info.samples) == (56, 40, 4)
    assert info.georeferenced and info.zone == 18 and info.northern is False
    assert info.pixel_to_map(0, 0) == pytest.approx((720106.4, 8289766.2))
    assert info.pixel_to_map(56, 40) == pytest.approx((720112.0, 8289762.2))
    assert info.ground_size() == pytest.approx((5.6, 4.0))
    bl, br, tr_, tl = info.corners_geodetic()
    assert bl[0] < tl[0] and bl[1] == pytest.approx(tl[1], abs=1e-6)  # north is up
    assert br[1] > bl[1]                                               # east is right
    assert tl[0] == pytest.approx(-15.4598, abs=1e-3)


def test_pixel_is_point_shifts_half_a_pixel(tmp_path):
    p = tmp_path / "point.tif"
    write_geotiff(p, _picture(), origin=(1000.0, 2000.0), scale=(2.0, 2.0),
                  pixel_is_point=True)
    info = read_info(p)
    # The tie point (0.5, 0.5) → (1000, 2000) names the first pixel's
    # centre: its top-left corner is a metre up-left of it.
    assert info.pixel_to_map(0, 0) == pytest.approx((999.0, 2001.0))


@pytest.mark.parametrize("compression,predictor,tile", [
    (1, 1, 0), (8, 1, 0), (8, 2, 0), (32773, 1, 0), (5, 1, 0),
    (1, 1, 16), (8, 2, 16), (5, 2, 16),
])
def test_pixels_decode_exactly(tmp_path, compression, predictor, tile):
    rgba = _picture()
    p = tmp_path / "px.tif"
    write_geotiff(p, rgba, compression=compression, predictor=predictor, tile=tile)
    info = read_info(p)
    assert unsupported_reason(info) is None
    out, factor = read_rgba(info)
    assert factor == 1
    assert out.shape == rgba.shape
    assert np.array_equal(out, rgba)


def test_rgb_without_alpha_comes_back_opaque(tmp_path):
    rgba = _picture()
    p = tmp_path / "rgb.tif"
    write_geotiff(p, rgba, samples=3, compression=8)
    out, _ = read_rgba(read_info(p))
    assert np.array_equal(out[..., :3], rgba[..., :3])
    assert (out[..., 3] == 255).all()


def test_big_mosaics_reduce_while_streaming(tmp_path):
    rgba = _picture(h=70, w=100, seed=3)
    p = tmp_path / "big.tif"
    write_geotiff(p, rgba, compression=8, tile=32)
    info = read_info(p)
    seen = []
    out, factor = read_rgba(info, max_px=40, progress=lambda fr: seen.append(fr) or True)
    assert factor == 4                       # 100 px → 25 px fits under 40
    assert out.shape == (18, 25, 4)          # ceil(70/4), ceil(100/4)
    # A reduced pixel is the box average of its 4×4 block.
    block = rgba[4:8, 8:12].reshape(-1, 4).astype(int).sum(0) // 16
    assert np.array_equal(out[1, 2], block.astype(np.uint8))
    assert seen[-1] == pytest.approx(1.0)
    # Strips that don't divide by the factor carry rows across bands.
    p2 = tmp_path / "strips.tif"
    write_geotiff(p2, rgba, compression=1)   # 16-row strips, factor 4
    out2, _ = read_rgba(read_info(p2), max_px=40)
    assert np.array_equal(out2, out)
    # At a small display size one output pixel spans several 32-pixel tiles.
    coarse, factor = read_rgba(info, max_px=2)
    from_strips, strips_factor = read_rgba(read_info(p2), max_px=2)
    assert factor == strips_factor == 64
    assert np.array_equal(coarse, from_strips)


def test_cancel_and_unsupported_are_reported(tmp_path):
    p = tmp_path / "x.tif"
    write_geotiff(p, _picture(), compression=8, tile=16)
    info = read_info(p)
    with pytest.raises(GeoTiffError):
        read_rgba(info, progress=lambda fr: False)
    info.compression = 34712                 # JPEG 2000
    assert "compression" in unsupported_reason(info)
    with pytest.raises(GeoTiffError):
        read_info(tmp_path / "missing.png") if (tmp_path / "missing.png").write_bytes(b"PNG!") else None


def test_jpeg_in_tiff_tile_decodes_to_true_colours_and_alpha():
    """A tile of a real GDAL orthophoto (JPEG-in-TIFF, 4 samples): the fourth
    JPEG component stores coverage. The no-data fringe
    reads alpha 0 and the field its true colours (verified by eye against
    the whole mosaic — brown earth, not blue)."""
    info = read_info(DATA / "orto_jpeg_tile.tif")
    assert info.compression == 7 and info.samples == 4 and info.tiled
    assert info.zone == 18 and info.northern is False
    out, _ = read_rgba(info)
    assert out.shape == (256, 256, 4)
    alpha = out[..., 3]
    assert (alpha == 255).mean() > 0.3 and (alpha == 0).mean() > 0.3
    valid = out[alpha == 255][..., :3].astype(float)
    r, g, b = valid.mean(0)
    assert r > b                             # sunlit earth, not inverted sky
    assert 40 < valid.mean() < 220           # a picture, not black or white
    assert (out[alpha == 0][..., :3] < 40).mean() > 0.9   # no-data is black


def test_import_places_the_orthomosaic_on_the_datum(monkeypatch, tmp_path):
    """File ▸ Import ▸ Orthomosaic: the picture lands on the datum plane at
    its true size and place, locked, on the Images layer; a scene without
    a datum gets one at the picture's centre and the base map follows."""
    from app_shell.host import Host
    from core.image_plane import IMAGE_LAYER
    from views.filedialogs import file_dialogs

    src = DATA / "orto_jpeg_tile.tif"
    monkeypatch.setattr(file_dialogs, "getOpenFileName",
                        lambda *a, **k: (str(src), ""))
    monkeypatch.setattr("core.texture.texture_cache_root", lambda: tmp_path)
    host = Host(0, 900, 600, 1., settings=str(tmp_path / "settings"))
    win = host.window
    win.new_document()
    try:
        scene = win.viewport.scene
        assert scene.georef is None
        host.menu("import:orthomosaic")
        assert not host.errors
        assert len(scene.image_planes) == 1
        plane = scene.image_planes[0]
        info = read_info(src)
        w_m, h_m = info.ground_size()
        assert plane.width() == pytest.approx(w_m, rel=1e-3)     # 25.6 m
        assert plane.height() == pytest.approx(h_m, rel=1e-3)
        assert plane.locked and plane.layer == IMAGE_LAYER
        assert plane.origin.z() == 0.0
        # The datum sits at the picture's centre → the plane is centred.
        c = plane.center()
        assert abs(c.x()) < 0.05 and abs(c.y()) < 0.05
        assert scene.tile_layer is not None and scene.tile_layer.patches
        assert scene.georef is not None
        assert scene.georef.lat == pytest.approx(-15.4604, abs=1e-3)
        # u points east, v north (north angle 0).
        assert plane.u.x() > 0 and abs(plane.u.y()) < 0.05
        assert plane.v.y() > 0 and abs(plane.v.x()) < 0.05
        assert Path(plane.path).exists() and Path(plane.path).parent.parent == tmp_path
        # One undo step takes the picture (and its layer) back.
        assert win.viewport.history.undo()
        assert scene.image_planes == []
    finally:
        win._saved_version = win.viewport.scene.version
        host.close()


def test_failed_survey_cache_write_preserves_the_document(tmp_path, monkeypatch):
    from app_shell.host import Host
    from views.filedialogs import file_dialogs
    tile = tmp_path / "survey.tif"
    write_geotiff(tile, _picture())
    host = Host(0, 900, 600, 1., settings=str(tmp_path / "settings"))
    try:
        host.window.new_document()
        scene = host.viewport.scene
        messages = []
        def fail(*args):
            raise OSError("cache full")
        monkeypatch.setattr(file_dialogs, "getOpenFileName", lambda *args: (str(tile), ""))
        monkeypatch.setattr("core.texture.cache_image", fail)
        monkeypatch.setattr("views.chrome_dialogs.critical", lambda *args: messages.append(args[-1]))
        host.menu("import:orthomosaic")
        assert messages == ["cache full"] and not host.errors
        assert scene.georef is None and scene.tile_layer is None and not scene.image_planes
    finally:
        host.close()
