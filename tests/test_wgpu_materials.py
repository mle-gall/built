"""Material handoff tests require no window or GPU device."""
import sys

import numpy as np
import pytest
from ingetrazo_core import source_arrays
from ingetrazo_render import material_texture_array
from ingetrazo_render import SurveyRaster
from ingetrazo_core import Vec3 as QVector3D
from core.mesh import Mesh


def mesh_with(attrs):
    mesh = Mesh()
    mesh.add_face([QVector3D(0,0,0), QVector3D(2,0,0), QVector3D(0,3,0)])
    mesh.faces[0].attrs.update(attrs)
    return mesh


def test_front_paint_back_default_and_mirrored_paint():
    own = {'color': (0.4,0.1,0.05)}
    front = source_arrays(mesh_with(own))
    np.testing.assert_array_equal(front[6][1], [102,25,12,255])
    assert front[10][0,0] == 0
    both = source_arrays(mesh_with({**own, 'back': True}))
    assert both[10][0,0] == int.from_bytes(bytes([102,25,12,255]), 'little')


def test_separate_back_color_and_container_precedence():
    own = {'color': (0.4,0.1,0.05), 'back': {'color': (0,1,0)}}
    arrays = source_arrays(mesh_with(own), {'color': (0,0,1)})
    np.testing.assert_array_equal(arrays[6][1], [102,25,12,255])
    assert arrays[10][0,0] == 0xff00ff00
    inherited = source_arrays(mesh_with({}), {'color': (0,0,1)})
    assert inherited[10][0,0] == 0xffff0000


def test_layer_color_replaces_explicit_face_material():
    arrays = source_arrays(mesh_with({'color': (1,0,0), 'layer': 'tag'}),
                            {'by_layer': {'tag': (0,1,0)}, 'color': (0,0,1)})
    np.testing.assert_array_equal(arrays[6][1], [0,255,0,255])
    assert arrays[10][0,0] == 0xff00ff00


def test_front_and_back_affines_preserve_leaf_grain():
    tex = {'path': 'unused.png', 'uvw': [2,0,0,.5,0,3,0,-1]}
    other = {**tex, 'uvw': [0,0,4,2,1,1,0,0]}
    ids = {}
    arrays = source_arrays(mesh_with({'texture': tex, 'back': {'texture': other}}),
                            {'texture': {**tex,'uvw': [0]*8}}, ids)
    np.testing.assert_array_equal(arrays[9][0], tex['uvw']+other['uvw'])
    assert arrays[10][0,1:].tolist() == [1,1]
    assert ids == {'unused.png': 1}


def _png(path, w, h, pixels):
    """Write ``pixels`` (rows of (r, g, b, a), top first) as a PNG."""
    assert len(pixels) == w * h
    assert SurveyRaster(w, h, bytes(v for px in pixels for v in px)).save(str(path))


def _pdf(path, w, h, content):
    """A one-page PDF of ``w`` × ``h`` points drawing ``content``."""
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>",
               b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
               b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 %d %d] /Contents 4 0 R >>" % (w, h),
               b"<< /Length %d >>\nstream\n%s\nendstream" % (len(content), content)]
    out, offsets = bytearray(b"%PDF-1.4\n"), []
    for n, body in enumerate(objects, 1):
        offsets.append(len(out))
        out += b"%d 0 obj\n%s\nendobj\n" % (n, body)
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    out += b"".join(b"%010d 00000 n \n" % o for o in offsets)
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)
    path.write_bytes(bytes(out))


def test_array_flips_v_and_supplies_mips(tmp_path):
    red, blue = (255, 0, 0, 255), (0, 0, 255, 255)
    path = tmp_path/'tile.png'
    _png(path, 2, 2, [red, red, blue, red])
    w,h,layers,levels = material_texture_array({str(path):1})
    assert (w,h,layers) == (2,2,2)
    assert [len(v) for v in levels] == [32,8]
    assert levels[0][16:20] == bytes([0,0,255,255])


def test_material_pixels_use_native_codecs_and_preserve_alpha(tmp_path, monkeypatch):
    path = tmp_path / "transparent.png"
    pixels = [(255, 20, 0, 0), (0, 20, 255, 255)]
    _png(path, 2, 1, pixels)
    monkeypatch.setitem(sys.modules, "PySide6.QtGui", None)
    w, h, count, mips = material_texture_array([path, tmp_path / "missing.png"])
    assert (w, h, count) == (2, 1, 3)
    assert mips[0][8:16] == bytes(v for px in pixels for v in px)
    assert mips[0][-8:] == b"\xff" * 8
    r, g, b, a = mips[1][4:8]
    assert r == 0 and abs(g - 20) <= 1 and b >= 254 and abs(a - 128) <= 1


def test_system_codec_fallback_uses_same_orientation_and_budget(tmp_path, monkeypatch):
    # XPM under an unrecognised suffix exercises content detection at both entry points.
    from ingetrazo_render import raster_dimensions
    import sys
    path = tmp_path / "texture.custom"
    path.write_text('/* XPM */\nstatic char *p[] = {"1 2 2 2", "aa c #010203", '
                    '" b c #040506", "aa", " b"};')
    monkeypatch.setitem(sys.modules, "PySide6.QtGui", None)
    assert raster_dimensions(path) == (1, 2)
    w, h, count, mips = material_texture_array([path])
    assert (w, h, count) == (1, 2, 2)
    assert mips[0][8:] == bytes((4, 5, 6, 255, 1, 2, 3, 255))


@pytest.mark.parametrize("ext", ["tiff", "svgz", "gif"])
def test_specialty_files_decode_once_for_materials_and_references(tmp_path, monkeypatch, ext):
    import gzip
    import struct
    from ingetrazo_render import reference_texture, raster_dimensions
    import ingetrazo_core

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
    monkeypatch.setitem(sys.modules, "PySide6.QtGui", None)
    assert raster_dimensions(path) == (1, 2)
    w, h, data = reference_texture(path, 16)
    assert (w, h) == (1, 2) and data.tolist() == [255, 255, 255, 255, 0, 0, 0, 255]
    w, h, count, mips = material_texture_array([path])
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
    png = tmp_path / "art.png"
    _png(png, 16, 16, [(255, 0, 0, 128)] + [(0, 0, 255, 255)] * 255)
    data = png.read_bytes()
    path = tmp_path / "art.icns"
    path.write_bytes(b"icns" + struct.pack(">I", len(data) + 16)
                     + b"icp4" + struct.pack(">I", len(data) + 8) + data)
    monkeypatch.setitem(sys.modules, "PySide6.QtGui", None)
    assert raster_dimensions(path) == (16, 16)
    w, h, pixels = reference_texture(path, 16)
    assert (w, h) == (16, 16)
    assert pixels[:4].tolist() == [255, 0, 0, 128]
    assert pixels[-4:].tolist() == [0, 0, 255, 255]


@pytest.mark.parametrize(("suffix", "contents", "top", "bottom"), [
    ("xbm", b"#define test_width 1\n#define test_height 2\n"
            b"static unsigned char test_bits[] = {0x01, 0x00};", (0, 0, 0, 255), (255, 255, 255, 255)),
    ("wbmp", bytes((0, 0, 1, 2, 0, 128)), (0, 0, 0, 255), (255, 255, 255, 255)),
    ("xpm", b'/* XPM */\nstatic char *p[] = {"1 2 2 1", "  c None", '
            b'"z g black c Light Sky Blue3", " ", "z"};', (0, 0, 0, 0), (141, 182, 205, 255)),
])
def test_native_bitmap_header_and_reference(tmp_path, suffix, contents, top, bottom, monkeypatch):
    from ingetrazo_render import raster_dimensions, reference_texture
    import sys
    path = tmp_path / f"pixels.{suffix}"
    path.write_bytes(contents)
    monkeypatch.setitem(sys.modules, "PySide6.QtGui", None)
    assert raster_dimensions(path) == (1, 2)
    w, h, pixels = reference_texture(path, 32)
    assert (w, h, bytes(pixels)) == (1, 2, bytes((*top, *bottom)))


def test_cursor_resource_hotspot_and_pixels(tmp_path):
    from ingetrazo_render import raster_dimensions, reference_texture
    import struct
    png = tmp_path / "frame.png"
    _png(png, 16, 16, [(4, 19, 112, 172)] * 256)
    data = png.read_bytes()
    path = tmp_path / "cursor.cur"
    path.write_bytes(struct.pack("<HHHBBBBHHII", 0, 2, 1, 16, 16, 0, 0, 4, 9, len(data), 22) + data)
    assert raster_dimensions(path) == (16, 16)
    assert bytes(reference_texture(path, 32)[2]) == bytes((4, 19, 112, 172)) * 256


@pytest.mark.skipif(__import__("sys").platform != "darwin", reason="Core Graphics PDF reader")
def test_pdf_first_page_size_orientation_and_transparency(tmp_path, monkeypatch):
    from ingetrazo_render import raster_dimensions, reference_texture
    import sys
    path = tmp_path / "drawing.pdf"
    _pdf(path, 24, 32, b"1 0 0 rg 0 16 12 16 re f")    # red, the top-left quarter
    monkeypatch.setitem(sys.modules, "PySide6.QtGui", None)
    assert raster_dimensions(path) == (24, 32)
    _, _, rgba = reference_texture(path, 32)
    assert rgba[:4].tolist() == [255, 0, 0, 255]
    assert rgba[-4:].tolist() == [0, 0, 0, 0]
