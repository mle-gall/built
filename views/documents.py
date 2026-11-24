# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Several projects open in one window, each in its own tab (#1066).

A :class:`Document` is one open project. It owns a whole
:class:`views.viewport.Viewport` — the scene, its undo history, the camera,
every drawing cache and the GL textures live there already — plus what the
main window used to keep for its single document: the file, the name of an
import, the version last saved and auto-saved, the auto-save slot of an
untitled drawing, and the sheet composer.

The window keeps the viewports in a stack under a :class:`ProjectTabBar`
and its ``viewport`` is a property that answers the ACTIVE document's. The
hundreds of places that read ``window.viewport`` — the window, the panels,
the extensions — follow the active tab without knowing tabs exist; a tab in
the background is a hidden widget that Qt neither paints nor updates, and
coming back to it rebuilds nothing.
"""
from __future__ import annotations

from PySide6.QtCore import QEvent, QPoint, Qt, QTimer, Signal
from PySide6.QtWidgets import (QHBoxLayout, QSizePolicy, QTabBar, QToolButton,
                               QWidget)

from core.i18n import tr


from core.document_host import Document, DocumentHost


def live_viewport(viewport):
    """A callable that answers the viewport of the project in front of
    ``viewport``'s window — what a panel built with the viewport of its day
    keeps, so it acts on whichever project the user is looking at. It keeps
    the WINDOW, never that viewport: a panel holding a viewport would hold
    its whole closed project in memory. A viewport outside a tabbed window
    (tests, a lone widget) is answered as it is."""
    try:
        win = viewport.window()
    except (AttributeError, RuntimeError):
        win = None
    if win is not None and hasattr(win, "documents"):
        return lambda: win.viewport
    return lambda: viewport


#: The «+» after a row of tabs: its icon (px). The project tabs and the
#: scene tabs under the viewport share the same «+».
PLUS_ICON = 16


def size_plus_button(plus, tabs) -> None:
    """``plus`` as tall as a tab of ``tabs`` and a little wider than tall —
    the same «+» after the project tabs and after the scene tabs."""
    h = tabs.sizeHint().height() if tabs.count() else 0
    if h <= 0:
        probe = QTabBar()
        probe.setFont(tabs.font())
        probe.addTab("Xg")
        h = probe.sizeHint().height()
    if plus.height() != h or plus.width() != max(30, h + 4):
        plus.setFixedSize(max(30, h + 4), h)


_probe_heights: dict = {}


def panel_tab_height(bar) -> int:
    """The text tab height for the current font and style."""
    key = (bar.font().key(), bar.style().name())
    h = _probe_heights.get(key)
    if h is None:
        probe = QTabBar()
        probe.setFont(bar.font())
        probe.setStyle(bar.style())
        probe.addTab("Xg")
        h = _probe_heights[key] = probe.tabSizeHint(0).height()
        probe.deleteLater()
    return h


class _ProjectTabs(QTabBar):
    """The project tabs, drawn as the side panels' tabs are (the same
    QTabBar and style, the same height), with a line in the accent colour
    under the active project's name; as wide as a name and its cross need,
    and the cross bigger and against the tab's right edge."""

    #: The accent line: thickness and gap above the tab's foot (px).
    LINE_PX = 2
    LINE_GAP = 3
    #: The close cross: its button, the icon in it, its gap from the edge.
    CLOSE_PX = 16
    CLOSE_ICON = 13
    CLOSE_EDGE = 4
    #: Room around a name: left, between name and cross, and right (px).
    PAD_LEFT, PAD_GAP, PAD_RIGHT = 10, 4, 4

    def tabSizeHint(self, index: int):  # noqa: N802 — Qt override
        hint = super().tabSizeHint(index)
        height = self._panel_tab_height()
        # Qt asks for every tab many times over a single layout.
        name = self.tabText(index)
        key = (name, hint.width(), self.font().key(), self.style().name())
        width = self._hint_widths.get(key)
        if width is None:
            width = self._hint_widths[key] = self._tab_width(index, hint,
                                                             height)
        hint.setWidth(width)
        hint.setHeight(height)
        return hint

    def _tab_width(self, index: int, hint, height: int) -> int:
        fm = self.fontMetrics()
        name = self.tabText(index)
        width = (self.PAD_LEFT + fm.horizontalAdvance(name) + self.PAD_GAP
                 + self.CLOSE_PX + self.PAD_RIGHT)
        # The style keeps margins of its own around the name: widen until
        # the whole name fits, measured as the selected tab (the narrower
        # text area), so the size does not change when it comes to front.
        from PySide6.QtCore import QRect
        from PySide6.QtWidgets import QStyle, QStyleOptionTab
        opt = QStyleOptionTab()
        self.initStyleOption(opt, index)
        opt.state |= QStyle.State_Selected
        opt.position = QStyleOptionTab.Middle
        need = fm.horizontalAdvance(name) + 2
        while width < hint.width():
            opt.rect = QRect(0, 0, width, height)
            if self.style().subElementRect(
                    QStyle.SE_TabBarTabText, opt, self).width() >= need:
                break
            width += 2
        return min(hint.width(), width)

    _hint_widths: dict = {}

    def _panel_tab_height(self) -> int:
        return panel_tab_height(self)

    def sizeHint(self):  # noqa: N802 — Qt override
        """Exactly as wide as its tabs: Qt's hint keeps room for the
        scroll arrows, which left «+» a gap away from the last tab.

        As tall as a tab from the start, tabs or not. The bar is built
        empty (Qt's hint: 0 high) and its first tab comes while the window
        is still hidden, when Qt asks for no new size. So it opened at the
        row's height and grew to a tab's on a later layout, a moment after
        the window showed: with a font whose tab is taller than that row
        (macOS), the accent line under the name was cut off until then,
        and the views under the bar shrank by the difference."""
        hint = super().sizeHint()
        if self.count():
            hint.setWidth(sum(self.tabSizeHint(i).width()
                              for i in range(self.count())))
        hint.setHeight(max(hint.height(), self._panel_tab_height()))
        return hint

    def minimumSizeHint(self):  # noqa: N802 — Qt override
        hint = super().minimumSizeHint()
        hint.setWidth(min(hint.width(), self.sizeHint().width()))
        return hint

    # ---- the cross ---------------------------------------------------------
    def tabInserted(self, index: int) -> None:  # noqa: N802 — Qt override
        super().tabInserted(index)
        from PySide6.QtCore import QSize
        from views.icons import tool_icon
        btn = QToolButton(self)
        btn.setObjectName("project_tab_close")
        btn.setProperty("icon_key", "close")
        btn.setIcon(tool_icon("close"))
        btn.setIconSize(QSize(self.CLOSE_ICON, self.CLOSE_ICON))
        btn.setFixedSize(self.CLOSE_PX, self.CLOSE_PX)
        btn.setAutoRaise(True)
        btn.setFocusPolicy(Qt.NoFocus)
        btn.setCursor(Qt.PointingHandCursor)
        btn.setToolTip(tr("Close Project") + " (Ctrl+W)")
        btn.setAccessibleName(tr("Close Project"))
        btn.clicked.connect(lambda _c=False, b=btn: self._close_clicked(b))
        self.setTabButton(index, QTabBar.RightSide, btn)

    def _close_clicked(self, btn) -> None:
        for i in range(self.count()):
            if self.tabButton(i, QTabBar.RightSide) is btn:
                self.tabCloseRequested.emit(i)
                return

    def tabLayoutChange(self) -> None:  # noqa: N802 — Qt override
        super().tabLayoutChange()
        self._seat_close_buttons()

    def resizeEvent(self, ev) -> None:  # noqa: N802 — Qt override
        super().resizeEvent(ev)
        self._seat_close_buttons()

    def _seat_close_buttons(self) -> None:
        """Against the tab's far edge (the left one, right to left), centred
        on its height (the style lays it out with a margin of its own)."""
        for i in range(self.count()):
            btn = self.tabButton(i, QTabBar.RightSide)
            if btn is None:
                continue
            r = self.tabRect(i)
            x = (r.left() + self.CLOSE_EDGE if self.isRightToLeft()
                 else r.right() - self.CLOSE_EDGE - btn.width() + 1)
            want = QPoint(x, r.top() + (r.height() - btn.height()) // 2)
            if btn.pos() != want:
                btn.move(want)

    # ---- the accent line ---------------------------------------------------
    def underline_rect(self):
        from PySide6.QtCore import QRect
        from PySide6.QtWidgets import QStyle, QStyleOptionTab
        i = self.currentIndex()
        if i < 0 or not self.tabText(i):
            return None
        opt = QStyleOptionTab()
        self.initStyleOption(opt, i)
        text = self.style().subElementRect(QStyle.SE_TabBarTabText, opt, self)
        width = min(self.fontMetrics().horizontalAdvance(opt.text),
                    text.width())
        tab = self.tabRect(i)
        y = tab.bottom() - self.LINE_GAP - self.LINE_PX + 1
        return QRect(text.center().x() - width // 2, y, width, self.LINE_PX)

    def paintEvent(self, ev) -> None:  # noqa: N802 — Qt override
        # Qt seats the crosses again on its own (a click on a tab did, and
        # the cross jumped back to the style's place): every frame, before
        # drawing, they go back against the edge.
        self._seat_close_buttons()
        super().paintEvent(ev)
        rect = self.underline_rect()
        if rect is None:
            return
        from PySide6.QtGui import QPainter
        from views.theme import accent_color
        p = QPainter(self)
        p.fillRect(rect, accent_color())
        p.end()


class ProjectTabBar(QWidget):
    """The project tabs above the viewport: the file name, a dot while it
    has unsaved changes, a × to close; «+» makes a new project; a middle
    click closes; tabs are reordered by dragging them.

    Signals carry the :class:`Document` (each tab's data)."""

    activated = Signal(object)
    closeRequested = Signal(object)
    newRequested = Signal()
    moved = Signal()
    #: The views button: the layouts of the project in front.

    #: The mark of a project with unsaved changes, after its name.
    DIRTY_MARK = " •"

    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        self.setObjectName("project_tabs")
        row = QHBoxLayout(self)
        row.setContentsMargins(4, 2, 0, 0)
        row.setSpacing(0)
        from PySide6.QtCore import QSize
        from views.icons import tool_icon
        bar = self.tabs = _ProjectTabs(self)
        bar.setObjectName("project_tab_bar")
        # Drawn as the side panels' tabs (no document mode, no style sheet
        # of its own) with the accent line under the active name.
        bar.setDocumentMode(False)
        bar.setExpanding(False)
        bar.setMovable(True)
        bar.setTabsClosable(False)          # our own, bigger cross
        bar.setUsesScrollButtons(True)
        bar.setElideMode(Qt.ElideMiddle)
        bar.setDrawBase(False)
        bar.setSelectionBehaviorOnRemove(QTabBar.SelectPreviousTab)
        bar.setSizePolicy(QSizePolicy.Preferred, QSizePolicy.Fixed)
        bar.tabCloseRequested.connect(self._on_close_index)
        bar.currentChanged.connect(self._on_current)
        bar.tabMoved.connect(lambda *_a: self.moved.emit())
        bar.installEventFilter(self)
        row.addWidget(bar, 0, Qt.AlignBottom)
        # «+» right after the last tab, as tall as a tab; drawn, so it is
        # centred to the pixel (a font's «+» sits off centre).
        plus = self.plus = QToolButton(self)
        plus.setObjectName("project_tab_new")
        plus.setProperty("icon_key", "plus")
        plus.setIcon(tool_icon("plus"))
        plus.setIconSize(QSize(PLUS_ICON, PLUS_ICON))
        plus.setToolButtonStyle(Qt.ToolButtonIconOnly)
        plus.setAutoRaise(True)
        plus.setToolTip(tr("New project (Ctrl+N)"))
        plus.setCursor(Qt.PointingHandCursor)
        plus.clicked.connect(self.newRequested.emit)
        row.addWidget(plus, 0, Qt.AlignBottom)
        row.addStretch(1)
        self._quiet = False
        from views import mac_glass
        from views.theme import style as theme_style
        if mac_glass.enabled():
            # On glass (macOS 26+): the tabs as rounded pills, like
            # Safari's and Finder's, the strip see-through.
            theme_style(self,
                "#project_tabs {{ background: transparent; }}"
                "QTabBar#project_tab_bar::tab {{ padding: 3px 12px;"
                " max-width: 260px; border: none; border-radius: 7px;"
                " margin: 3px 2px; background: transparent; }}"
                "QTabBar#project_tab_bar::tab:selected {{"
                " background: rgba({ink_rgb}, 36); }}"
                "QTabBar#project_tab_bar::tab:!selected:hover {{"
                " background: rgba({ink_rgb}, 16); }}"
                "QTabBar#project_tab_bar::close-button {{"
                " image: url({tab_close}); border-radius: 4px;"
                " subcontrol-position: right; margin: 2px; }}"
                "QTabBar#project_tab_bar::close-button:hover {{"
                " background: rgba({ink_rgb}, 40); }}"
                "QToolButton#project_tab_new {{ padding: 0px 8px;"
                " font-weight: bold; border: none; border-radius: 7px; }}"
                "QToolButton#project_tab_new:hover {{"
                " background: rgba({ink_rgb}, 16); }}")
            return
        theme_style(self,
            "#project_tabs {{ background: palette(window); }}"
            "QToolButton#project_tab_close {{ border: none;"
            " border-radius: 4px; background: transparent; padding: 0px; }}"
            "QToolButton#project_tab_close:hover {{"
            " background: rgba({accent_rgb}, 66); }}"
            "QToolButton#project_tab_new {{"
            " border: none; border-radius: 6px; background: transparent;"
            " padding: 0px; }}"
            "QToolButton#project_tab_new:hover {{"
            " background: rgba({accent_rgb}, 36); }}")

    def showEvent(self, ev) -> None:  # noqa: N802 — Qt override
        super().showEvent(ev)
        self.match_plus_to_tabs()

    def match_plus_to_tabs(self) -> None:
        """«+» as tall as a tab (a little wider than tall)."""
        size_plus_button(self.plus, self.tabs)

    # ---- the tabs ---------------------------------------------------------
    def count(self) -> int:
        return self.tabs.count()

    def documents(self) -> list:
        return [self.tabs.tabData(i) for i in range(self.tabs.count())]

    def index_of(self, doc) -> int:
        for i in range(self.tabs.count()):
            if self.tabs.tabData(i) is doc:
                return i
        return -1

    def add(self, doc, index: int = -1) -> int:
        self._quiet = True
        try:
            if index < 0:
                i = self.tabs.addTab("")
            else:
                i = self.tabs.insertTab(index, "")
            self.tabs.setTabData(i, doc)
        finally:
            self._quiet = False
        return i

    def remove(self, doc) -> None:
        i = self.index_of(doc)
        if i < 0:
            return
        self._quiet = True
        try:
            self.tabs.removeTab(i)
        finally:
            self._quiet = False

    def set_current(self, doc) -> None:
        i = self.index_of(doc)
        if i >= 0 and i != self.tabs.currentIndex():
            self._quiet = True
            try:
                self.tabs.setCurrentIndex(i)
            finally:
                self._quiet = False

    def current(self):
        i = self.tabs.currentIndex()
        return self.tabs.tabData(i) if i >= 0 else None

    def set_label(self, doc, name: str, dirty: bool, tip: str = "") -> None:
        i = self.index_of(doc)
        if i < 0:
            return
        text = name + (self.DIRTY_MARK if dirty else "")
        if self.tabs.tabText(i) != text:
            self.tabs.setTabText(i, text)
        if self.tabs.tabToolTip(i) != tip:
            self.tabs.setTabToolTip(i, tip)

    def neighbour(self, doc, step: int):
        """The document ``step`` tabs away from ``doc``, wrapping round."""
        n = self.tabs.count()
        i = self.index_of(doc)
        if n == 0 or i < 0:
            return None
        return self.tabs.tabData((i + step) % n)

    # ---- user input -------------------------------------------------------
    def _on_current(self, index: int) -> None:
        if self._quiet or index < 0:
            return
        doc = self.tabs.tabData(index)
        # After the press: QTabBar is still handling it when the signal
        # returns, and the window rebuilds things under the pointer.
        QTimer.singleShot(0, self, lambda d=doc: self.activated.emit(d))

    def _on_close_index(self, index: int) -> None:
        doc = self.tabs.tabData(index)
        if doc is not None:
            QTimer.singleShot(0, self, lambda d=doc: self.closeRequested.emit(d))

    def eventFilter(self, obj, event):  # noqa: N802 — Qt override
        if obj is self.tabs:
            t = event.type()
            if (t == QEvent.MouseButtonRelease
                    and event.button() == Qt.MiddleButton):
                i = self.tabs.tabAt(event.position().toPoint())
                if i >= 0:
                    self._on_close_index(i)
                    return True
            elif (t == QEvent.MouseButtonDblClick
                  and event.button() == Qt.LeftButton
                  and self.tabs.tabAt(event.position().toPoint()) < 0):
                self.newRequested.emit()       # the empty strip: a new tab
                return True
        return super().eventFilter(obj, event)

    def tab_rect_global(self, doc):
        i = self.index_of(doc)
        if i < 0:
            return None
        r = self.tabs.tabRect(i)
        return r.translated(self.tabs.mapToGlobal(QPoint(0, 0)))
