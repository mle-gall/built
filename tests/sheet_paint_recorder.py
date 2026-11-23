# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A QPainter stand-in that records what the sheet painters draw as
primitives in the painter's base frame (paper mm), for the painter parity
goldens (tests/test_sheet_paint_parity.py, docs/rust-sheets.md §6).

Each primitive is a dict: ``op`` (line | poly | ellipse | rrect | path |
text | image | clip), its geometry mapped through the painter's transform,
and the pen and brush in force. ``ingetrazo_core`` sheets::paint records the
same shape of list, so the two compare primitive by primitive."""
from __future__ import annotations

import math

from PySide6.QtCore import QLineF, QPointF, QRectF, Qt
from PySide6.QtGui import (QBrush, QColor, QFont, QFontMetricsF, QPainterPath,
                           QPen, QPolygonF, QTransform)


def _hex(c: QColor) -> str:
    return "#%02x%02x%02x%02x" % (c.red(), c.green(), c.blue(), c.alpha())


_STYLES = {Qt.NoPen: "none", Qt.SolidLine: "solid", Qt.DashLine: "dash",
           Qt.DotLine: "dot", Qt.DashDotLine: "dashdot", Qt.CustomDashLine: "custom"}
_CAPS = {Qt.FlatCap: "flat", Qt.SquareCap: "square", Qt.RoundCap: "round"}


class RecordingPainter:
    def __init__(self) -> None:
        self.t = QTransform()
        self.pen_ = QPen(QColor(0, 0, 0))
        self.brush_ = QBrush(Qt.NoBrush)
        self.font_ = QFont()
        self.opacity_ = 1.0
        self.clip_ = None
        self.stack = []
        self.ops = []

    # ---- state ----------------------------------------------------------------
    def save(self):
        self.stack.append((QTransform(self.t), QPen(self.pen_), QBrush(self.brush_),
                           QFont(self.font_), self.opacity_, self.clip_))

    def restore(self):
        self.t, self.pen_, self.brush_, self.font_, self.opacity_, self.clip_ = self.stack.pop()

    def translate(self, *a):
        if len(a) == 1:
            a = (a[0].x(), a[0].y())
        self.t.translate(float(a[0]), float(a[1]))

    def rotate(self, deg):
        self.t.rotate(float(deg))

    def scale(self, sx, sy):
        self.t.scale(float(sx), float(sy))

    def setPen(self, pen):  # noqa: N802
        if isinstance(pen, QPen):
            self.pen_ = QPen(pen)
        elif isinstance(pen, QColor):
            self.pen_ = QPen(pen)
        elif isinstance(pen, Qt.GlobalColor):
            self.pen_ = QPen(QColor(pen))
        else:
            self.pen_ = QPen(pen)

    def pen(self):
        return QPen(self.pen_)

    def setBrush(self, brush):  # noqa: N802
        self.brush_ = QBrush(brush)

    def brush(self):
        return QBrush(self.brush_)

    def setFont(self, font):  # noqa: N802
        self.font_ = QFont(font)

    def font(self):
        return QFont(self.font_)

    def fontMetrics(self):  # noqa: N802
        return QFontMetricsF(self.font_)

    def setRenderHint(self, *a):  # noqa: N802
        pass

    def setRenderHints(self, *a):  # noqa: N802
        pass

    def setOpacity(self, o):  # noqa: N802
        self.opacity_ = float(o)

    def opacity(self):
        return self.opacity_

    def worldTransform(self):  # noqa: N802
        return QTransform(self.t)

    def hasClipping(self):  # noqa: N802
        return self.clip_ is not None

    def clipBoundingRect(self):  # noqa: N802
        if self.clip_ is None:
            return QRectF()
        inv, _ok = self.t.inverted()
        return inv.mapRect(self.clip_)

    # ---- what gets recorded ---------------------------------------------------
    def _scale(self) -> float:
        t = self.t
        return math.sqrt(abs(t.m11() * t.m22() - t.m12() * t.m21()))

    def _pen(self):
        p = self.pen_
        if p.style() == Qt.NoPen:
            return None
        w = p.widthF()
        return {"color": _hex(p.color()), "width": w * self._scale(),
                "style": _STYLES.get(p.style(), "solid"),
                "dash": [d * w * self._scale() for d in p.dashPattern()]
                if p.style() != Qt.SolidLine else [],
                "cap": _CAPS.get(p.capStyle(), "square")}

    def _brush(self, brush=None):
        b = self.brush_ if brush is None else QBrush(brush)
        if b.style() == Qt.NoBrush:
            return None
        return _hex(b.color())

    def _pt(self, p):
        q = self.t.map(QPointF(p))
        return [q.x(), q.y()]

    def _rect(self, r):
        r = QRectF(r)
        return [self._pt(r.topLeft()), self._pt(r.topRight()),
                self._pt(r.bottomRight()), self._pt(r.bottomLeft())]

    def _add(self, op, **kw):
        kw["op"] = op
        kw.setdefault("pen", self._pen())
        kw.setdefault("brush", self._brush())
        if self.opacity_ != 1.0:
            kw["opacity"] = self.opacity_
        self.ops.append(kw)

    @staticmethod
    def _rectf(a):
        if len(a) == 4:
            return QRectF(*map(float, a))
        r = a[0]
        return QRectF(r) if not isinstance(r, QRectF) else r

    def drawLine(self, *a):  # noqa: N802
        if len(a) == 4:
            p, q = QPointF(a[0], a[1]), QPointF(a[2], a[3])
        elif len(a) == 1:
            p, q = a[0].p1(), a[0].p2()
        else:
            p, q = QPointF(a[0]), QPointF(a[1])
        self._add("line", pts=self._pt(p) + self._pt(q), brush=None)

    def drawLines(self, lines):  # noqa: N802
        for ln in lines:
            if isinstance(ln, QLineF):
                self.drawLine(ln)
            else:
                self.drawLine(*ln)

    def drawRect(self, *a):  # noqa: N802
        self._add("poly", pts=self._rect(self._rectf(a)), closed=True)

    def fillRect(self, r, fill):  # noqa: N802
        self._add("poly", pts=self._rect(QRectF(r)), closed=True, pen=None,
                  brush=self._brush(QBrush(fill) if not isinstance(fill, QBrush) else fill))

    def drawRoundedRect(self, r, rx, ry, *a):  # noqa: N802
        s = self._scale()
        self._add("rrect", pts=self._rect(QRectF(r)), rx=float(rx) * s, ry=float(ry) * s)

    def drawEllipse(self, *a):  # noqa: N802
        if len(a) == 3:
            c, rx, ry = QPointF(a[0]), float(a[1]), float(a[2])
        elif len(a) == 1:
            r = QRectF(a[0])
            c, rx, ry = r.center(), r.width() / 2, r.height() / 2
        else:
            r = QRectF(*map(float, a))
            c, rx, ry = r.center(), r.width() / 2, r.height() / 2
        o = self._pt(c)
        ax = self._pt(c + QPointF(rx, 0))
        ay = self._pt(c + QPointF(0, ry))
        self._add("ellipse", center=o, ax=[ax[0] - o[0], ax[1] - o[1]],
                  ay=[ay[0] - o[0], ay[1] - o[1]])

    def _points(self, poly):
        return [self._pt(p) for p in (poly if not isinstance(poly, QPolygonF) else list(poly))]

    def drawPolygon(self, poly, *a):  # noqa: N802
        self._add("poly", pts=self._points(poly), closed=True)

    def drawPolyline(self, poly):  # noqa: N802
        self._add("poly", pts=self._points(poly), closed=False, brush=None)

    def _path(self, path: QPainterPath):
        # Flattened at a hundredth of a millimetre: in mm units Qt would
        # flatten an arc as if each millimetre were a pixel.
        fine = self.t * QTransform.fromScale(100.0, 100.0)
        return [[[p.x() / 100.0, p.y() / 100.0] for p in poly]
                for poly in path.toSubpathPolygons(fine)]

    def drawPath(self, path):  # noqa: N802
        self._add("path", rings=self._path(path))

    def fillPath(self, path, brush):  # noqa: N802
        self._add("path", rings=self._path(path), pen=None, brush=self._brush(brush))

    def drawText(self, *a):  # noqa: N802
        if len(a) == 3:
            rect, flags, text = QRectF(a[0]), int(a[1]), str(a[2])
        else:
            rect, flags, text = QRectF(QPointF(a[0]), QPointF(a[0])), 0, str(a[1])
        f = self.font_
        size = (f.pixelSize() if f.pixelSize() > 0 else f.pointSizeF()) * self._scale()
        self._add("text", rect=self._rect(rect), flags=flags, text=text, size=size,
                  bold=f.bold(), italic=f.italic(), underline=f.underline(),
                  color=_hex(self.pen_.color()), brush=None, pen=None)

    def drawImage(self, r, image, *a):  # noqa: N802
        self._add("image", rect=self._rect(QRectF(r)), pen=None, brush=None)

    def setClipRect(self, r, *a):  # noqa: N802
        rr = QRectF(r)
        self.clip_ = self.t.mapRect(rr)
        self._add("clip", rect=self._rect(rr), pen=None, brush=None)

    def setClipPath(self, path, *a):  # noqa: N802
        self.clip_ = self.t.map(path).boundingRect()
        self._add("clip", rings=self._path(path), pen=None, brush=None)
