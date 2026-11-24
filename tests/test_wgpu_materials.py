"""Material handoff tests require no window or GPU device."""
import sys

import numpy as np
import pytest
from views.wgpu_scene import _source_arrays, _texture_array
from PySide6.QtGui import QImage, QColor
from ingetrazo_core import Vec3 as QVector3D
from core.mesh import Mesh


def mesh_with(attrs):
    mesh = Mesh()
    mesh.add_face([QVector3D(0,0,0), QVector3D(2,0,0), QVector3D(0,3,0)])
    mesh.faces[0].attrs.update(attrs)
    return mesh


def test_front_paint_back_default_and_mirrored_paint():
    own = {'color': (0.4,0.1,0.05)}
    front = _source_arrays(mesh_with(own))
    np.testing.assert_array_equal(front[6][1], [102,25,12,255])
    assert front[10][0,0] == 0
    both = _source_arrays(mesh_with({**own, 'back': True}))
    assert both[10][0,0] == int.from_bytes(bytes([102,25,12,255]), 'little')


def test_separate_back_color_and_container_precedence():
    own = {'color': (0.4,0.1,0.05), 'back': {'color': (0,1,0)}}
    arrays = _source_arrays(mesh_with(own), {'color': (0,0,1)})
    np.testing.assert_array_equal(arrays[6][1], [102,25,12,255])
    assert arrays[10][0,0] == 0xff00ff00
    inherited = _source_arrays(mesh_with({}), {'color': (0,0,1)})
    assert inherited[10][0,0] == 0xffff0000


def test_layer_color_replaces_explicit_face_material():
    arrays = _source_arrays(mesh_with({'color': (1,0,0), 'layer': 'tag'}),
                            {'by_layer': {'tag': (0,1,0)}, 'color': (0,0,1)})
    np.testing.assert_array_equal(arrays[6][1], [0,255,0,255])
    assert arrays[10][0,0] == 0xff00ff00


def test_front_and_back_affines_preserve_leaf_grain():
    tex = {'path': 'unused.png', 'uvw': [2,0,0,.5,0,3,0,-1]}
    other = {**tex, 'uvw': [0,0,4,2,1,1,0,0]}
    ids = {}
    arrays = _source_arrays(mesh_with({'texture': tex, 'back': {'texture': other}}),
                            {'texture': {**tex,'uvw': [0]*8}}, ids)
    np.testing.assert_array_equal(arrays[9][0], tex['uvw']+other['uvw'])
    assert arrays[10][0,1:].tolist() == [1,1]
    assert ids == {'unused.png': 1}


def test_array_flips_v_and_supplies_mips(tmp_path):
    image = QImage(2,2,QImage.Format_RGBA8888)
    image.fill(QColor('red'))
    image.setPixelColor(0,1,QColor('blue'))
    path = tmp_path/'tile.png'
    assert image.save(str(path))
    w,h,layers,levels = _texture_array({str(path):1})
    assert (w,h,layers) == (2,2,2)
    assert [len(v) for v in levels] == [32,8]
    assert levels[0][16:20] == bytes([0,0,255,255])


def test_material_pixels_use_native_codecs_and_preserve_alpha(tmp_path, monkeypatch):
    from views import wgpu_scene
    path = tmp_path / "transparent.png"
    image = QImage(2, 1, QImage.Format_RGBA8888)
    image.setPixelColor(0, 0, QColor(255, 20, 0, 0))
    image.setPixelColor(1, 0, QColor(0, 20, 255, 255))
    assert image.save(str(path))
    monkeypatch.setattr(wgpu_scene, "_legacy_texture_pixels",
                        lambda p: pytest.fail(f"Unexpected legacy decode of {p}"))
    w, h, count, mips = _texture_array([path, tmp_path / "missing.png"])
    assert (w, h, count) == (2, 1, 3)
    assert mips[0][8:16] == bytes(image.constBits())
    assert mips[0][-8:] == b"\xff" * 8
    r, g, b, a = mips[1][4:8]
    assert r == 0 and abs(g - 20) <= 1 and b >= 254 and abs(a - 128) <= 1


def test_system_codec_fallback_uses_same_orientation_and_budget(tmp_path, monkeypatch):
    from views import wgpu_scene
    path = tmp_path / "texture.custom"
    path.write_bytes(b"synthetic system codec")
    called = []
    def decode(p):
        called.append(p)
        return 1, 2, bytes((1, 2, 3, 255, 4, 5, 6, 255))
    monkeypatch.setattr(wgpu_scene, "_legacy_texture_pixels", decode)
    w, h, count, mips = _texture_array([path])
    assert (w, h, count) == (1, 2, 2) and called == [path]
    assert mips[0][8:] == bytes((4, 5, 6, 255, 1, 2, 3, 255))


@pytest.mark.parametrize("ext", ["tiff", "svgz", "gif"])
def test_specialty_files_decode_once_for_materials_and_references(tmp_path, monkeypatch, ext):
    import gzip
    import struct
    from ingetrazo_render import reference_texture, raster_dimensions
    from views import wgpu_scene

    path = tmp_path / f"stripes.{ext}"
    # White over black. Generate real two-row files independently of Qt's codecs.
    if ext == "svgz":
        path.write_bytes(gzip.compress(b'<svg xmlns="http://www.w3.org/2000/svg" width="1" height="2">'
                                       b'<rect width="1" height="2"/><rect width="1" height="1" fill="white"/></svg>'))
    elif ext == "gif":
        # GIF89a, black/white global palette. LZW codes clear, white, black, end.
        path.write_bytes(bytes.fromhex("47494638396101000200800000000000ffffff"
                                       "2c00000000010002000002020c0a003b"))
    else:
        tags = [(256, 1), (257, 2), (258, 8), (259, 1), (262, 1),
                (273, 110), (278, 2), (279, 2)]
        ifd = b"".join(struct.pack("<HHII", tag, 4, 1, value) for tag, value in tags)
        path.write_bytes(b"II*\0" + struct.pack("<IH", 8, len(tags)) + ifd + bytes(4) + b"\xff\0")
    monkeypatch.setattr(wgpu_scene, "_legacy_texture_pixels",
                        lambda _: pytest.fail("Renderer called the Qt plugin"))
    assert raster_dimensions(path) == (1, 2)
    w, h, data = reference_texture(path, 16)
    assert (w, h) == (1, 2) and data.tolist() == [255, 255, 255, 255, 0, 0, 0, 255]
    w, h, count, mips = _texture_array([path])
    assert (w, h, count) == (1, 2, 2)
    assert mips[0][8:] == bytes((0, 0, 0, 255, 255, 255, 255, 255))


def test_reference_resizing_preserves_aspect_and_translucent_edges():
    from ingetrazo_render import fit_reference_rgba
    row = bytes([255, 0, 0, 0, 0, 0, 255, 255]) * 2
    width, height, rgba = fit_reference_rgba(4, 2, row * 2, 2)
    assert (width, height) == (2, 1)
    assert all(r == g == 0 and b == 255 and 0 < a < 255 for r, g, b, a in rgba.reshape((-1, 4)))


@pytest.mark.skipif(sys.platform != "darwin", reason="macOS ImageIO")
def test_platform_image_keeps_top_row_and_straight_alpha(tmp_path, monkeypatch):
    import struct
    from ingetrazo_render import reference_texture, raster_dimensions
    from views import wgpu_scene
    image = QImage(16, 16, QImage.Format_RGBA8888)
    image.fill(QColor(0, 0, 255, 255))
    image.setPixelColor(0, 0, QColor(255, 0, 0, 128))
    png = tmp_path / "art.png"
    assert image.save(str(png))
    data = png.read_bytes()
    path = tmp_path / "art.icns"
    path.write_bytes(b"icns" + struct.pack(">I", len(data) + 16)
                     + b"icp4" + struct.pack(">I", len(data) + 8) + data)
    monkeypatch.setattr(wgpu_scene, "_legacy_texture_pixels",
                        lambda _: pytest.fail("Requested a Qt platform codec"))
    assert raster_dimensions(path) == (16, 16)
    w, h, pixels = reference_texture(path, 16)
    assert (w, h) == (16, 16)
    assert pixels[:4].tolist() == [255, 0, 0, 128]
    assert pixels[-4:].tolist() == [0, 0, 255, 255]
