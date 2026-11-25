# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The PDF writer (built_core sheets::pdf), read back with PyMuPDF:
a page per sheet at the paper's size; every line of the display lists at
its place with its pen; the texts in the embedded Inter, extractable, laid
out where Qt's drawText puts them; a frame's render as an image of its
pixels; the border on top. Synthetic sheets, no model files."""
from __future__ import annotations

import json
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np  # noqa: E402
import pytest  # noqa: E402
from pathlib import Path  # noqa: E402

import built_core as NATIVE  # noqa: E402
from built_render import SurveyRaster  # noqa: E402
from tests.headless_gpu import pixel  # noqa: E402

pymupdf = pytest.importorskip("pymupdf")
PT = 72.0 / 25.4


def _sheets():
    from built_core.composition import (BarraEscala, Composicion, CotaItem, MarcoVista, TextoItem)
    a = Composicion(name="Plans", paper="A3", landscape=True, border=True, border_style="double")
    a.frames.append(MarcoVista(x_mm=20, y_mm=20, w_mm=120, h_mm=90, scale_n=100.0, view_key="std:top",
                               style="vectorial", show_title=True, title_number="1", title_sheet="A-01",
                               border=True, cut_fill="hatch"))
    a.frames.append(MarcoVista(x_mm=160, y_mm=20, w_mm=100, h_mm=80, scale_n=150.0, view_key="std:iso",
                               style="sombreado", show_scale=True, scale_pos="inside-br"))
    a.texts.append(TextoItem(x_mm=20, y_mm=130, w_mm=90, text="Planta baja — Ñandú «AV» 2,5 m²",
                             size_pt=14.0, bold=True))
    a.texts.append(TextoItem(x_mm=20, y_mm=150, w_mm=60, size_pt=9.0, italic=True, align="right",
                             text="Una nota larga que se parte en varias líneas para ver el ajuste."))
    a.scalebars.append(BarraEscala(x_mm=160, y_mm=130, scale_n=100.0))
    a.cotas.append(CotaItem(x_mm=160, y_mm=160, dx_mm=60.0, dy_mm=0.0, scale_n=100.0))
    a.cajetin = a.default_cajetin()
    a.cajetin.set_field("PROYECTO", "Vivienda unifamiliar")
    b = Composicion(name="Details", paper="A4", landscape=False)
    b.texts.append(TextoItem(x_mm=15, y_mm=15, w_mm=100, text="Second sheet", size_pt=20.0))
    return [a, b]


def _content(frame):
    rng = np.random.default_rng(3)
    segs = rng.uniform(0.0, 80.0, (25, 4))
    kinds = np.array([k % 4 for k in range(25)], np.int8)
    fills = [np.array([[10, 10], [40, 10], [40, 30], [10, 30]], float)]
    return {"segs": segs, "kinds": kinds, "fills": fills}


@pytest.fixture
def written(tmp_path):
    a, b = _sheets()
    vector, shaded = a.frames
    w, h = 400, 300
    rgba = np.zeros((h, w, 4), np.uint8)
    rgba[..., 0] = np.arange(w, dtype=np.uint8)[None, :]
    rgba[..., 1] = 120
    rgba[..., 2] = np.arange(h, dtype=np.uint8)[:, None]
    rgba[..., 3] = 255
    contents = {vector._doc_key: _content(vector), shaded._doc_key: {"image": (w, h)}}
    pictures = {f"frame:{shaded._doc_key}": (w, h, rgba.tobytes())}
    path = tmp_path / "sheets.pdf"
    NATIVE.sheet_pdf(str(path), [a, b], contents, pictures)
    return path, (a, b), contents


def _display(item, contents):
    from built_core.composition import MarcoVista
    if isinstance(item, MarcoVista):
        return json.loads(NATIVE.sheet_frame_display(item, contents.get(item._doc_key)))
    return json.loads(NATIVE.sheet_item_display(item))


def test_a_page_per_sheet_at_the_paper_s_size(written):
    path, (a, b), _ = written
    doc = pymupdf.open(str(path))
    assert doc.page_count == 2
    for page, comp in zip(doc, (a, b)):
        w, h = comp.page_size_mm()
        assert page.rect.width == pytest.approx(w * PT, abs=0.01)
        assert page.rect.height == pytest.approx(h * PT, abs=0.01)


def test_every_line_of_the_display_lists_is_drawn_where_and_as_it_says(written):
    path, (a, _b), contents = written
    page = pymupdf.open(str(path))[0]
    drawn = []
    for d in page.get_drawings():
        for it in d["items"]:
            if it[0] == "l":
                drawn.append((it[1].x / PT, it[1].y / PT, it[2].x / PT, it[2].y / PT, d.get("width") or 0.0))
    drawn = np.array(drawn)
    want = []
    for item in a.all_items():
        for op in _display(item, contents):
            if op["op"] == "line" and op.get("pen"):
                x0, y0, x1, y1 = op["pts"]
                want.append((x0 + item.x_mm, y0 + item.y_mm, x1 + item.x_mm, y1 + item.y_mm, op["pen"]["width"]))
    assert len(want) > 30
    missing = 0
    for x0, y0, x1, y1, width in want:
        d1 = np.abs(drawn[:, :4] - [x0, y0, x1, y1]).max(axis=1)
        d2 = np.abs(drawn[:, :4] - [x1, y1, x0, y0]).max(axis=1)
        hit = np.minimum(d1, d2) < 0.01
        if not hit.any():
            missing += 1
            continue
        assert np.isclose(drawn[hit, 4] / PT, width, atol=0.01).any(), (x0, y0, width)
    # Lines a clip leaves out entirely are not missing: PyMuPDF still lists them.
    assert missing == 0


def test_the_texts_are_inter_extractable_and_where_qt_puts_them(written):
    path, (a, b), _ = written
    doc = pymupdf.open(str(path))
    fonts = {f[3] for f in doc[0].get_fonts()}
    assert {"Inter-Regular", "Inter-SemiBold"} <= fonts
    text = doc[0].get_text()
    assert "Planta baja — Ñandú «AV» 2,5 m²" in text
    assert "Vivienda unifamiliar" in text
    assert "Second sheet" in doc[1].get_text()
    # The bold one-liner: its first baseline at the box top plus the
    # ascent, its first glyph at the box's left (AlignLeft | AlignTop).
    item = a.texts[0]
    op = next(o for o in json.loads(NATIVE.sheet_item_display(item)) if o["op"] == "text")
    spans = [s for blk in doc[0].get_text("rawdict")["blocks"] for ln in blk.get("lines", ())
             for s in ln["spans"] if "".join(c["c"] for c in s["chars"]).startswith("Planta")]
    assert spans, "the bold text"
    origin = spans[0]["chars"][0]["origin"]
    box = op["rect"]
    left = box[0][0] + item.x_mm
    top = box[0][1] + item.y_mm
    ascent = _inter_ascent("Inter-SemiBold.ttf") * op["size"]
    assert origin[0] / PT == pytest.approx(left, abs=0.05)
    assert origin[1] / PT == pytest.approx(top + ascent, abs=0.05)
    assert "SemiBold" in spans[0]["font"]


def _lightness(rgba):
    """HSL lightness, 0-255, as QColor.lightness() answers it."""
    return (max(rgba[:3]) + min(rgba[:3])) // 2


def _inter_ascent(name):
    """The bundled Inter's ascent per unit of size, from its hhea table:
    what the sheet font metrics (sheets::metrics) lay text out with."""
    import struct
    data = (Path(__file__).resolve().parents[1] / "resources" / "fonts" / "inter" / name).read_bytes()
    tables = {}
    for i in range(struct.unpack(">H", data[4:6])[0]):
        tag, _, off, _ = struct.unpack(">4sIII", data[12 + 16 * i:28 + 16 * i])
        tables[tag] = off
    upm = struct.unpack(">H", data[tables[b"head"] + 18:tables[b"head"] + 20])[0]
    return struct.unpack(">h", data[tables[b"hhea"] + 4:tables[b"hhea"] + 6])[0] / upm


def test_a_frame_s_render_is_an_image_of_its_pixels(written):
    path, (a, _b), _ = written
    doc = pymupdf.open(str(path))
    imgs = doc[0].get_images(full=True)
    assert len(imgs) == 1
    xref = imgs[0][0]
    pix = pymupdf.Pixmap(doc, xref)
    assert (pix.width, pix.height) == (400, 300)
    px = np.frombuffer(pix.samples, np.uint8).reshape(300, 400, pix.n)
    assert px[0, 10, 0] == 10 and px[5, 0, 2] == 5 and px[7, 7, 1] == 120
    shaded = a.frames[1]
    rect = doc[0].get_image_rects(xref)[0]
    # 400 × 300 px at 300 dpi is smaller than the frame: drawn at its own size.
    assert rect.x0 / PT == pytest.approx(shaded.x_mm, abs=0.01)
    assert rect.width / PT == pytest.approx(400 * 25.4 / 300, abs=0.01)


def test_the_image_export_draws_the_same_page(tmp_path):
    """sheets::raster: the page the PDF writer writes, on white paper at
    the dpi asked: the frame's render where the frame is, ink where the
    lines and the text are, paper elsewhere."""
    a, _b = _sheets()
    vector, shaded = a.frames
    w, h = 400, 300
    rgba = np.zeros((h, w, 4), np.uint8)
    rgba[..., 0] = 200
    rgba[..., 3] = 255
    contents = {vector._doc_key: _content(vector), shaded._doc_key: {"image": (w, h)}}
    out = tmp_path / "sheet.png"
    size = NATIVE.sheet_image(str(out), a, 100.0, contents, {f"frame:{shaded._doc_key}": (w, h, rgba.tobytes())})
    pw, ph = a.page_size_mm()
    assert size == (round(pw * 100 / 25.4), round(ph * 100 / 25.4))
    img = SurveyRaster.open(str(out))
    assert (img.width, img.height) == size
    px = lambda x_mm, y_mm: pixel(img, int(x_mm * 100 / 25.4), int(y_mm * 100 / 25.4))  # noqa: E731
    assert px(shaded.x_mm + 5, shaded.y_mm + 5)[:2] == (200, 0)
    assert _lightness(px(200, 180)) == 255                          # paper
    # The bold text's ink, inside its box.
    t = a.texts[0]
    inked = [_lightness(px(t.x_mm + dx * 0.25, t.y_mm + dy * 0.25))
             for dx in range(0, 200) for dy in range(0, 24)]
    assert min(inked) < 80
    jpg = tmp_path / "sheet.jpg"
    NATIVE.sheet_image(str(jpg), a, 50.0, contents, {f"frame:{shaded._doc_key}": (w, h, rgba.tobytes())})
    assert jpg.read_bytes()[:2] == b"\xff\xd8"


def test_an_image_item_shows_its_shape_and_opacity_in_the_files(tmp_path):
    """sheets::picture: an image item cut to an ellipse at half opacity
    leaves its box's corners paper and blends its middle with the paper."""
    from built_core.composition import Composicion, ImagenItem
    pic = tmp_path / "red.png"
    red = np.zeros((200, 400, 4), np.uint8)
    red[..., 0] = red[..., 3] = 255
    assert SurveyRaster(400, 200, red.tobytes()).save(str(pic))
    comp = Composicion(name="I", paper="A4", landscape=True)
    comp.images.append(ImagenItem(x_mm=20, y_mm=20, w_mm=80, h_mm=40, path=str(pic), shape="ellipse", opacity=0.5))
    out = tmp_path / "i.png"
    NATIVE.sheet_image(str(out), comp, 100.0)
    got = SurveyRaster.open(str(out))
    px = lambda x, y: pixel(got, int(x * 100 / 25.4), int(y * 100 / 25.4))  # noqa: E731
    assert px(21, 21)[:3] == (255, 255, 255)                        # a corner: cut away
    mid = px(60, 40)
    assert mid[0] == 255 and 120 <= mid[1] <= 135                  # half red on white
    NATIVE.sheet_pdf(str(tmp_path / "i.pdf"), [comp])
    doc = pymupdf.open(str(tmp_path / "i.pdf"))
    assert len(doc[0].get_images(full=True)) == 1
