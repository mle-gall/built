"""Verify overlay pixels independently of native presentation and GPU access."""
import sys

import pytest
from views.wgpu_overlay import Primitive, axes, pixels, rasterize, snap_glyph


def test_rgba_scale_and_transparent_background():
    image = rasterize([Primitive("rectangle", ((10, 10), (30, 30)),
                                 "#ff0000", fill="#ff0000")], 100, 80, 2)
    from tests.headless_gpu import pixel
    assert (image.width, image.height) == (200, 160)
    assert pixel(image, 40, 40) == (255, 0, 0, 255)
    assert pixel(image, 150, 100)[3] == 0
    assert pixels(image).size == 200 * 160 * 4


def test_all_primitives_and_glyphs():
    items = [snap_glyph((20, 20), "endpoint"), snap_glyph((40, 40), "midpoint"),
             snap_glyph((60, 60), "on_edge"),
             Primitive("text", ((20, 80),), text="12.5 m"),
             *axes((90, 90), ((1, 0), (0, 1), (-1, -1)))]
    image = rasterize(items, 150, 150, 2)
    assert pixels(image).reshape(-1, 4)[:, 3].sum() > 0
    with pytest.raises(ValueError, match="Unknown"):
        rasterize([Primitive("invalid", ())], 10, 10, 1)


PRIMITIVE_DIGESTS = {1: "eb3d8c013fc4eb6f12298a9a", 2: "f482445fab1a60937c3510a4",
                     3: "6647da35c4c1a39969647fe4"}


# Font rasterization differs in the text and axis labels. Captured from
# the RGBA artifact of the x86-64 Linux release build.
LINUX_DIGESTS = {1: "7ade38d35c27eeaedcde9132", 2: "d5336dffddfd1415cfdffb36",
                 3: "245de47bc64dc6a366fb76d2"}


def _primitive_set():
    from built_core import Input as Qt
    return [
        Primitive("polyline", ((10, 10), (90.3, 40.7), (30, 80)), "#5a6e8c", 1 / 3),
        Primitive("polyline", ((5, 95), (195, 95)), "#f2732a", 1.5, dash=(6, 3)),
        Primitive("polygon", ((100, 10), (150, 20), (130, 70)), "#289e5c", 2.0, fill="#40289e5c"),
        Primitive("rectangle", ((180.5, 60.2), (120.1, 15.7)), "#3366c7", 1.5, fill="#1c3366c7",
                  pen_style=Qt.DashLine),
        Primitive("rectangle", ((20, 20), (60, 50)), "#f37329", 1.5, pen_style=Qt.DotLine),
        Primitive("point", ((160, 80),), "#ff00ff", 3.5, fill="#ff00ff"),
        Primitive("text", ((40, 120),), "#00dfff", text="12.5 m"),
        snap_glyph((70, 140), "midpoint"), snap_glyph((110, 140), "vertex", "#1aa633"),
        *axes((150, 140), ((1, 0), (0, 1), (-1, -1))),
    ]


def primitive_pixels(dpr):
    from built_render import AnnotationCanvas
    from views.wgpu_overlay import paint_layer, selection_rectangle
    canvas = AnnotationCanvas(200 * dpr, 200 * dpr, 200.)
    items = _primitive_set() + [selection_rectangle((10, 150), (60, 190)),
                                selection_rectangle((190, 150), (130, 190), zoom=True)]
    paint_layer(canvas, items)
    return canvas.composite(bytes(200 * dpr * 200 * dpr * 4))


@pytest.mark.parametrize("dpr", [1, 2, 3])
def test_primitives_are_pinned(dpr):
    """The loop moved from Python to AnnotationCanvas.draw_primitives. The
    digests are of the Python loop on the same canvas with the native paint
    types, before the port."""
    import hashlib
    reference = LINUX_DIGESTS if sys.platform == "linux" else PRIMITIVE_DIGESTS
    assert hashlib.sha256(primitive_pixels(dpr)).hexdigest()[:24] == reference[dpr]
