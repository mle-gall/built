# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""SVG writer for view exports: the hidden-line view as a vector drawing
to finish in an illustration program.

It takes the very groups the DXF export writes — ``(layer, segments[,
linetype[, rgb[, dashes[, weight_mm]]]])`` with the segments ``(x0, y0,
x1, y1)`` in metres — and writes one SVG layer per group (a top-level ``<g>`` named
after it, tagged as a layer the way the free vector editors read it), its
lines in the layer's colour, line type and pen. The drawing goes out at
paper size: ``scale_n`` is the 1:N the metres are drawn at, in
millimetres.

Segments that continue one another (the pieces of one edge, the facets of
an arc) are chained into one path; a corner or a junction ends it, so each
edge comes apart on its own.
"""
from __future__ import annotations

import re
from pathlib import Path
from xml.sax.saxutils import quoteattr

import ingetrazo_core as NATIVE
import numpy as np

#: The pen of a group by its line class when it brings none of its own:
#: the vector style's cut / profile / edge weights, in paper millimetres.
DEFAULT_PENS = {"cut": 0.5, "profile": 0.35, "edge": 0.18}


def _hex(rgb) -> str:
    if rgb is None:
        return "#000000"
    r, g, b = (max(0, min(255, round(float(c) * 255.0))) for c in rgb[:3])
    return f"#{r:02x}{g:02x}{b:02x}"


def _xml_id(name: str, taken: set) -> str:
    base = re.sub(r"[^A-Za-z0-9_.-]", "_", name.strip()) or "capa"
    if not re.match(r"[A-Za-z_]", base):
        base = "_" + base
    out, i = base, 2
    while out in taken:
        out, i = f"{base}_{i}", i + 1
    taken.add(out)
    return out


#: Segments that continue one another joined into polylines (only where
#: exactly two meet and the line turns at most 30°): ``[(points, closed)]``.
#: In Rust (ingetrazo_core sheets::svg, docs/rust-sheets.md §5).
chain_segments = NATIVE.chain_segments


def save_svg_layers(path: str | Path, groups, scale_n: float = 100.0,
                    pens: dict | None = None, faces=None,
                    line_colours: bool = True, image=None,
                    paper=None) -> int:
    """Write the view ``groups`` (as :func:`formats.dxf_out.save_dxf_layers`
    takes them) to an SVG at 1:``scale_n``, one layer per non-empty group
    — see :func:`svg_text` for the rest. Returns the number of line
    segments written."""
    text, n = svg_text(groups, scale_n, pens, faces, line_colours, image,
                       paper)
    Path(path).write_text(text, encoding="utf-8")
    return n


def svg_text(groups, scale_n: float = 100.0, pens: dict | None = None,
             faces=None, line_colours: bool = True, image=None, paper=None,
             bake_px_per_mm: float | None = None, window=None,
             margin: float | None = None, seam_mm: float | None = None):
    """The SVG of a view as ``(text, line segments)``.

    ``pens`` overrides :data:`DEFAULT_PENS` (``cut`` / ``profile`` /
    ``edge`` in mm); a group's own sixth element, its weight in mm, wins
    over both.

    ``faces`` — the view's filled faces (:func:`core.vista_caras.view_faces`,
    in the same plane as the lines) go UNDER the lines, one layer per model
    layer («<layer>-CARAS»; the see-through ones on «<layer>-CARAS-TRANSP»
    above the opaque ones, the sun's shadows on «<layer>-SOMBRAS» over
    them). Each face is one shape filled with its colour or its texture
    laid out as the view shows it (see :func:`_face_layers`).
    ``line_colours`` off: every line in black, whatever its layer's
    colour (the «Default colour only» export); line types and pens stay.

    ``image`` — ``(QImage, (x0, y0, x1, y1))``: a picture of the view and
    the window it shows, in the drawing's plane; it goes under everything
    on a layer of its own, and the page is that window.

    ``paper`` — ``(width_mm, height_mm)``: the page is that sheet and the
    drawing is fitted into it, centred, within the margin — at whatever
    scale fits (the ``desc`` says which); without it the page is the
    drawing at 1:``scale_n`` plus the margin. ``bake_px_per_mm`` draws the
    textures coarser (a preview).

    ``window`` — ``(x0, y0, x1, y1)`` in the drawing's plane: the part of
    it the page shows (what the screen shows, say) instead of the whole
    drawing; ``margin`` — the page's margin in mm (10 by default; 0 to
    fill the page with the window edge to edge). ``seam_mm``: the hair
    of colour round each face (a small preview wants it wider)."""
    # The page, the line layers and their chaining are Rust's
    # (ingetrazo_core sheets::svg); the faces and the picture are laid
    # below them here, on the page Rust works out.
    faces = list(faces or ())
    fixed = image[1] if window is None and image is not None else None
    extra = None
    if window is None and image is None and faces:
        xs = np.concatenate([np.asarray(r, dtype=float)[:, 0] for f in faces for r in f.rings])
        ys = np.concatenate([np.asarray(r, dtype=float)[:, 1] for f in faces for r in f.rings])
        extra = (float(xs.min()), float(ys.min()), float(xs.max()), float(ys.max()))
    picture = image is not None and image[0] is not None and not image[0].isNull()

    def under(page, taken):
        k = page["k"]

        def px(x):
            return (x - page["min_x"]) * k + page["off_x"]

        def py(y):                             # SVG's y grows downwards
            return (page["max_y"] - y) * k + page["off_y"]
        out = []
        clip = page["clip"]
        if picture:
            # On its own window: a sheet of another shape widens the page's
            # window round it, the picture stays where it was taken.
            ix0, iy0, ix1, iy1 = image[1]
            name = "VISTA-IMAGEN"
            out.append(f'<g id="{_xml_id(name, taken)}" '
                       f'inkscape:groupmode="layer" '
                       f'inkscape:label={quoteattr(name)}{clip}>\n'
                       f'<image x="{px(ix0):.3f}" y="{py(iy1):.3f}" '
                       f'width="{(ix1 - ix0) * k:.3f}" '
                       f'height="{(iy1 - iy0) * k:.3f}" '
                       f'preserveAspectRatio="none" '
                       f'xlink:href="{_encode(image[0])}"/>\n</g>\n')
        if faces:
            out += _face_layers(faces, px, py, taken, bake_px_per_mm, clip,
                                (page["width"], page["height"]),
                                seam_mm or _SEAM_MM)
        return "".join(out)
    return NATIVE.svg_lines(
        groups, float(scale_n), pens, bool(line_colours),
        None if paper is None else (float(paper[0]), float(paper[1])),
        None if window is None else tuple(map(float, window)),
        None if margin is None else float(margin),
        None if fixed is None else tuple(map(float, fixed)), extra,
        under if (picture or faces) else None)


#: Longest side, in pixels, a texture is read at: enough for a sheet,
#: and a 4k photo texture does not make the file ten times heavier.
MAX_TEXTURE_PX = 1024

#: How finely a textured face's fill is drawn: pixels per paper
#: millimetre (8 ≈ 200 dpi), and at most this many pixels a side.
BAKE_PX_PER_MM = 8.0
MAX_BAKE_PX = 2048

#: A hair of the face's own colour round each opaque face, so two
#: neighbours leave no light seam between them in a viewer.
_SEAM_MM = 0.15


def _face_layers(faces, px, py, taken, bake_px_per_mm=None,
                 clip: str = "", page=(1.0e5, 1.0e5),
                 seam_mm: float = _SEAM_MM) -> list:
    """The ``<defs>`` with the textured faces' fills and the face layers.

    A textured face is filled with its texture already laid out as the view
    shows it — repeated, turned and slanted onto the face, darkened by its
    light — drawn here into one image over the face's box on the page. The
    fill needs no pattern transform, which the illustration programs read
    unevenly (the bricks came in level and unslanted where the view has
    them along the wall)."""
    defs: list = []
    textures: dict = {}        # (path, shade) → QImage, or None
    layers: dict = {}          # (layer, see-through) → [path elements]
    shadows: dict = {}         # layer → [path elements]
    ground: list = []          # the shadows on the bare ground

    def path_d(rings):
        return " ".join(
            "M" + " L".join(f"{px(x):.3f},{py(y):.3f}" for x, y in ring)
            + " Z" for ring in rings)

    def texture(path, shade):
        key = (str(path), round(float(shade), 5))
        if key not in textures:
            textures[key] = _shaded_texture(path, shade)
        return textures[key]

    for f in faces:
        d = path_d(f.rings)
        if f.extra.get("ground_shadow"):
            ground.append(f'<path d="{d}" fill="#000000" '
                          f'fill-opacity="{f.opacity:.3g}"/>\n')
            continue
        if f.shadow:
            # A black veil as dark as the shadow settings say: over the
            # face's colour or texture it gives the shaded tone, and stays
            # one shape to recolour, fade or switch off with its layer.
            shadows.setdefault(f.layer, []).append(
                f'<path d="{path_d(f.shadow)}" fill="#000000" '
                f'fill-opacity="{f.shadow_alpha:.3g}"/>\n')
        fill = _hex(f.rgb)
        tex = f.texture
        if tex is not None:
            img = texture(tex["path"], tex.get("shade", 1.0))
            m = _uv_to_page(tex["uv"], px, py) if img is not None else None
            baked = (_bake(img, m, f.rings, px, py, bake_px_per_mm)
                     if m is not None else None)
            if baked is not None:
                bx, by, bw, bh, uri = baked
                pid = _xml_id(f"cara_{len(defs) + 1}", taken)
                # The tile is the whole page from its origin and the image
                # sits where the face is: a tile starting at the face's box
                # with the image at its corner is placed one way by some
                # renderers and another by others (the preview showed the
                # bricks shifted, the page through the gaps).
                pw, ph = page
                defs.append(
                    f'<pattern id="{pid}" patternUnits="userSpaceOnUse" '
                    f'x="0" y="0" width="{pw:.3f}" height="{ph:.3f}">'
                    f'<image x="{bx:.3f}" y="{by:.3f}" '
                    f'width="{bw:.3f}" height="{bh:.3f}" '
                    f'preserveAspectRatio="none" xlink:href="{uri}"/>'
                    f'</pattern>\n')
                fill = f"url(#{pid})"
        attrs = f'fill="{fill}"'
        see_through = f.opacity < 0.999
        if see_through:
            attrs += f' fill-opacity="{f.opacity:.3g}"'
        else:
            attrs += (f' stroke="{_hex(f.rgb)}" stroke-width="{seam_mm:g}" '
                      'stroke-linejoin="round"')
        layers.setdefault((f.layer, see_through), []).append(
            f'<path d="{d}" {attrs}/>\n')
    out = []
    if defs:
        out += ["<defs>\n", *defs, "</defs>\n"]

    def layer(name, paths):
        out.append(f'<g id="{_xml_id(name, taken)}" '
                   f'inkscape:groupmode="layer" '
                   f'inkscape:label={quoteattr(name)}{clip} '
                   'fill-rule="evenodd">\n')
        out.extend(paths)
        out.append("</g>\n")

    if ground:
        layer("SOMBRAS-SUELO", ground)
    for see in (False, True):
        for (name_of, flag), paths in layers.items():
            if flag == see:
                layer(f"{name_of}-CARAS" + ("-TRANSP" if see else ""), paths)
    for name_of, paths in shadows.items():
        layer(f"{name_of}-SOMBRAS", paths)
    return out


def _uv_to_page(pins, px, py):
    """The map (a ``QTransform``) taking a texture tile's own coordinates
    — (u, −v), so the image's top row sits at v = 1 as the viewport draws
    it — to the page, through the four pinned points: projective, so a
    face seen in perspective gets its texture exactly as the viewport lays
    it (an affine map only guesses it). ``None`` for a degenerate face."""
    from PySide6.QtCore import QPointF
    from PySide6.QtGui import QPolygonF, QTransform
    src = QPolygonF([QPointF(u, -v) for _xy, (u, v) in pins])
    dst = QPolygonF([QPointF(px(x), py(y)) for (x, y), _uv in pins])
    t = QTransform()
    if not QTransform.quadToQuad(src, dst, t):
        return None
    return t


def _bake(img, m, rings, px, py, px_per_mm=None):
    """The texture ``img`` laid onto the page by ``m`` (tile → page),
    drawn into an image over the box of the face's ``rings``:
    ``(x, y, w, h, uri)`` in page millimetres, or ``None`` for a face too
    small to draw. The painter works in the tile's own coordinates and
    fills the face's own outline taken back there — never the box, whose
    far corners a perspective map can throw past the horizon."""
    import math

    from PySide6.QtCore import QPointF, Qt
    from PySide6.QtGui import (QBrush, QImage, QPainter, QPainterPath,
                               QPolygonF, QTransform)
    xs = [px(x) for r in rings for x in r[:, 0]]
    ys = [py(y) for r in rings for y in r[:, 1]]
    bx, by = min(xs), min(ys)
    bw, bh = max(xs) - bx, max(ys) - by
    if bw <= 1e-6 or bh <= 1e-6:
        return None
    back, ok = m.inverted()
    if not ok:
        return None
    s = min(px_per_mm or BAKE_PX_PER_MM, MAX_BAKE_PX / max(bw, bh))
    w, h = max(1, math.ceil(bw * s)), max(1, math.ceil(bh * s))
    alpha = img.hasAlphaChannel()
    out = QImage(w, h, QImage.Format_ARGB32 if alpha
                 else QImage.Format_RGB32)
    out.fill(Qt.transparent if alpha else Qt.white)
    # The face, a hair wider so its edge pixels are filled too, in tile
    # coordinates.
    grow = 1.0 / s
    face = QPainterPath()
    face.setFillRule(Qt.OddEvenFill)
    for r in rings:
        pts = [QPointF(px(x), py(y)) for x, y in r]
        cx = sum(p.x() for p in pts) / len(pts)
        cy = sum(p.y() for p in pts) / len(pts)
        wide = []
        for p in pts:
            dx, dy = p.x() - cx, p.y() - cy
            d = math.hypot(dx, dy) or 1.0
            wide.append(back.map(QPointF(p.x() + dx / d * grow,
                                         p.y() + dy / d * grow)))
        face.addPolygon(QPolygonF(wide))
        face.closeSubpath()
    # A tile that comes out smaller than the image: shrink the image to
    # about twice the pixels the tile will take first, smoothly — sampled
    # straight, a fine texture far away broke into stripes (moiré), which
    # the viewport's mipmaps never show.
    o = m.map(QPointF(0.0, 0.0))
    tile_px = [math.hypot(q.x() - o.x(), q.y() - o.y()) * s
               for q in (m.map(QPointF(1.0, 0.0)), m.map(QPointF(0.0, 1.0)))]
    tw = max(4, min(img.width(), math.ceil(2.0 * tile_px[0])))
    th = max(4, min(img.height(), math.ceil(2.0 * tile_px[1])))
    if tw < img.width() or th < img.height():
        img = img.scaled(tw, th, Qt.IgnoreAspectRatio,
                         Qt.SmoothTransformation)
    brush = QBrush(img)
    brush.setTransform(QTransform.fromScale(1.0 / img.width(),
                                            1.0 / img.height()))
    p = QPainter(out)
    p.setRenderHint(QPainter.SmoothPixmapTransform)
    p.setRenderHint(QPainter.Antialiasing)
    # Tile → page → this image's pixels.
    p.setTransform(m * QTransform.fromTranslate(-bx, -by)
                   * QTransform.fromScale(s, s))
    p.setPen(Qt.NoPen)
    p.setBrush(brush)
    p.drawPath(face)
    p.end()
    return bx, by, bw, bh, _encode(out)


def _shaded_texture(path, shade: float):
    """The texture at ``path`` darkened by ``shade`` (the face's light) and
    kept within :data:`MAX_TEXTURE_PX`, or ``None`` if it cannot be read."""
    from PySide6.QtCore import Qt
    from PySide6.QtGui import QColor, QImage, QPainter
    src = QImage(str(path))
    if src.isNull():
        return None
    if max(src.width(), src.height()) > MAX_TEXTURE_PX:
        src = src.scaled(MAX_TEXTURE_PX, MAX_TEXTURE_PX, Qt.KeepAspectRatio,
                         Qt.SmoothTransformation)
    alpha = src.hasAlphaChannel()
    img = src.convertToFormat(QImage.Format_ARGB32 if alpha
                              else QImage.Format_RGB32)
    if shade < 0.999:
        p = QPainter(img)
        p.setCompositionMode(QPainter.CompositionMode_Multiply)
        g = max(0, min(255, round(255 * float(shade))))
        p.fillRect(img.rect(), QColor(g, g, g))
        if alpha:                   # Multiply fills the clear pixels too
            p.setCompositionMode(QPainter.CompositionMode_DestinationIn)
            p.drawImage(0, 0, src)
        p.end()
    return img


def _encode(img) -> str:
    """``img`` as a ``data:`` URI: JPEG unless it has transparency."""
    import base64

    from PySide6.QtCore import QBuffer, QByteArray, QIODevice
    alpha = img.hasAlphaChannel()
    data = QByteArray()
    buf = QBuffer(data)
    buf.open(QIODevice.WriteOnly)
    img.save(buf, "PNG" if alpha else "JPG", -1 if alpha else 90)
    buf.close()
    mime = "image/png" if alpha else "image/jpeg"
    return f"data:{mime};base64," + base64.b64encode(bytes(data)).decode()


def _image_uri(path, shade: float):
    """The shaded texture at ``path`` as a ``data:`` URI, or ``None``."""
    img = _shaded_texture(path, shade)
    return None if img is None else _encode(img)
