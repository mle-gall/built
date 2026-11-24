# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Shared cell, column and resize handling for the remaining Classic trees."""
from __future__ import annotations

from PySide6.QtCore import QEvent, QPoint, Qt, QTimer
from PySide6.QtGui import QPainter, QPalette
from PySide6.QtWidgets import (QAbstractItemView, QApplication, QMenu, QStyle,
                               QStyleOptionHeader, QToolButton, QTreeWidget,
                               QWidget)


class _HeaderButton(QToolButton):
    """The columns button, in a strip of its own at the tree's right edge,
    level with the header: the columns scroll, stretch and move under the
    header beside it, and it stays put. Drawn as a header section — the
    same face as the columns, after the header's own last line — so it
    reads as part of the header, not as a stray number on the first row."""

    def paintEvent(self, _ev) -> None:          # noqa: N802 — Qt override
        opt = QStyleOptionHeader()
        opt.initFrom(self)
        opt.rect = self.rect()
        opt.orientation = Qt.Horizontal
        opt.position = QStyleOptionHeader.End
        # «+3» stays «+3» in a right-to-left interface (#1067).
        from views.rtl import isolate_ltr
        opt.text = isolate_ltr(self.text(), self)
        opt.textAlignment = Qt.AlignCenter
        if self.isDown():
            opt.state |= QStyle.State_Sunken
        else:
            opt.state |= QStyle.State_Raised
        if self.underMouse():
            opt.state |= QStyle.State_MouseOver
        p = QPainter(self)
        p.setFont(self.font())
        self.style().drawControl(QStyle.CE_Header, opt, p, self)
        # The line before it. The header's last column draws none on its
        # right (the style leaves the end of a header open), so the style
        # draws it here: a section beside this one, its right edge — the
        # separator every other column has — 1 px inside this button, the
        # rest clipped away. 2 px of clip: at 125 % display scaling one
        # logical pixel falls between device pixels and the line vanished.
        r = self.rect()
        edge = QStyleOptionHeader()
        edge.initFrom(self)
        edge.orientation = Qt.Horizontal
        edge.position = QStyleOptionHeader.Middle
        edge.state |= QStyle.State_Raised
        edge.rect = r.translated(-r.width() + 2, 0)
        p.setClipRect(0, 0, 2, r.height())
        self.style().drawControl(QStyle.CE_HeaderSection, edge, p, self)
        p.end()


class StayOpenMenu(QMenu):
    """A menu of switches that stays open while they are flipped — several
    columns on or off in one go — and closes on a click outside or Esc.
    An entry with the ``keep_open`` property set stays open too; one with
    the ``close`` property closes the menu, as a menu does by default."""

    @staticmethod
    def _stays(action) -> bool:
        return (action is not None and action.isEnabled()
                and not action.property("close")
                and (action.isCheckable()
                     or bool(action.property("keep_open"))))

    def mouseReleaseEvent(self, event) -> None:
        action = self.actionAt(event.position().toPoint())
        if self._stays(action):
            action.trigger()
            event.accept()
            return
        super().mouseReleaseEvent(event)

    def keyPressEvent(self, event) -> None:
        action = self.activeAction()
        if (event.key() in (Qt.Key_Return, Qt.Key_Enter, Qt.Key_Space)
                and self._stays(action)):
            action.trigger()
            event.accept()
            return
        super().keyPressEvent(event)


class _StripFiller(QWidget):
    """The strip kept at the tree's right for the columns button, below the
    button: painted as the rows beside it — their tone, alternate or not,
    and the selection's — so it reads as their end, not as a lighter band
    (the frame behind it shows the window's colour)."""

    def __init__(self, tree) -> None:
        super().__init__(tree)
        self._tree = tree

    def paintEvent(self, _ev) -> None:          # noqa: N802 — Qt override
        tree = self._tree
        pal = tree.palette()
        p = QPainter(self)
        p.fillRect(self.rect(), pal.color(QPalette.Base))
        vp = tree.viewport()
        y = 0
        odd = None
        while y < vp.height():
            index = tree.indexAt(QPoint(1, y))
            if not index.isValid():
                break
            rect = tree.visualRect(index)
            if rect.height() <= 0:
                break
            # The first row's parity once, then every row flips it.
            odd = tree.row_parity(index) if odd is None else not odd
            item = tree.itemFromIndex(index)
            if item is not None and item.isSelected():
                colour = pal.color(QPalette.Highlight)
            elif tree.alternatingRowColors() and odd:
                colour = pal.color(QPalette.AlternateBase)
            else:
                colour = pal.color(QPalette.Base)
            p.fillRect(0, rect.top(), self.width(), rect.height(), colour)
            y = rect.bottom() + 1
        p.end()


class SideTree(QTreeWidget):
    """A cell control acts without replacing the row selection."""

    #: The cells a press acts on directly, without selecting their row.
    CELLS: tuple = ()

    def __init__(self, panel) -> None:
        super().__init__()
        self._panel = panel
        #: Width of the strip kept free at the right for the columns button.
        self.button_strip = 0
        self._filler = _StripFiller(self)
        self._filler.hide()
        # It repaints with the rows: scrolled, expanded, selected.
        self.viewport().installEventFilter(self)
        #: A press on one of :attr:`CELLS` is the panel's until the button
        #: is let go: Qt never saw it, so a move with the button held would
        #: draw a selection box from wherever Qt's last press was — a few
        #: pixels of wobble while clicking an eye selected a run of rows.
        self._cell_press = False

    def resizeEvent(self, event) -> None:
        # Cocoa backing-store flushes plus row delegates can exceed a frame
        # during a dock drag, even with uniform row heights. Keep the existing
        # rows until the pane settles; the dock splitter still moves.
        if (QApplication.platformName() == "cocoa" and self.isVisible()
                and event.oldSize().isValid()):
            timer = getattr(self, "_resize_paint_timer", None)
            if timer is None:
                timer = self._resize_paint_timer = QTimer(self, singleShot=True,
                                                        interval=80)
                timer.timeout.connect(self._finish_resize_paint)
            if not timer.isActive():
                self._resize_updates_enabled = self.updatesEnabled()
                self.setUpdatesEnabled(False)
            timer.start()
        super().resizeEvent(event)

    def _finish_resize_paint(self) -> None:
        if self._resize_updates_enabled:
            self.setUpdatesEnabled(True)

    # ---- The strip of the columns button ---------------------------------
    def updateGeometries(self) -> None:          # noqa: N802 — Qt override
        """Qt lays the header over the whole viewport; a strip is then kept
        at the right for the columns button, header and rows one strip
        narrower."""
        super().updateGeometries()
        m = self.viewportMargins()
        if m.right() != self.button_strip:
            self.setViewportMargins(m.left(), m.top(), self.button_strip,
                                    m.bottom())
        vg = self.viewport().geometry()
        head = self.header()
        if not head.isHidden():
            head.setGeometry(vg.left(), vg.top() - m.top(), vg.width(),
                             m.top())
        b = self._columns_button()
        if b is not None:
            g = head.geometry()
            b.setGeometry(g.right() + 1, g.top(), b.width(), g.height())
        self._filler.setGeometry(vg.right() + 1, vg.top(), self.button_strip,
                                 vg.height())
        self._filler.setVisible(self.button_strip > 0)

    def _columns_button(self):
        """The panel's columns button: the Browser and Layers name it
        differently (it grew from each)."""
        b = getattr(self._panel, "columns_button", None)
        return b if b is not None else getattr(
            self._panel, "_columns_button", None)

    def row_parity(self, index) -> bool:
        """Whether the row of ``index`` is one the tree paints in the
        alternate tone: every other row shown, counted from the top. For
        the first row on screen, scrolling by rows already counts them."""
        if (self.verticalScrollMode() == QAbstractItemView.ScrollPerItem
                and index == self.indexAt(self.viewport().rect().topLeft())):
            return bool(self.verticalScrollBar().value() % 2)
        n = 0
        above = self.indexAbove(index)
        while above.isValid():
            n += 1
            above = self.indexAbove(above)
        return bool(n % 2)

    def eventFilter(self, obj, ev) -> bool:
        if obj is self.viewport() and ev.type() == QEvent.Paint:
            self._filler.update()
        return super().eventFilter(obj, ev)

    # ---- The cells that act at once --------------------------------------
    def _cell_click(self, event) -> bool:
        """Act on a plain left click on one of :attr:`CELLS`; whether it
        was one (and so taken)."""
        if (event.button() != Qt.LeftButton
                or event.modifiers() & (Qt.ControlModifier
                                        | Qt.ShiftModifier)):
            return False
        pos = event.position().toPoint()
        item = self.itemAt(pos)
        col = self.columnAt(pos.x())
        if item is None or col not in self.CELLS:
            return False
        event.accept()
        self._cell_press = True
        self.cell_clicked(item, col)
        return True

    def cell_clicked(self, item, column) -> None:
        """A press on one of :attr:`CELLS`: the panel answers it. The panel
        may defer the action and keep only what it read here — a press that
        rebuilds the rows would leave the item behind."""
        self._panel.cell_clicked(item, column)

    def mousePressEvent(self, event) -> None:    # noqa: N802 — Qt override
        if not self._cell_click(event):
            super().mousePressEvent(event)

    def mouseDoubleClickEvent(self, event) -> None:  # noqa: N802 — Qt override
        # Two quick clicks on a cell are two clicks (off and on again), not
        # a double click for the list to select or rename the row with.
        if not self._cell_click(event):
            super().mouseDoubleClickEvent(event)

    def mouseMoveEvent(self, event) -> None:         # noqa: N802 — Qt override
        if self._cell_press:
            event.accept()
            return
        super().mouseMoveEvent(event)

    def mouseReleaseEvent(self, event) -> None:      # noqa: N802 — Qt override
        if self._cell_press:
            self._cell_press = False
            event.accept()
            return
        super().mouseReleaseEvent(event)

    # ---- Delete ----------------------------------------------------------
