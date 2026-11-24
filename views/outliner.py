# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Browser: the model's tree, and the composer's views and sheets.

**The Browser** (a sidebar tab of the model window) is the Outliner
(#219): every group and component at every depth, under the document's
name. An icon says what each row is (group, component, image, 3D text, a
BIM-tagged element, or what an extension says it is); the eye shows or
hides it, the padlock locks it; the filter keeps the rows whose name, type,
BIM class or layer match, with the containers on the way to them. A click
selects in the view — opening the groups around it, as double-clicking down
to it would — and a double-click (or Enter) opens it for editing. A
selection made in the view is found in the tree, its branch unfolded, and
the groups open for editing read in bold, so the tree always says where you
are. Below the groups, the section planes, each with its cut, frame and
fill.

The Outliner is built on the Project Structure plugin Carlos Martins
(@fafecm) shared in #87 — the tree mirrored with the view both ways, the
eye per node, entering a nested group through its path — and on the scope
Roy Méndez Castro (@roymendezcastro) wrote down in #219. Its drag and
drop and its padlock are here too: re-nesting keeps every group where it
stands in the world, whatever frame the new parent has, and the lock is
saved in the document.

The model stays the only source of truth: rows hold their group (or scene,
sheet, frame) and renames and hides are the usual undoable commands.
"""
from __future__ import annotations

from PySide6.QtCore import QItemSelectionModel, QPointF, QRectF, QSize, Qt
from PySide6.QtGui import (QColor, QFont, QIcon, QPainter, QPalette, QPen,
                           QPixmap)
from PySide6.QtWidgets import (QAbstractItemView, QApplication, QDialog,
                               QDockWidget,
                               QHBoxLayout,
                               QHeaderView, QLabel, QLineEdit, QStyle,
                               QToolButton, QTreeWidget,
                               QTreeWidgetItem, QVBoxLayout, QWidget)

from core import outliner
from core.group import Group
from core.i18n import tr
from core.section import SectionPlane
from views.tree_navigation import open_to
from views.tree_icons import isolate_icon, row_icon
from views.side_tree import SideTree, StayOpenMenu, _HeaderButton

# Model tab columns.
(COL_NAME, COL_EYE, COL_LOCK, COL_TYPE, COL_LAYER, COL_MATERIAL, COL_COPIES,
 COL_IFC) = range(8)
#: The isolate target. It sits after the padlock on screen (COLUMN_ORDER)
#: but is numbered last, so the columns before it and the rows' state
#: tuples keep the order they had.
COL_ISOLATE = 8
#: Columns the header's right-click shows or hides, and which start shown.
OPTIONAL_COLUMNS = ((COL_TYPE, "Type", True), (COL_LAYER, "Layer", True),
                    (COL_MATERIAL, "Material", False),
                    (COL_COPIES, "Copies", False),
                    (COL_IFC, "IFC class", False))


def _paint_icon(paint: dict, size: int = 16) -> QIcon:
    """A small swatch of a paint for a menu: the texture's image when it
    can be read, its colour otherwise."""
    pix = QPixmap(size, size)
    tex = paint.get("texture") or {}
    path = tex.get("path")
    if path:
        img = QPixmap(str(path))
        if not img.isNull():
            return QIcon(img.scaled(size, size))
    col = paint.get("color") or tex.get("tint") or (0.6, 0.6, 0.6)
    pix.fill(QColor.fromRgbF(*[max(0.0, min(1.0, float(c)))
                               for c in list(col)[:3]]))
    return QIcon(pix)


# Keys the tree answers itself while it has the focus, instead of the
# window's single-key shortcuts (Space is Select, Esc cancels a tool…).
_TREE_KEYS = {Qt.Key_Space, Qt.Key_Return, Qt.Key_Enter, Qt.Key_Escape,
              Qt.Key_F2, Qt.Key_Up, Qt.Key_Down, Qt.Key_Left, Qt.Key_Right,
              Qt.Key_Home, Qt.Key_End, Qt.Key_PageUp, Qt.Key_PageDown}


# ---- Icons ---------------------------------------------------------------------
# Drawn, like the tool icons (views/icons.py), in the palette's own ink so
# they read in the light and the dark theme alike.

_ICONS: dict = {}


def _ink() -> QColor:
    from PySide6.QtWidgets import QApplication
    return QApplication.palette().text().color()


def _accent() -> QColor:
    from PySide6.QtWidgets import QApplication
    return QApplication.palette().highlight().color()


def _cube(p, ink, fill=None) -> None:
    """A small box in cavalier view: the group's mark."""
    front = QRectF(2.5, 6.5, 9, 8)
    if fill is not None:
        p.setBrush(fill)
    p.drawPolygon([QPointF(2.5, 6.5), QPointF(6.5, 2.5), QPointF(15.5, 2.5),
                   QPointF(15.5, 10.5), QPointF(11.5, 14.5),
                   QPointF(2.5, 14.5)])
    p.setBrush(Qt.NoBrush)
    p.drawRect(front)
    p.drawLine(QPointF(11.5, 6.5), QPointF(15.5, 2.5))


def _draw(key: str):
    ink = _ink()
    pm = QPixmap(18, 18)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    p.setPen(QPen(ink, 1.2))
    if key == "group":
        _cube(p, ink)
    elif key == "component":
        # A component is a box FILLED: the definition its copies share.
        fill = QColor(_accent())
        fill.setAlpha(110)
        _cube(p, ink, fill)
    elif key == "ifc":
        _cube(p, ink)
        p.setPen(Qt.NoPen)
        p.setBrush(_accent())
        p.drawEllipse(QRectF(10, 10, 7.5, 7.5))
    elif key == "rename":
        # A pencil, point down-left, over the line it writes.
        p.drawPolygon([QPointF(4.7, 11.0), QPointF(12.9, 2.9),
                       QPointF(15.1, 5.1), QPointF(7.0, 13.3)])
        p.drawPolygon([QPointF(4.7, 11.0), QPointF(7.0, 13.3),
                       QPointF(2.6, 15.4)])
        p.drawLine(QPointF(11.1, 4.6), QPointF(13.4, 6.9))
        p.drawLine(QPointF(9.5, 16.2), QPointF(16.2, 16.2))
    elif key in ("isolate", "isolate_on"):
        # Drawn, not the ◎ / ◉ characters: few interface fonts have them,
        # and looking for one that does is where the 0.5.8 Flatpak died at
        # start-up on a machine with a broken user font (#658).
        p.setBrush(Qt.NoBrush)
        p.drawEllipse(QRectF(2.5, 2.5, 13, 13))
        if key == "isolate_on":
            p.setPen(Qt.NoPen)
            p.setBrush(_accent())
            p.drawEllipse(QRectF(5.5, 5.5, 7, 7))
        else:
            p.drawEllipse(QRectF(6, 6, 6, 6))
    elif key in ("expand_all", "collapse_all"):
        # ⊞ / ⊟, drawn for the same reason as Isolate's (#658).
        p.setBrush(Qt.NoBrush)
        p.drawRect(QRectF(2.5, 2.5, 13, 13))
        p.drawLine(QPointF(5.5, 9), QPointF(12.5, 9))
        if key == "expand_all":
            p.drawLine(QPointF(9, 5.5), QPointF(9, 12.5))
    elif key == "document":
        p.drawPolygon([QPointF(2, 9), QPointF(9, 3), QPointF(16, 9)])
        p.drawRect(QRectF(4, 9, 10, 7))
    elif key == "sheet":
        p.drawPolygon([QPointF(3.5, 1.5), QPointF(11.5, 1.5),
                       QPointF(14.5, 4.5), QPointF(14.5, 16.5),
                       QPointF(3.5, 16.5)])
        p.drawLine(QPointF(11.5, 1.5), QPointF(11.5, 4.5))
        p.drawLine(QPointF(11.5, 4.5), QPointF(14.5, 4.5))
        p.drawLine(QPointF(6, 12), QPointF(12, 12))
        p.drawLine(QPointF(6, 14), QPointF(10, 14))
    elif key == "frame":
        p.drawRect(QRectF(2.5, 4.5, 13, 10))
        p.drawPolyline([QPointF(4, 13), QPointF(8, 8), QPointF(11, 11),
                        QPointF(14, 7)])
    elif key in ("cut", "cut_off"):
        # A box sliced: the cut half filled when the plane cuts.
        if key == "cut":
            fill = QColor(_accent())
            fill.setAlpha(160)
            p.setPen(Qt.NoPen)
            p.setBrush(fill)
            p.drawPolygon([QPointF(3, 3), QPointF(15, 3), QPointF(3, 15)])
            p.setBrush(Qt.NoBrush)
            p.setPen(QPen(ink, 1.2))
        else:
            faint = QColor(ink)
            faint.setAlpha(110)
            p.setPen(QPen(faint, 1.2))
        p.drawRect(QRectF(3, 3, 12, 12))
        p.drawLine(QPointF(1.5, 16.5), QPointF(16.5, 1.5))
    p.end()
    return QIcon(pm)


#: The size every mark of a row is drawn at (eye, padlock, target): the
#: same in the Browser and in the Layers tab.
MARK_SIZE = 18


def _faded(icon: QIcon, size: int = MARK_SIZE) -> QIcon:
    """``icon`` at 40 %: the eye or padlock a row inherits from a group
    around it — closed, but not its own — at the row's own size, only
    fainter (16 px made an inherited mark smaller than its neighbours)."""
    src = icon.pixmap(size, size)
    pm = QPixmap(src.size())
    pm.setDevicePixelRatio(src.devicePixelRatio())
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setOpacity(0.4)
    p.drawPixmap(0, 0, src)
    p.end()
    return QIcon(pm)


def mark_icon(key: str, inherited: bool = False) -> QIcon:
    """A row's mark — ``eye``, ``eye_off``, ``lock``, ``lock_open``,
    ``isolate``… — as the two side trees draw it: same drawing, same size,
    and faded when the state comes from the group around it."""
    icon = node_icon(key)
    return _faded(icon) if inherited else icon


def node_icon(key: str) -> QIcon:
    """The icon for a row kind — ``group``, ``component``, ``ifc``,
    ``image``, ``text``, ``document``, ``scene``, ``sheet``, ``frame``,
    ``eye``, ``eye_off``, ``warning``.

    Kept once per theme: they are painted in the palette's ink and accent,
    and a theme switched with the app open must not get the old ones back
    (#1086)."""
    from PySide6.QtWidgets import QApplication
    # The palette's cache key, not its colours: the Browser asks three times
    # per row, and reading the colours cost a 20 000-row refresh 0.2 s.
    slot = (key, QApplication.palette().cacheKey())
    if slot not in _ICONS:
        if key.startswith("layers_"):
            # The Layers bar's icons (views/icons.py): the two trees draw
            # the same buttons with the same set.
            from views.icons import tool_icon
            _ICONS[slot] = tool_icon(key)
        elif key in ("image", "text", "sections"):
            from views.icons import tool_icon
            _ICONS[slot] = tool_icon({"image": "image", "text": "text3d",
                                      "sections": "section_planes"}[key])
        elif key == "warning":
            _ICONS[slot] = QApplication.style().standardIcon(
                QStyle.SP_MessageBoxWarning)
        elif key in ("folder", "scene", "eye", "eye_off", "lock", "lock_open"):
            _ICONS[slot] = row_icon(key)
        else:
            _ICONS[slot] = _draw(key)
    return _ICONS[slot]


def fill_icon(color, on: bool) -> QIcon:
    """The section fill as a swatch: the style's fill colour, hatched, or
    an empty faint square when the fill is off."""
    ink = _ink()
    pm = QPixmap(18, 18)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    if on:
        r, g, b = (max(0, min(255, round(c * 255))) for c in color[:3])
        p.setBrush(QColor(r, g, b))
        p.setPen(QPen(ink, 1.2))
        p.drawRect(QRectF(3, 3, 12, 12))
        p.setPen(QPen(QColor(255, 255, 255, 140), 1.0))
        for k in (6, 10, 14):
            p.drawLine(QPointF(3, 3 + k), QPointF(3 + k, 3))
    else:
        faint = QColor(ink)
        faint.setAlpha(110)
        p.setPen(QPen(faint, 1.2, Qt.DashLine))
        p.drawRect(QRectF(3, 3, 12, 12))
    p.end()
    return QIcon(pm)


def _tool_button(text: str, tip: str, slot) -> QToolButton:
    btn = QToolButton()
    btn.setText(text)
    btn.setToolTip(tip)
    btn.setAutoRaise(True)
    btn.clicked.connect(slot)
    return btn


# ---- Model 3D ------------------------------------------------------------------

class BatchRenameDialog(QDialog):
    """Rename several rows at once (#565, @fafecm; #87): a name and a
    number (W01, W02…), find and replace, or a prefix and a suffix — with
    the names before and after side by side before anything changes. The
    rows come in the tree's order, top to bottom, so the numbers follow
    it. ``names()`` gives the new names, one per row."""

    NUMBER, REPLACE, AFFIX = range(3)

    def __init__(self, old_names, parent=None) -> None:
        super().__init__(parent)
        from PySide6.QtWidgets import (QCheckBox, QDialogButtonBox,
                                       QFormLayout, QSpinBox, QTabWidget,
                                       QTableWidget, QTableWidgetItem)
        self._Item = QTableWidgetItem
        # As Preferences: «— IngeTrazo» in the title, one tab per way of
        # renaming, forms with their labels, OK and Cancel at the bottom.
        self.setWindowTitle(tr("Rename selected") + " — Built")
        self.setMinimumWidth(440)
        self.resize(480, 440)
        self.old = list(old_names)
        lay = QVBoxLayout(self)
        self.mode = self.pages = QTabWidget()
        # Name and number
        page = QWidget()
        form = QFormLayout(page)
        self.base = QLineEdit(outliner_common_prefix(self.old) or "W")
        self.start = QSpinBox()
        self.start.setRange(0, 99999)
        self.start.setValue(1)
        self.digits = QSpinBox()
        self.digits.setRange(1, 6)
        self.digits.setValue(2)
        form.addRow(tr("Name:"), self.base)
        form.addRow(tr("Start at:"), self.start)
        form.addRow(tr("Digits:"), self.digits)
        self.pages.addTab(page, tr("Name and number"))
        # Find and replace
        page = QWidget()
        form = QFormLayout(page)
        self.find = QLineEdit()
        self.replace = QLineEdit()
        self.match_case = QCheckBox(tr("Match case"))
        form.addRow(tr("Find:"), self.find)
        form.addRow(tr("Replace with:"), self.replace)
        form.addRow("", self.match_case)
        self.pages.addTab(page, tr("Find and replace"))
        # Prefix and suffix
        page = QWidget()
        form = QFormLayout(page)
        self.prefix = QLineEdit()
        self.suffix = QLineEdit()
        form.addRow(tr("Prefix:"), self.prefix)
        form.addRow(tr("Suffix:"), self.suffix)
        self.pages.addTab(page, tr("Prefix and suffix"))
        lay.addWidget(self.pages)
        self.preview = QTableWidget(len(self.old), 2)
        self.preview.setHorizontalHeaderLabels([tr("Before"), tr("After")])
        self.preview.verticalHeader().setVisible(False)
        self.preview.horizontalHeader().setStretchLastSection(True)
        self.preview.setEditTriggers(QAbstractItemView.NoEditTriggers)
        self.preview.setColumnWidth(0, 200)
        lay.addWidget(self.preview, 1)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok
                                   | QDialogButtonBox.Cancel)
        self.ok = buttons.button(QDialogButtonBox.Ok)
        self.ok.setText(tr("Rename"))
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        lay.addWidget(buttons)
        for w in (self.base, self.find, self.replace, self.prefix,
                  self.suffix):
            w.textChanged.connect(self._update)
        for w in (self.start, self.digits):
            w.valueChanged.connect(self._update)
        self.match_case.toggled.connect(self._update)
        self.mode.currentChanged.connect(self._update)
        self._update()

    def names(self) -> list:
        mode = self.mode.currentIndex()
        if mode == self.NUMBER:
            return outliner.numbered_names(len(self.old), self.base.text(),
                                           self.start.value(),
                                           self.digits.value())
        if mode == self.REPLACE:
            return outliner.replaced_names(self.old, self.find.text(),
                                           self.replace.text(),
                                           self.match_case.isChecked())
        return outliner.affixed_names(self.old, self.prefix.text(),
                                      self.suffix.text())

    def _update(self, *_a) -> None:
        new = self.names()
        bold = QFont(self.font())
        bold.setBold(True)
        for row, (before, after) in enumerate(zip(self.old, new)):
            self.preview.setItem(row, 0, self._Item(before))
            cell = self._Item(after)
            if after != before:
                cell.setFont(bold)          # what will change
            self.preview.setItem(row, 1, cell)
        changed = any(a != b for a, b in zip(self.old, new))
        self.ok.setEnabled(changed and all(n.strip() for n in new))


def outliner_common_prefix(names) -> str:
    """What the names start with in common, trailing digits, spaces and
    separators dropped — «Muro 01», «Muro 02» → «Muro »… → «Muro»; a fair
    first guess for the base of a numbering."""
    import os
    common = os.path.commonprefix(list(names)) if names else ""
    return common.rstrip("0123456789 _-#.").strip()


class _Tree(SideTree):
    """The Browser's tree, with the keyboard of an outliner: Space shows
    or hides the selected rows, Enter opens the current one, Esc steps out
    one level, Tab puts the selection into the group above it and Shift+Tab
    takes it out one level. A drag carries every selected row, stacked and
    see-through, and outlines the group it would go into — or the row it
    would stand beside — as the Layers tab does."""

    #: A press on one of these acts at once and is kept from the selection:
    #: the rows stay as selected as they were, so the eye of one of several
    #: selected rows hides them all (as the material panel paints them all).
    CELLS = (COL_EYE, COL_LOCK, COL_ISOLATE)

    def event(self, ev) -> bool:
        from PySide6.QtCore import QEvent
        editing = self.state() == QAbstractItemView.EditingState
        if (ev.type() == QEvent.ShortcutOverride and not editing
                and ev.key() in _TREE_KEYS
                and not (ev.modifiers() & (Qt.ControlModifier
                                           | Qt.AltModifier))):
            ev.accept()
            return True
        return super().event(ev)

    def keyPressEvent(self, ev) -> None:
        if self.state() != QAbstractItemView.EditingState:
            key = ev.key()
            if key == Qt.Key_F2:
                plane = self._panel._plane_of(self.currentItem())
                if plane is not None:
                    self._panel.rename_plane(plane)
                    return
                picked = self._panel._selected_groups()
                if len(picked) > 1:         # several rows: all at once
                    self._panel.ask_batch_rename(picked)
                    return
            if key == Qt.Key_Space:
                self._panel.toggle_hidden()
                return
            if key in (Qt.Key_Return, Qt.Key_Enter):
                item = self.currentItem()
                if item is not None:
                    self._panel.open_item(item)
                return
            if key == Qt.Key_Escape:
                self._panel.step_out()
                return
        super().keyPressEvent(ev)

class OutlinerPanel(QWidget):
    """The model's tree of groups and components (the Model 3D tab)."""

    def __init__(self, window) -> None:
        super().__init__()
        self._window = window
        self._updating = False
        self._stale = False
        self._shape: tuple | None = None
        self._items: dict = {}              # id(group) -> its row
        self._groups: dict = {}             # id(group) -> the group
        self._state: dict = {}              # id(group) -> what its row shows
        self._shown_selection: tuple = ()
        #: Rows picked in the tree that the view cannot select — hidden or
        #: locked — kept highlighted here: clicked, they used to be
        #: deselected at once, the view having nothing to select. Dropped
        #: when the selection changes anywhere else.
        self._tree_only: set = set()
        #: What Isolate left in view and what it hid, to say «alone» on the
        #: target of those rows and to undo exactly that on the way out.
        self._isolated: list = []
        self._kept: list = []
        self._shown_path: tuple = ()
        self._query = ""
        #: The funnel's quick filter and whether it is inverted: seen in
        #: the rows, never written to the document.
        self._kind_providers: tuple = ()
        self._root = None
        self._sec_root = None               # the «Section planes» folder
        self._sec_items: dict = {}          # id(plane) -> its row
        self._sec_state: dict = {}
        self._sec_shape = None
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        lay.setSpacing(4)
        bar = QHBoxLayout()
        self.search = QLineEdit()
        self.search.setPlaceholderText(tr(
            "Filter by name, type or layer… (* and ? work)"))
        self.search.setClearButtonEnabled(True)
        self.search.textChanged.connect(self._on_filter)
        bar.addWidget(self.search, 1)
        # Rename: one row in place, several rows at once (Rename selected).
        self._rename_btn = _tool_button("", tr(
            "Rename (F2): several selected rows at once — a name and a "
            "number, find and replace, a prefix or a suffix; one row in "
            "place; nothing selected: every row the filter shows"),
            self._rename_picked)
        self._rename_btn.setIcon(node_icon("layers_rename"))
        bar.addWidget(self._rename_btn)
        # Expand all and Collapse all in one button too: ⊞ while a branch
        # is folded, ⊟ once everything is open.
        self._expand_btn = _tool_button("", tr("Expand all"),
                                        self._expand_or_collapse)
        self._expand_btn.setIcon(node_icon("layers_expand"))
        bar.addWidget(self._expand_btn)
        lay.addLayout(bar)
        self.tree = _Tree(self)
        self.tree.setColumnCount(9)
        self.tree.setHeaderLabels([tr("Name"), "", "", tr("Type"),
                                   tr("Layer"), tr("Material"), tr("Copies"),
                                   tr("IFC class"), ""])
        head = self.tree.header()
        # The Name column is the user's to size — drag its edge in the
        # header; the width is remembered. The last column takes the rest.
        head.setStretchLastSection(True)
        # A title that does not fit ends in «…»: cut short at the column's
        # edge, the last one looked hidden under the columns button.
        head.setTextElideMode(Qt.ElideRight)
        head.setSectionResizeMode(COL_NAME, QHeaderView.Interactive)
        head.resizeSection(COL_NAME, 200)
        head.setSectionResizeMode(COL_EYE, QHeaderView.Fixed)
        head.resizeSection(COL_EYE, 26)
        head.setSectionResizeMode(COL_LOCK, QHeaderView.Fixed)
        head.resizeSection(COL_LOCK, 26)
        head.setSectionResizeMode(COL_ISOLATE, QHeaderView.Fixed)
        head.resizeSection(COL_ISOLATE, 26)
        head.setSectionResizeMode(COL_TYPE, QHeaderView.Interactive)
        head.resizeSection(COL_TYPE, 110)
        head.setSectionResizeMode(COL_LAYER, QHeaderView.Interactive)
        head.resizeSection(COL_LAYER, 80)
        for col, width in ((COL_MATERIAL, 100), (COL_COPIES, 60),
                           (COL_IFC, 110)):
            head.setSectionResizeMode(col, QHeaderView.Interactive)
            head.resizeSection(col, width)
        # Right-click on the header: which columns show (remembered).
        head.setContextMenuPolicy(Qt.CustomContextMenu)
        head.customContextMenuRequested.connect(self._on_header_menu)
        # A button at the header's right end opens the same menu, so the
        # columns that are off show they are there to turn on.
        self._columns_button = _HeaderButton(self.tree)
        self._columns_button.setToolTip(tr(
            "Columns: show or hide Type, Layer, Material, Copies and IFC "
            "class"))
        self._columns_button.clicked.connect(self._on_columns_button)
        head.installEventFilter(self)
        self.tree.horizontalScrollBar().valueChanged.connect(
            lambda _v: self._place_columns_button())
        self._apply_column_choice()
        # The eye, the padlock and the target come first, before the name;
        # the tree's branches stay on the Name column wherever it sits. The
        # columns can be dragged to another order, which is remembered;
        # Window ▸ Panels ▸ Restore default panels puts them back.
        self.tree.setTreePosition(COL_NAME)
        head.setSectionsMovable(True)
        head.setFirstSectionMovable(True)
        self._apply_column_order()
        head.sectionMoved.connect(self._save_column_order)
        self.tree.headerItem().setIcon(COL_ISOLATE, isolate_icon(False))
        self.tree.headerItem().setToolTip(COL_ISOLATE, tr(
            "Isolate — a row's target shows only that group, a second click "
            "shows back what it hid; click here to end the isolation"))
        # The eye's and the padlock's own headers act on the whole model.
        head.setSectionsClickable(True)
        head.sectionClicked.connect(self._on_header_clicked)
        self.tree.setUniformRowHeights(True)
        # A short step per level of nesting: the branch arrow still fits,
        # and a deep tree keeps its names in a sidebar-wide panel (20 px a
        # level was Qt's default).
        self.tree.setIndentation(self.INDENT)
        self.tree.setAlternatingRowColors(True)
        self.tree.setSelectionMode(QAbstractItemView.ExtendedSelection)
        # Double-click OPENS a group, as in the view: renaming is F2 or the
        # context menu.
        self.tree.setEditTriggers(QAbstractItemView.EditKeyPressed)
        self.tree.setToolTip(tr(
            "Click to select in the model; double-click or Enter opens the "
            "group. The eye (or Space) shows or hides, the padlock locks; "
            "F2 renames; Delete removes the picked rows; Esc steps out one "
            "level. Drag rows onto a group, or press Tab / Shift+Tab, to "
            "move them in or out of it."))
        self.tree.itemSelectionChanged.connect(self._on_tree_selection)
        self.tree.itemSelectionChanged.connect(self._update_rename_button)
        self.tree.itemClicked.connect(self._on_item_clicked)
        self.tree.itemDoubleClicked.connect(self._on_double_clicked)
        self.tree.itemChanged.connect(self._on_item_changed)
        self.tree.setContextMenuPolicy(Qt.CustomContextMenu)
        self.tree.customContextMenuRequested.connect(self._on_context_menu)
        self.tree.itemExpanded.connect(
            lambda _i: self._update_expand_button())
        self.tree.itemCollapsed.connect(
            lambda _i: self._update_expand_button())
        self._root = None
        lay.addWidget(self.tree, 1)
        self._empty = QLabel(tr(
            "No groups or components in the model yet."))
        self._empty.setWordWrap(True)
        self._empty.setEnabled(False)
        lay.addWidget(self._empty)
        self.summary = QLabel()
        self.summary.setEnabled(False)
        lay.addWidget(self.summary)
        self._matches = 0
        self.refresh()

    # ---- Model → view --------------------------------------------------------
    def _scene(self):
        return self._window.viewport.scene

    def on_scene_changed(self) -> None:
        """The dock's per-change call: refreshed when on screen, on
        showing otherwise — a hidden tab costs nothing per edit. On screen,
        one refresh 300 ms after the last edit: every row's state is read
        again (17 ms per edit on the Plaza, release_check 09-10), and a
        burst of edits pays it once."""
        if not self.isVisible():
            if not self._stale and self._holds_departed_groups():
                self._forget_rows()     # New / Open: let the old one go
            self._stale = True
            return
        timer = getattr(self, "_refresh_timer", None)
        if timer is None:
            from PySide6.QtCore import QTimer
            timer = self._refresh_timer = QTimer(self)
            timer.setSingleShot(True)
            timer.timeout.connect(
                lambda: self.refresh(rows_if_changed=True, deferred=True))
        timer.start(300)

    def showEvent(self, ev) -> None:
        super().showEvent(ev)
        if self._stale:
            self.refresh()

    def _document_name(self) -> str:
        w = self._window
        path = getattr(w, "_current_path", None)
        if path is not None:
            return path.name
        return getattr(w, "_import_name", None) or tr("Untitled")

    def refresh_icons(self) -> None:
        """Repaint every mark drawn in the palette's ink and accent after
        the theme or the accent changes — the header's, the buttons' and
        every row's. The rows set theirs once and only touch what changed,
        so a theme switched with the app open left Light's black eyes and
        padlocks on Dark's rows (the Layers tab's #1086, for the Browser)."""
        hi = self.tree.headerItem()
        hi.setIcon(COL_EYE, node_icon("eye"))
        hi.setIcon(COL_LOCK, node_icon("lock"))
        hi.setIcon(COL_ISOLATE, isolate_icon(False))
        self._rename_btn.setIcon(node_icon("layers_rename"))
        self._update_expand_button()
        root = getattr(self, "_root", None)
        if root is not None:
            root.setIcon(COL_NAME, node_icon("document"))
        # Forget what each row last showed, so the next pass paints all.
        was = self._updating
        self._updating = True
        try:
            for key, item in list(getattr(self, "_items", {}).items()):
                state = self._state.get(key)
                if state is not None:
                    self._paint(item, None, state)
        finally:
            self._updating = was
        self._sec_state = {}
        self._sec_folder = None
        self._rows_key = None
        self.refresh()

    def refresh(self, rows_if_changed: bool = False, *, deferred=False) -> None:
        from views.panel_refresh import refresh_steps
        scene = self._scene()
        version = scene.version
        refresh_steps(self, self._refresh_steps(rows_if_changed), deferred=deferred,
                      current=lambda: self._scene() is scene and scene.version == version)

    def _refresh_steps(self, rows_if_changed):
        """Bring the tree up to date: rows rebuilt only when groups came or
        went, otherwise updated in place — a refresh follows every edit and
        every click, and a rebuild would lose the scroll and the branches
        the user unfolded. ``rows_if_changed`` (the model's own change
        notice): the rows are read again only if the content or the open
        path changed since they last were."""
        if self.tree.state() == QAbstractItemView.EditingState:
            return                      # a rename in progress: leave it be
        self._stale = False
        # The rows Isolate left in view on their own: read once, not once
        # per row (a walk of the model per row would cost a big document).
        self._alone_ids = {id(g) for g in self._alone_now()}
        self._kind_providers = tuple(
            getattr(self._window, "_outliner_kinds", None) or ())
        scene = self._scene()
        struct = outliner.structure(scene.groups)
        yield
        # A click changes the view, not the rows: with the content and the
        # open path as they were, every row reads as it did (21 406 rows
        # read again after each click on the plant of #158, ~0.15 s).
        rows_key = (getattr(scene, "content_version", None), id(scene),
                    struct, tuple(id(g) for g in outliner.open_path(scene)),
                    self._kind_providers, self._document_name())
        old_key = getattr(self, "_rows_key", None)
        hist = getattr(self._window.viewport, "history", None)
        # Only groups put in or taken out since the rows were last read
        # (a Delete, a Paste, their undo): their rows
        # come and go, the others stay as they are — but for the copies
        # count of a component that gained or lost one.
        groups_only = (rows_if_changed and old_key is not None
                       and rows_key[0] is not None
                       and not self._kind_providers
                       and old_key[1] == rows_key[1]
                       and old_key[3:] == rows_key[3:]
                       and hist is not None
                       and hist.feed.current("row_groups", rows_key[0]))
        # Only a group opened or closed since (History.context_changed):
        # what a row shows changes for the groups on the open path, old and
        # new, and for the copies of a component whose count moved (its
        # instance edits a world copy) — the other 21 406 rows of the plant
        # of #158 read as they did (~0.15 s each way).
        ctx_only = (rows_if_changed and old_key is not None
                    and rows_key[0] is not None
                    and not self._kind_providers
                    and struct == self._shape
                    and old_key[1:3] == rows_key[1:3]
                    and old_key[4:] == rows_key[4:]
                    and hist is not None
                    and hist.feed.current("row_groups", rows_key[0]))
        patched = rebuilt = False
        if struct != self._shape:
            patched = groups_only and self._patch_rows(scene, struct)
            if not patched:
                yield from self._rebuild(scene)
                rebuilt = True
            self._shape = struct
            self._struct_shape = struct
        # Nor does a Move — unless an extension's kind
        # adds properties to the rows, which may well be sizes.
        moved_only = (old_key is not None and not self._kind_providers
                      and old_key[1:] == rows_key[1:]
                      and hist is not None and rows_key[0] is not None
                      and hist.feed.current("rows", rows_key[0]))
        if (not rows_if_changed or rows_key[0] is None
                or (not moved_only and (rows_key != old_key or (hist is not None
                    and not hist.feed.current("rows", rows_key[0]))))):
            if ctx_only:
                self._update_some_rows(scene, set(old_key[3])
                                       | set(rows_key[3]))
            elif not patched and not rebuilt:
                if (hist is not None and rows_if_changed and not self._kind_providers
                        and old_key is not None and old_key[1:] == rows_key[1:]
                        and hist.feed.current("row_edits", rows_key[0])):
                    changed = set(hist.feed.readers.dirty_row_uids())
                    self._update_some_rows(scene, {k for k, g in self._groups.items() if g.uid in changed})
                else:
                    self._update_rows(scene)
            if hist is not None and rows_key[0] is not None:
                hist.feed.readers.mark("rows", rows_key[0])
                hist.feed.readers.mark("row_edits", rows_key[0])
                hist.feed.readers.mark("row_groups", rows_key[0])
        self._rows_key = rows_key
        self._sync_sections(scene)
        if self._filters_active():
            self._apply_filter()
        self._sync_selection(scene)
        self._empty.setVisible(not self._items)
        self._update_summary()
        self._update_expand_button()
        self._update_rename_button()

    def _holds_departed_groups(self) -> bool:
        """Whether a row still holds a group the model no longer has —
        after New or Open (both load into the same scene), or a delete."""
        if not self._groups:
            return False
        live = {id(g) for g, _p, _d in outliner.walk(self._scene().groups)}
        return any(key not in live for key in self._groups)

    def _forget_rows(self) -> None:
        """Drop the rows of a document that is no longer open. Each row
        holds its group, so a hidden Browser waiting to be shown kept the
        whole previous document in memory after Open (release_check
        10-10); the next refresh builds the rows of the new one."""
        self._kept_expanded = {self._groups[key].uid
                               for key, item in self._items.items()
                               if item.isExpanded()}
        self.tree.clear()
        self._items = {}
        self._groups = {}
        self._state = {}
        self._rows_key = None
        self._root = None
        self._shape = None

    def _update_summary(self) -> None:
        """The footer: what the model holds, or what the filter found."""
        total = len(self._state)
        if self._filters_active():
            text = tr("{shown} of {total} match", shown=self._matches,
                      total=total)
        else:
            comps = sum(1 for st in self._state.values()
                        if st[3] == "component")
            hidden = sum(1 for st in self._state.values() if st[4])
            text = tr("{groups} groups · {components} components · "
                      "{hidden} hidden", groups=total - comps,
                      components=comps, hidden=hidden)
        if self.summary.text() != text:
            self.summary.setText(text)

    def _rebuild(self, scene):
        """Rows for every group, each born with its texts and icons: one
        constructor call does what a setText per column would, and the
        cache it fills tells ``_update_rows`` there is nothing left to do."""
        expanded = {self._groups[key].uid
                    for key, item in self._items.items() if item.isExpanded()}
        expanded |= self.__dict__.get("_kept_expanded", set())
        scroll = self.tree.verticalScrollBar().value()
        counts = outliner.instance_counts(scene.groups)
        opened = {id(g) for g in outliner.open_path(scene)}
        items = {}
        groups = {}
        states = {}
        root = QTreeWidgetItem([self._document_name(), "", "", "", ""])
        root.setIcon(COL_NAME, node_icon("document"))
        root.setFlags(Qt.ItemIsEnabled | Qt.ItemIsDropEnabled)
        font = QFont(root.font(COL_NAME))
        font.setBold(True)
        root.setFont(COL_NAME, font)
        editable = Qt.ItemIsEditable
        # Imported assemblies contain many rows with identical roles,
        # icons and tooltips. Copy those Qt values once instead of
        # issuing dozens of setters for every hidden child row.
        templates = {}
        template_owner = QTreeWidgetItem()
        from core.materials import material_sig
        yield
        for i, (g, parent, _depth) in enumerate(outliner.walk(scene.groups)):
            key = id(g)
            state = self._row_state(g, counts, opened)
            template_key = (state[1:], material_sig(g.material))
            template = templates.get(template_key)
            if template is not None:
                item = template.clone()
                item.setText(COL_NAME, state[0])
                item.setData(COL_NAME, Qt.UserRole, g)
            else:
                item = QTreeWidgetItem([state[0], "", "", state[1],
                                        state[2], state[7], state[8],
                                        state[9]])
                item.setData(COL_NAME, Qt.UserRole, g)
                item.setFlags(item.flags() | editable)
                self._paint(item, None, state)
                if len(templates) < 512:
                    # Store a detached leaf: cloning an attached row
                    # later would also copy its descendants.
                    # PySide leaves clone() ownership with C++. Give the
                    # template a parent so its group role dies after the build.
                    template = item.clone()
                    template_owner.addChild(template)
                    templates[template_key] = template
            holder = root if parent is None else items[id(parent)]
            holder.addChild(item)
            items[key] = item
            groups[key] = g
            states[key] = state
            if i % 128 == 0:
                yield
        self._updating = True
        self.tree.blockSignals(True)
        try:
            self.tree.clear()
            self.__dict__.pop("_kept_expanded", None)
            self._items, self._groups, self._state = items, groups, states
            self._counts, self._root = counts, root
            self.tree.addTopLevelItem(root)
            root.setExpanded(True)
            sec = self._sec_root = QTreeWidgetItem(["", "", "", "", ""])
            sec.setIcon(COL_NAME, node_icon("sections"))
            sec.setFlags(Qt.ItemIsEnabled)
            sec.setFont(COL_NAME, font)
            self.tree.addTopLevelItem(sec)
            self._sec_items = {}
            self._sec_shape = None
            self._sec_state = {}
            self._sec_folder = None         # a new row: its texts to write
            for key, item in self._items.items():
                if self._groups[key].uid in expanded:
                    item.setExpanded(True)
        finally:
            self.tree.blockSignals(False)
            self._updating = False
        self._shown_selection = ()
        self._shown_path = ()
        self.tree.verticalScrollBar().setValue(scroll)

    #: Past this many rows coming or going, all of them are made again.
    _PATCH_MAX = 2000

    def _patch_rows(self, scene, shape) -> bool:
        """Take out the rows of the groups that left and put in rows for
        the ones that came, in their places — when every other group kept
        its parent and its order (``False`` otherwise, nothing touched).
        Making all 21 406 rows again after a Delete or a Paste on the plant
        of #158 took ~0.8 s; this, the rows that changed."""
        old = getattr(self, "_struct_shape", None)
        if old is None or self._root is None:
            return False
        was = {k for k, _p in old}
        now = {k for k, _p in shape}
        gone = was - now
        came = now - was
        if len(gone) + len(came) > self._PATCH_MAX:
            return False
        if ([x for x in old if x[0] not in gone]
                != [x for x in shape if x[0] not in came]):
            return False                 # something else moved in the tree
        counts = outliner.instance_counts(scene.groups)
        opened = {id(g) for g in outliner.open_path(scene)}
        scroll = self.tree.verticalScrollBar().value()
        self._updating = True
        self.tree.blockSignals(True)
        try:
            up = dict(old)
            for key in gone:
                item = self._items.pop(key)
                self._groups.pop(key, None)
                self._state.pop(key, None)
                if up.get(key) in gone:
                    continue        # goes with its parent's row
                parent = item.parent()
                if parent is not None:
                    parent.removeChild(item)   # its own rows go with it
            if came:
                editable = Qt.ItemIsEditable
                place: dict = {}             # parent id → children so far
                for g, parent, _depth in outliner.walk(scene.groups):
                    pk = id(parent) if parent is not None else 0
                    at = place.get(pk, 0)
                    place[pk] = at + 1
                    key = id(g)
                    if key not in came:
                        continue
                    state = self._row_state(g, counts, opened)
                    item = QTreeWidgetItem([state[0], "", "", state[1],
                                            state[2], state[7], state[8],
                                            state[9]])
                    item.setData(COL_NAME, Qt.UserRole, g)
                    item.setFlags(item.flags() | editable)
                    self._paint(item, None, state)
                    (self._root if parent is None
                     else self._items[id(parent)]).insertChild(at, item)
                    self._items[key] = item
                    self._groups[key] = g
                    self._state[key] = state
            # A component that gained or lost a copy: its rows' count.
            before = getattr(self, "_counts", None) or {}
            changed = {m for m in set(before) | set(counts)
                       if before.get(m) != counts.get(m)}
            if changed:
                for key, g in self._groups.items():
                    if key in came or id(g.mesh) not in changed:
                        continue
                    state = self._row_state(g, counts, opened)
                    if state != self._state[key]:
                        self._paint(self._items[key], self._state[key], state)
                        self._state[key] = state
            self._counts = counts
        finally:
            self.tree.blockSignals(False)
            self._updating = False
        self.tree.verticalScrollBar().setValue(scroll)
        return True

    def _kind(self, group) -> str:
        return "ifc" if outliner.ifc_class(group) else outliner.kind(group)

    def _providers(self):
        # Read once per refresh (see refresh): asked per row, the lookup of
        # a missing attribute on the Qt window cost ~1 s on 10 000 rows.
        return self._kind_providers

    def _type_text(self, group, counts) -> str:
        custom = outliner.custom_kind(group, self._providers())
        if custom is not None:
            return custom["type"]
        cls = outliner.ifc_class(group)
        if cls:
            return cls
        kind = outliner.kind(group)
        if kind == "component":
            n = counts.get(id(group.mesh), 1)
            return (tr("Component") if n < 2
                    else tr("Component ({n} copies)", n=n))
        return {"group": tr("Group"), "image": tr("Image"),
                "text": tr("3D Text")}[kind]

    def _row_state(self, g, counts, opened) -> tuple:
        """Everything a row shows: ``(name, type, layer, kind, hidden,
        open, locked, material, copies, IFC class, properties, hidden by,
        locked by, alone)`` — compared as one tuple, so an unchanged row
        costs no Qt call at all. The three last name the group around it
        that hides or locks it (``""`` when none) and whether Isolate left
        it in view on its own."""
        custom = outliner.custom_kind(g, self._providers())
        props = "\n".join(f"{k}: {v}" for k, v in custom["props"]) \
            if custom else ""
        copies = (str(counts.get(id(g.mesh), 1)) if g.is_component()
                  else "")
        hidden_by, locked_by = g._closed_ancestors()
        return (g.name or tr("Unnamed group"),
                custom["type"] if custom else self._type_text(g, counts),
                outliner.layer_name(g), self._kind(g), bool(g.hidden),
                id(g) in opened, bool(getattr(g, "locked", False)),
                outliner.material_name(g), copies, outliner.ifc_class(g),
                props,
                (hidden_by or tr("Unnamed group")) if hidden_by is not None else "",
                (locked_by or tr("Unnamed group")) if locked_by is not None else "",
                id(g) in getattr(self, "_alone_ids", ()))

    def _paint(self, item, old, new) -> None:
        """Bring ``item`` from the ``old`` state (``None``: just made, texts
        already in) to ``new``, touching only what differs."""
        if old is not None:
            for col, i in ((COL_NAME, 0), (COL_TYPE, 1), (COL_LAYER, 2),
                           (COL_MATERIAL, 7), (COL_COPIES, 8),
                           (COL_IFC, 9)):
                if old[i] != new[i]:
                    item.setText(col, new[i])
        if old is None or old[10] != new[10]:
            item.setToolTip(COL_TYPE, new[10])
        if old is None or old[7] != new[7]:
            group = item.data(COL_NAME, Qt.UserRole)
            paint = getattr(group, "material", None)
            item.setIcon(COL_MATERIAL, _paint_icon(paint) if paint
                         else QIcon())
        kind, hidden, bold = new[3], new[4], new[5]
        if old is None or old[3] != kind:
            item.setData(COL_TYPE, Qt.UserRole, kind)
            item.setIcon(COL_NAME, node_icon(kind))
        hidden_by = new[11]
        if old is None or old[4] != hidden or old[11] != hidden_by:
            item.setData(COL_EYE, Qt.UserRole, hidden)
            if hidden:
                item.setIcon(COL_EYE, mark_icon("eye_off"))
                item.setToolTip(COL_EYE, "")
            elif hidden_by:
                # Hidden by the group around it: closed, faded, not its own.
                item.setIcon(COL_EYE, mark_icon("eye_off", inherited=True))
                item.setToolTip(COL_EYE, tr("Hidden by «{name}»",
                                            name=hidden_by))
            else:
                item.setIcon(COL_EYE, mark_icon("eye"))
                item.setToolTip(COL_EYE, "")
            grey = hidden or bool(hidden_by)
            if grey or old is not None:
                item.setForeground(COL_NAME, self.tree.palette().brush(
                    QPalette.PlaceholderText if grey else QPalette.Text))
        if (old is None and bold) or (old is not None and old[5] != bold):
            font = QFont(item.font(COL_NAME))
            font.setBold(bold)
            item.setFont(COL_NAME, font)
            item.setToolTip(COL_NAME, tr("Open for editing") if bold else "")
        locked, locked_by = new[6], new[12]
        if old is None or old[6] != locked or old[12] != locked_by:
            item.setData(COL_LOCK, Qt.UserRole, locked)
            if locked:
                item.setIcon(COL_LOCK, mark_icon("lock"))
                item.setToolTip(COL_LOCK, "")
            elif locked_by:
                item.setIcon(COL_LOCK, mark_icon("lock", inherited=True))
                item.setToolTip(COL_LOCK, tr("Locked by «{name}»",
                                             name=locked_by))
            else:
                item.setIcon(COL_LOCK, mark_icon("lock_open"))
                item.setToolTip(COL_LOCK, "")
        alone = new[13]
        if old is None or old[13] != alone:
            # The Layers tab's own mark, so a target reads the same in
            # both lists: faint when idle, solid when the row is alone.
            item.setIcon(COL_ISOLATE, isolate_icon(alone))
            item.setToolTip(COL_ISOLATE, tr(
                "Shown alone — click to show back what Isolate hid") if alone
                else tr("Isolate: show only this group"))

    def _update_rows(self, scene) -> None:
        counts = self._counts = outliner.instance_counts(scene.groups)
        opened = {id(g) for g in outliner.open_path(scene)}
        self._updating = True
        try:
            name = self._document_name()
            if self._root.text(COL_NAME) != name:
                self._root.setText(COL_NAME, name)
            state_of = self._state
            for key, g in self._groups.items():
                state = self._row_state(g, counts, opened)
                old = state_of[key]
                if state != old:
                    self._paint(self._items[key], old, state)
                    state_of[key] = state
        finally:
            self._updating = False

    def _update_some_rows(self, scene, keys) -> None:
        """``_update_rows`` for the rows of ``keys`` and of every copy of
        a component whose count changed."""
        counts = outliner.instance_counts(scene.groups)
        old_counts = getattr(self, "_counts", None) or {}
        moved = {m for m in set(counts) | set(old_counts)
                 if counts.get(m) != old_counts.get(m)}
        self._counts = counts
        if moved:
            keys = set(keys) | {k for k, g in self._groups.items()
                                if id(g.mesh) in moved}
        opened = {id(g) for g in outliner.open_path(scene)}
        self._updating = True
        try:
            state_of = self._state
            for key in keys:
                g = self._groups.get(key)
                if g is None:
                    continue
                state = self._row_state(g, counts, opened)
                old = state_of[key]
                if state != old:
                    self._paint(self._items[key], old, state)
                    state_of[key] = state
        finally:
            self._updating = False

    def _reveal(self, item) -> None:
        parent = item.parent()
        while parent is not None:
            parent.setExpanded(True)
            parent = parent.parent()

    def _sync_selection(self, scene) -> None:
        """Highlight what is selected in the view and, when that changed,
        unfold its branch and scroll it into sight; unfold the groups open
        for editing too, so the context is on screen."""
        path = tuple(id(g) for g in outliner.open_path(scene))
        if path != self._shown_path:
            for key in path:
                item = self._items.get(key)
                if item is not None:
                    self._reveal(item)
                    item.setExpanded(True)
            self._shown_path = path
        chosen = tuple(id(g) for g in scene.selection
                       if id(g) in self._items)
        if set(chosen) != set(self._shown_selection):
            self._tree_only = set()         # selected elsewhere since
        self.tree.blockSignals(True)
        try:
            wanted = set(chosen) | {k for k in self._tree_only
                                    if k in self._items}
            # One selection for the whole tree, not one setSelected() per
            # row: every call re-diffed and re-emitted the growing
            # selection, quadratic, and a Structure node of 13k members
            # froze the window for an hour (Maxence, 2026-11-10).
            from PySide6.QtCore import QItemSelection
            selection, changed = QItemSelection(), False
            for key, item in self._items.items():
                want = key in wanted
                if item.isSelected() != want:
                    changed = True
                if want:
                    index = self.tree.indexFromItem(item, COL_NAME)
                    selection.select(index, index)
            if changed:
                self.tree.selectionModel().select(
                    selection, QItemSelectionModel.ClearAndSelect
                    | QItemSelectionModel.Rows)
            if chosen and set(chosen) != set(self._shown_selection):
                first = self._items[chosen[0]]
                self._reveal(first)
                self.tree.setCurrentItem(first, COL_NAME,
                                         QItemSelectionModel.NoUpdate)
                self.tree.scrollToItem(first)
        finally:
            self.tree.blockSignals(False)
        self._shown_selection = chosen

    # ---- Filter --------------------------------------------------------------
    def _on_filter(self, text: str) -> None:
        self._query = text.strip().casefold()
        self._apply_filter()
        self._update_summary()

    def _filters_active(self) -> bool:
        return bool(self._query)

    def _apply_filter(self) -> None:
        """Hide the rows the search and the funnel leave out — no rebuild:
        a filter typed letter by letter over thousands of rows only flips
        their flag. The containers on the way to a match stay, and unfold
        to reveal it."""
        self._filter_sections()
        if not self._filters_active():
            self._matches = len(self._items)
            for item in self._items.values():
                if item.isHidden():
                    item.setHidden(False)
            return
        scene = self._scene()
        counts = outliner.instance_counts(scene.groups)
        keep, hit = outliner.filter_keep(
            scene.groups, self._query,
            lambda g: self._type_text(g, counts))
        self._matches = len(hit)
        for key, item in self._items.items():
            item.setHidden(key not in keep)
            if key in keep and key not in hit:
                item.setExpanded(True)

    def _all_expanded(self) -> bool:
        """Whether every group with children is unfolded."""
        rows = list((getattr(self, "_items", None) or {}).values())
        return all(item.isExpanded() for item in rows if item.childCount())

    def _update_expand_button(self) -> None:
        b = getattr(self, "_expand_btn", None)
        if b is None or getattr(self, "tree", None) is None:
            return
        open_ = self._all_expanded()
        b.setIcon(node_icon("layers_collapse" if open_ else "layers_expand"))
        b.setToolTip(tr("Collapse all") if open_ else tr("Expand all"))

    def _expand_or_collapse(self) -> None:
        if self._all_expanded():
            self._collapse_all()
        else:
            self._expand_all()
        self._update_expand_button()

    def _expand_all(self) -> None:
        self.tree.expandAll()

    def _collapse_all(self) -> None:
        self.tree.collapseAll()
        if self._root is not None:
            self._root.setExpanded(True)

    # ---- View → model --------------------------------------------------------
    def _open_to(self, path) -> bool:
        return open_to(self._window.viewport, path)

    def _group_of(self, item):
        data = item.data(COL_NAME, Qt.UserRole) if item is not None else None
        return data if isinstance(data, Group) else None

    def _plane_of(self, item):
        data = item.data(COL_NAME, Qt.UserRole) if item is not None else None
        return data if isinstance(data, SectionPlane) else None

    # ---- Section planes ------------------------------------------------------
    _SECTION_MODE_TEXT = {
        "both": "Cut and frame", "cut": "Cut only (no frame)",
        "frame": "Frame only (no cut)", "off": "Off"}

    def _sync_sections(self, scene) -> None:
        """The «Section planes» folder: one row per plane, its mode in
        words, the eye for its frame and the slice for its cut; the folder
        row carries the two global switches (View ▸ Section Planes and
        Section Cuts)."""
        sec = self._sec_root
        if sec is None:
            return
        planes = list(getattr(scene, "section_planes", None) or ())
        shape = tuple(id(sp) for sp in planes)
        self._updating = True
        try:
            if shape != self._sec_shape:
                expanded = sec.isExpanded() or self._sec_shape is None
                sec.takeChildren()
                self._sec_items = {}
                self._sec_state = {}
                for sp in planes:
                    item = QTreeWidgetItem(["", "", "", "", ""])
                    item.setData(COL_NAME, Qt.UserRole, sp)
                    item.setFlags(Qt.ItemIsEnabled | Qt.ItemIsSelectable)
                    item.setIcon(COL_NAME, node_icon("sections"))
                    item.setToolTip(COL_LOCK, tr(
                        "Click: this plane cuts the model (or stops)"))
                    sec.addChild(item)
                    self._sec_items[id(sp)] = item
                self._sec_shape = shape
                sec.setExpanded(expanded)
            fill = self._fill_state(scene)
            for sp in planes:
                item = self._sec_items[id(sp)]
                mode = outliner.section_mode(sp)
                own = dict(getattr(sp, "fill", None) or {})
                state = (outliner.section_label(sp), mode, fill,
                         tuple(sorted((k, tuple(v) if k == "color" else v)
                                      for k, v in own.items())))
                if self._sec_state.get(id(sp)) == state:
                    continue
                self._sec_state[id(sp)] = state
                item.setText(COL_NAME, state[0])
                item.setText(COL_TYPE, tr(self._SECTION_MODE_TEXT[mode]))
                cut, frame = outliner.SECTION_MODES[mode]
                item.setData(COL_EYE, Qt.UserRole, not frame)
                item.setIcon(COL_EYE, node_icon("eye" if frame else "eye_off"))
                item.setToolTip(COL_EYE, tr("Hide the frame, keep the cut")
                                if frame else tr("Show the frame"))
                item.setData(COL_LOCK, Qt.UserRole, cut)
                item.setIcon(COL_LOCK, node_icon("cut" if cut else "cut_off"))
                self._paint_fill(item, cut, fill, sp)
                item.setForeground(COL_NAME, self.tree.palette().brush(
                    QPalette.PlaceholderText if mode == "off"
                    else QPalette.Text))
            # The folder: the global switches, and the count.
            shown = bool(getattr(scene, "show_section_planes", True))
            cuts = bool(getattr(scene, "show_section_cuts", True))
            folder = (tr("Section planes"), shown, cuts, len(planes), fill)
            if getattr(self, "_sec_folder", None) != folder:
                self._sec_folder = folder
                sec.setText(COL_NAME, folder[0])
                sec.setText(COL_TYPE, tr("{n} planes", n=len(planes)))
                sec.setIcon(COL_EYE, node_icon("eye" if shown else "eye_off"))
                sec.setToolTip(COL_EYE, tr(
                    "Every section plane's frame (View ▸ Section Planes)"))
                sec.setIcon(COL_LOCK, node_icon("cut" if cuts else "cut_off"))
                sec.setToolTip(COL_LOCK, tr(
                    "Section cuts in the model (View ▸ Section Cuts)"))
                self._paint_fill(sec, True, fill)
            sec.setHidden(not planes)
        finally:
            self._updating = False

    def _filter_sections(self) -> None:
        """The section planes follow the same search and funnel as the
        groups; the folder shows only when one of its rows does."""
        sec = self._sec_root
        if sec is None:
            return
        any_shown = False
        for i in range(sec.childCount()):
            item = sec.child(i)
            plane = self._plane_of(item)
            show = True
            if plane is not None:
                show = outliner.text_matches(plane.name or "", self._query)
            item.setHidden(not show)
            any_shown = any_shown or show
        sec.setHidden(sec.childCount() == 0 or not any_shown)

    _PATTERN_TEXT = {"solid": "Solid", "hatch": "Hatched 45°",
                     "cross": "Cross-hatched", "none": "No fill",
                     "layers": "Layers' hatches"}

    def set_plane_fill(self, plane, **changes) -> None:
        """Change this cut's own fill — ``pattern``, ``color``, ``spacing``;
        a value of ``None`` drops that key back to the style's. One undo
        step."""
        from core.history import SetSectionFillCommand
        fill = dict(getattr(plane, "fill", None) or {})
        for key, value in changes.items():
            if value is None:
                fill.pop(key, None)
            else:
                fill[key] = value
        if fill == (getattr(plane, "fill", None) or {}):
            return
        self._window.viewport.history.execute(
            SetSectionFillCommand(plane, fill or None))
        self._window.viewport.update()
        self.refresh()

    def pick_plane_fill_color(self, plane) -> None:
        from views.color_dialog import get_color
        from core.section import effective_fill
        _p, color, _s = effective_fill(
            plane, getattr(self._scene(), "display_style", None))
        chosen = get_color(QColor.fromRgbF(*(float(c) for c in color[:3])),
                           self, tr("Fill color of this cut"))
        if chosen.isValid():
            self.set_plane_fill(plane, color=(chosen.redF(), chosen.greenF(),
                                              chosen.blueF()))

    def ask_plane_hatch_spacing(self, plane) -> None:
        """The hatch spacing, typed as any length (5cm, 50mm, 2") — a bare
        number in the document's unit."""
        from core.section import effective_fill
        from core.units import fmt_len
        from views import prompts
        from views.document_interaction import _parse_length_field
        _p, _c, spacing = effective_fill(
            plane, getattr(self._scene(), "display_style", None))
        text, ok = prompts.get_text(self, tr("Hatch spacing"),
                                    tr("Distance between hatch lines:"),
                                    text=fmt_len(spacing))
        if not ok:
            return
        value = _parse_length_field(text or "")
        if value is None or value <= 0:
            self._window.viewport.flash_status(
                tr("Not a length: {text}", text=text), 3000)
            return
        self.set_plane_fill(plane, spacing=float(value))

    def _plane_fill_menu(self, menu, plane) -> None:
        """The cut's own fill: pattern, colour, spacing, or the style's."""
        from PySide6.QtCore import QTimer
        sub = menu.addMenu(tr("Fill of this cut"))
        sub.setToolTipsVisible(True)
        own = getattr(plane, "fill", None) or {}
        current = own.get("pattern")
        act = sub.addAction(tr("As the style"), lambda: self.set_plane_fill(
            plane, pattern=None))
        act.setCheckable(True)
        act.setChecked(current is None)
        for pattern in ("solid", "hatch", "cross", "layers", "none"):
            act = sub.addAction(tr(self._PATTERN_TEXT[pattern]),
                                lambda p=pattern: self.set_plane_fill(
                                    plane, pattern=p))
            act.setCheckable(True)
            act.setChecked(current == pattern)
            if pattern == "layers":
                act.setToolTip(tr(
                    "Each layer's cut with the hatch the Layers panel gives "
                    "it, as a sheet at 1:50 draws it"))
        sub.addSeparator()
        # Dialogs open once the menu has closed (see _fill_actions).
        sub.addAction(tr("Fill color of this cut…"), lambda: QTimer.singleShot(
            0, lambda: self.pick_plane_fill_color(plane)))
        if own.get("color") is not None:
            sub.addAction(tr("Use the style's fill color"),
                          lambda: self.set_plane_fill(plane, color=None))
        sub.addAction(tr("Hatch spacing…"), lambda: QTimer.singleShot(
            0, lambda: self.ask_plane_hatch_spacing(plane)))

    def rename_plane(self, plane) -> bool:
        """Name and symbol of a section plane — the dialog the Section tool
        opens on placing one, here undoable."""
        from PySide6.QtWidgets import (QDialog, QDialogButtonBox,
                                       QFormLayout)
        dlg = QDialog(self)
        dlg.setWindowTitle(tr("Name Section Plane"))
        form = QFormLayout(dlg)
        name_edit = QLineEdit(plane.name)
        sym_edit = QLineEdit(plane.symbol)
        sym_edit.setMaxLength(3)
        form.addRow(tr("Name"), name_edit)
        form.addRow(tr("Symbol"), sym_edit)
        buttons = QDialogButtonBox(
            QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(dlg.accept)
        buttons.rejected.connect(dlg.reject)
        form.addRow(buttons)
        self._rename_dialog = dlg           # reachable from the tests
        if dlg.exec() != QDialog.Accepted:
            return False
        return self.set_plane_name(plane, name_edit.text(), sym_edit.text())

    def set_plane_name(self, plane, name: str, symbol: str) -> bool:
        from core.history import RenameSectionPlaneCommand
        name = (name or "").strip() or plane.name
        symbol = (symbol or "").strip()[:3] or plane.symbol
        if (name, symbol) == (plane.name, plane.symbol):
            return False
        self._window.viewport.history.execute(
            RenameSectionPlaneCommand(plane, name, symbol))
        self._window.viewport.update()
        self.refresh()
        return True

    @staticmethod
    def _fill_state(scene) -> tuple:
        """``(on, colour)`` of the section fill — the display style's."""
        style = getattr(scene, "display_style", None)
        return (bool(getattr(style, "section_fill", True)),
                tuple(getattr(style, "section_fill_color",
                              (0.35, 0.37, 0.41))))

    def _paint_fill(self, item, show: bool, fill, plane=None) -> None:
        """The Fill cell (Layer column): on the folder row and on the plane
        that cuts — where a fill can show — empty on the others. A plane
        shows ITS fill: its own pattern and colour over the style's."""
        on, color = fill
        item.setData(COL_LAYER, Qt.UserRole, on if show else None)
        if not show:
            item.setIcon(COL_LAYER, QIcon())
            item.setText(COL_LAYER, "")
            item.setToolTip(COL_LAYER, "")
            return
        text = tr("Fill") if on else tr("No fill")
        if plane is not None:
            from core.section import effective_fill
            pattern, color, _s = effective_fill(
                plane, getattr(self._scene(), "display_style", None))
            on = pattern != "none"
            text = tr(self._PATTERN_TEXT[pattern])
        item.setIcon(COL_LAYER, fill_icon(color, on))
        item.setText(COL_LAYER, text)
        item.setToolTip(COL_LAYER, tr(
            "Section Fill: paint the cut faces solid (click to switch; its "
            "colour from the context menu or the Styles panel)"))

    def set_section_fill(self, on: bool) -> None:
        """View ▸ Section Fill, from the browser — the menu, the toolbar
        and the Styles panel follow."""
        w = self._window
        w._set_style_field("section_fill", bool(on))
        sync = getattr(w, "_sync_style_menu", None)
        if sync is not None:
            sync()
        self.refresh()

    def pick_section_fill_color(self) -> None:
        """The Styles panel's own colour picker for the fill — same dialog,
        same hint when the fill is off."""
        panel = getattr(self._window, "styles_panel", None)
        if panel is not None:
            panel._pick_color(tr("Section fill color"), "section_fill_color")
        self.refresh()

    def _fill_actions(self, menu) -> None:
        from PySide6.QtCore import QTimer
        on = self._fill_state(self._scene())[0]
        act = menu.addAction(tr("Section Fill"),
                             lambda: self.set_section_fill(not on))
        act.setCheckable(True)
        act.setChecked(on)
        # The dialog opens once the menu has closed: a modal opened inside
        # the menu's own event loop came back without a colour.
        menu.addAction(tr("Section fill color…"), lambda: QTimer.singleShot(
            0, self.pick_section_fill_color))

    def set_section_mode(self, plane, mode: str) -> None:
        """One plane: ``"both"``, ``"cut"``, ``"frame"`` or ``"off"`` —
        one undoable step."""
        from core.history import SetSectionModeCommand
        if outliner.section_mode(plane) == mode:
            return
        cut, frame = outliner.SECTION_MODES[mode]
        self._window.viewport.history.execute(
            SetSectionModeCommand(plane, cut, frame))
        self._window.viewport.update()
        self.refresh()

    def _toggle_section(self, plane, column) -> None:
        cut, frame = outliner.SECTION_MODES[outliner.section_mode(plane)]
        if column == COL_EYE:
            frame = not frame
        else:
            cut = not cut
        mode = {v: k for k, v in outliner.SECTION_MODES.items()}[(cut, frame)]
        self.set_section_mode(plane, mode)

    def set_section_globals(self, planes=None, cuts=None) -> None:
        """The View menu's two switches, from the folder row — they stay in
        step with the menu and the toolbar."""
        w = self._window
        if planes is not None:
            w._set_scene_visibility("show_section_planes", planes)
        if cuts is not None:
            w._set_scene_visibility("show_section_cuts", cuts)
        sync = getattr(w, "_sync_section_menu", None)
        if sync is not None:
            sync()
        self.refresh()

    def _select_plane(self, plane) -> None:
        scene = self._scene()
        if getattr(plane, "hidden", False):
            return                          # no frame to show selected
        scene.selection.clear()
        scene.selection.add(plane)
        self._shown_selection = ()
        scene.bump_view()
        self._window.viewport.update()

    def _selected_groups(self) -> list:
        return [g for g in (self._group_of(item)
                            for item in self.tree.selectedItems())
                if g is not None]

    def _on_tree_selection(self) -> None:
        """Rows picked in the tree become the view's selection. The view
        selects inside ONE context, so the rows that count are the current
        row's siblings; the groups above them open."""
        if self._updating:
            return
        current = self.tree.currentItem()
        plane = self._plane_of(current)
        if plane is not None and current.isSelected():
            self._select_plane(plane)
            return
        group = (self._group_of(current)
                 if current is not None and current.isSelected() else None)
        if group is None:
            picked = self._selected_groups()
            if not picked:
                return
            group = picked[0]
        scene = self._scene()
        path = outliner.path_to(scene.groups, group)
        if path is None:
            return
        siblings = [g for g in self._selected_groups()
                    if outliner.path_to(scene.groups, g) == path]
        if not self._open_to(path):
            self._window.viewport.flash_status(tr(
                "A group around it is hidden or locked: show or unlock "
                "it to select inside"),
                3500)
            return
        picks = [g for g in siblings if scene.entity_selectable(g)]
        # What the view cannot take (hidden, locked) stays picked here.
        self._tree_only = {id(g) for g in siblings if g not in picks}
        scene.selection.clear()
        scene.selection.update(picks)
        # Clicked here, so already in sight: the refresh this triggers must
        # not scroll the tree under the pointer.
        self._shown_selection = tuple(id(g) for g in picks)
        self._shown_path = tuple(id(g) for g in outliner.open_path(scene))
        scene.bump_view()
        self._window.viewport.update()

    def _on_item_clicked(self, item, column) -> None:
        if column == COL_ISOLATE:
            group = self._group_of(item)
            if group is None:
                return
            selected = self._selected_groups()
            self.toggle_isolated(selected if group in selected else [group])
            return
        if column == COL_LAYER and item.data(COL_LAYER, Qt.UserRole) \
                is not None and (item is self._sec_root
                                 or self._plane_of(item) is not None):
            self.set_section_fill(not self._fill_state(self._scene())[0])
            return
        if column in (COL_EYE, COL_LOCK):
            plane = self._plane_of(item)
            if plane is not None:
                self._toggle_section(plane, column)
                return
            if item is self._sec_root:
                scene = self._scene()
                if column == COL_EYE:
                    self.set_section_globals(planes=not getattr(
                        scene, "show_section_planes", True))
                else:
                    self.set_section_globals(cuts=not getattr(
                        scene, "show_section_cuts", True))
                return
        group = self._group_of(item)
        if group is None or column not in (COL_EYE, COL_LOCK):
            return
        # Closed by a group around it, not by itself: that group is what
        # opens it — say so instead of setting a state of its own nobody
        # would see.
        state = self._state.get(id(group))
        if state is not None:
            own = group.hidden if column == COL_EYE \
                else getattr(group, "locked", False)
            by = state[11] if column == COL_EYE else state[12]
            if by and not own:
                self._window.viewport.flash_status(
                    tr("Hidden by «{name}»: show it first", name=by)
                    if column == COL_EYE else
                    tr("Locked by «{name}»: unlock it first", name=by), 3500)
                return
        # The eye or the padlock clicked on one of several selected rows
        # acts on all of them, as in an outliner.
        selected = self._selected_groups()
        targets = selected if group in selected else [group]
        if column == COL_EYE:
            self.toggle_hidden(targets)
        else:
            self.toggle_locked(targets)

    def cell_clicked(self, item, column) -> None:
        """A press on one of the tree's cells — the eye, the padlock or the
        target: the panel answers it, and the selection is left as it was,
        so it acts on every selected row."""
        self._on_item_clicked(item, column)

    def _on_double_clicked(self, item, column) -> None:
        if column not in (COL_EYE, COL_LOCK, COL_ISOLATE):
            self.open_item(item)

    def open_item(self, item) -> None:
        """Open the row's group for editing — double-click or Enter. On a
        section plane: it cuts, or stops cutting."""
        plane = self._plane_of(item)
        if plane is not None:
            self._toggle_section(plane, COL_LOCK)
            return
        group = self._group_of(item)
        if group is None or group.hidden or getattr(group, "locked", False):
            return
        scene = self._scene()
        path = outliner.path_to(scene.groups, group)
        if path is None:
            return
        if self._open_to(path + [group]):
            scene.bump_view()
            self._window.viewport.update()
            self.refresh()

    def step_out(self) -> None:
        """Esc: leave the innermost open group."""
        if self._scene().edit_group is None:
            return
        self._window.viewport.end_one_group_edit()
        self._window.viewport.update()
        self.refresh()

    def toggle_hidden(self, groups=None) -> None:
        """Show the rows when any of them is hidden, hide them otherwise —
        one undoable step."""
        from core.history import HideCommand
        if groups is None:
            plane = self._plane_of(self.tree.currentItem())
            if plane is not None:
                self._toggle_section(plane, COL_EYE)
                return
            # Hiding drops the rows from the selection: the current row is
            # what a second Space shows again.
            groups = self._selected_groups()
            current = self._group_of(self.tree.currentItem())
            if not groups and current is not None:
                groups = [current]
        groups = list(groups)
        if not groups:
            return
        hide = not any(g.hidden for g in groups)
        self._window.viewport.history.execute(HideCommand(groups,
                                                          hidden=hide))
        self._keep_picked(groups)
        self._adopt_tree_only()
        self._window.viewport.update()
        self.refresh()

    def _keep_picked(self, groups) -> None:
        """Rows picked here that were just hidden or locked leave the view's
        selection, not this one: still highlighted, a second press of the
        eye or the padlock brings them back."""
        scene = self._scene()
        picked = {id(g) for g in self._selected_groups()}
        for g in groups:
            if id(g) in picked and not scene.entity_selectable(g):
                self._tree_only.add(id(g))
        self._shown_selection = tuple(
            id(g) for g in scene.selection if id(g) in self._items)

    def _adopt_tree_only(self) -> None:
        """Rows picked here while hidden or locked that the view can select
        now (shown, unlocked) join its selection: picked, then shown, they
        stay picked."""
        if not self._tree_only:
            return
        scene = self._scene()
        ready = [g for g in self._selected_groups()
                 if id(g) in self._tree_only and scene.entity_selectable(g)]
        if ready:
            scene.selection.update(ready)
            for g in ready:
                self._tree_only.discard(id(g))
            self._shown_selection = tuple(
                id(g) for g in scene.selection if id(g) in self._items)
            scene.bump_view()

    def toggle_locked(self, groups=None) -> None:
        """Unlock the rows when any of them is locked, lock them otherwise —
        one undoable step."""
        from core.protocol import commands
        if groups is None:
            groups = self._selected_groups()
            current = self._group_of(self.tree.currentItem())
            if not groups and current is not None:
                groups = [current]
        groups = list(groups)
        if not groups:
            return
        lock = not any(getattr(g, "locked", False) for g in groups)
        if lock and any(g is self._scene().edit_group
                        or g in outliner.open_path(self._scene())
                        for g in groups):
            self._window.viewport.end_group_edit()
        commands.lock(self._window.viewport.history, groups, lock)
        self._keep_picked(groups)
        self._adopt_tree_only()
        self._window.viewport.update()
        self.refresh()

    def _update_rename_button(self) -> None:
        b = getattr(self, "_rename_btn", None)
        if b is not None:
            b.setEnabled(bool(getattr(self, "_items", None)))

    def _shown_groups(self) -> list:
        """Every row, in the tree's order — or, with a filter typed, the
        rows it matches (not the groups shown only to hold them)."""
        scene = self._scene()
        hit = None
        if self._filters_active():
            counts = outliner.instance_counts(scene.groups)
            _keep, hit = outliner.filter_keep(
                scene.groups, self._query,
                lambda g: self._type_text(g, counts))
        return [g for g, _p, _d in outliner.walk(scene.groups)
                if id(g) in self._items and (hit is None or id(g) in hit)]

    def _rename_picked(self) -> None:
        """The pencil button: the selected rows renamed — several at once in
        the Rename selected window, one in its cell; with nothing selected,
        every row the tree shows (all, or what the filter keeps)."""
        picked = self._selected_groups()
        if not picked:
            # Nothing selected: every row on show (what the filter keeps).
            picked = self._shown_groups()
        if len(picked) > 1:
            self.ask_batch_rename(picked)
            return
        item = self.tree.currentItem()
        if picked and self._group_of(item) is not picked[0]:
            item = self._items.get(id(picked[0]))
        if item is not None and self._group_of(item) is not None:
            self.tree.setFocus()
            self.tree.editItem(item, COL_NAME)

    def ask_batch_rename(self, groups) -> bool:
        """The Rename selected window for ``groups`` (in the tree's order);
        what it gives is applied as one undo step. Whether anything was
        renamed."""
        order = {id(g): i for i, (g, _p, _d) in
                 enumerate(outliner.walk(self._scene().groups))}
        groups = sorted((g for g in groups if g is not None),
                        key=lambda g: order.get(id(g), 0))
        if len(groups) < 2:
            return False
        dlg = BatchRenameDialog([g.name or "" for g in groups], self)
        if dlg.exec() != QDialog.DialogCode.Accepted:
            return False
        return self.batch_rename(groups, dlg.names())

    def batch_rename(self, groups, names) -> bool:
        """Give ``groups`` the ``names`` — one undo step, the rows whose
        name does not change left out."""
        from core.protocol import commands
        pairs = [(g, n) for g, n in zip(groups, names)
                 if n.strip() and n != g.name]
        if not pairs:
            return False
        commands.rename_placements(self._window.viewport.history,
                                   [g for g, _ in pairs], [n for _, n in pairs])
        self._window.viewport.update()
        self.refresh()
        return True

    # ---- Columns ---------------------------------------------------------
    #: The step, in pixels, from one level of nesting to the next.
    INDENT = 12

    #: The factory order, left to right: the eye, the padlock and the
    #: target first, as in the Layers tab. Copies right after Layer: a
    #: short number — past Material it sat beyond the right edge of a
    #: sidebar-wide panel.
    COLUMN_ORDER = (COL_EYE, COL_LOCK, COL_ISOLATE, COL_NAME, COL_TYPE,
                    COL_LAYER, COL_COPIES, COL_MATERIAL, COL_IFC)
    #: The factory order before Copies moved next to Layer: saved as it
    #: was (no column ever moved by hand), it reads as the new one.
    _OLD_COLUMN_ORDER = (COL_EYE, COL_LOCK, COL_ISOLATE, COL_NAME, COL_TYPE,
                         COL_LAYER, COL_MATERIAL, COL_COPIES, COL_IFC)
    #: The factory widths.
    COLUMN_WIDTHS = {COL_NAME: 200, COL_EYE: 26, COL_LOCK: 26,
                     COL_ISOLATE: 26, COL_TYPE: 110, COL_LAYER: 80,
                     COL_MATERIAL: 100, COL_COPIES: 60, COL_IFC: 110}

    def _column_order(self) -> list:
        """The order the user left the columns in, or the factory one."""
        from core.settings import settings as QSettings
        raw = QSettings().value("browser/model_column_order")
        if isinstance(raw, str):
            raw = raw.split(",")
        try:
            order = [int(x) for x in raw or ()]
        except (TypeError, ValueError):
            order = []
        if (sorted(order) != sorted(self.COLUMN_ORDER)
                or tuple(order) == self._OLD_COLUMN_ORDER):
            return list(self.COLUMN_ORDER)
        return order

    def _apply_column_order(self, order=None) -> None:
        head = self.tree.header()
        head.blockSignals(True)
        try:
            for visual, col in enumerate(order or self._column_order()):
                head.moveSection(head.visualIndex(col), visual)
        finally:
            head.blockSignals(False)

    def _save_column_order(self, *_args) -> None:
        from core.settings import settings as QSettings
        head = self.tree.header()
        order = sorted(range(head.count()), key=head.visualIndex)
        QSettings().setValue("browser/model_column_order",
                             ",".join(str(c) for c in order))

    def reset_columns(self) -> None:
        """Window ▸ Panels ▸ Restore default panels, for the Browser: the
        factory order (the eye, the padlock and the target first), the
        factory columns shown and their widths; the choices remembered are
        forgotten."""
        from core.settings import settings as QSettings
        st = QSettings()
        for key in ("browser/model_column_order",
                    "browser/model_hidden_columns"):
            st.remove(key)
        self._apply_column_order(self.COLUMN_ORDER)
        self._apply_column_choice()
        for col, width in self.COLUMN_WIDTHS.items():
            self.tree.header().resizeSection(col, width)
        self.tree.horizontalScrollBar().setValue(0)
        self._shape = None                  # the rows, as the model holds them
        self.refresh()

    def eventFilter(self, obj, ev) -> bool:
        from PySide6.QtCore import QEvent
        if obj is self.tree.header() and ev.type() in (QEvent.Resize,
                                                       QEvent.Show):
            self._place_columns_button()
        return super().eventFilter(obj, ev)

    def _hidden_columns(self) -> set:
        from core.settings import settings as QSettings
        raw = QSettings().value("browser/model_hidden_columns")
        if raw is None:
            return {c for c, _l, shown in OPTIONAL_COLUMNS if not shown}
        if isinstance(raw, str):
            raw = [x for x in raw.split(",") if x]
        out = set()
        for x in raw or ():
            try:
                out.add(int(x))
            except (TypeError, ValueError):
                pass
        return out

    def _apply_column_choice(self) -> None:
        hidden = self._hidden_columns()
        for col, _label, _shown in OPTIONAL_COLUMNS:
            self.tree.setColumnHidden(col, col in hidden)

    def set_column_shown(self, col: int, shown: bool) -> None:
        from core.settings import settings as QSettings
        hidden = self._hidden_columns()
        (hidden.discard if shown else hidden.add)(col)
        QSettings().setValue("browser/model_hidden_columns",
                             ",".join(str(c) for c in sorted(hidden)) or ",")
        if shown and col == COL_COPIES:
            # Turned on to be read: right after Layer, wherever an older
            # order left it (past Material, out of view).
            head = self.tree.header()
            to = head.visualIndex(COL_LAYER) + 1
            if head.visualIndex(COL_COPIES) != to:
                head.moveSection(head.visualIndex(COL_COPIES),
                                 to if head.visualIndex(COL_COPIES) > to
                                 else to - 1)
                self._save_column_order()
        self._apply_column_choice()
        self._place_columns_button()

    def _columns_menu(self):
        """Which columns show — the Layers tab's way: the menu stays open
        while columns are switched, so Material, Copies and IFC class are
        turned on in one go; «Show all columns» turns every one on and
        «Default columns» puts back the ones that start shown."""
        menu = StayOpenMenu(self)
        hidden = self._hidden_columns()
        show_all = menu.addAction(tr("Show all columns"))
        show_all.setProperty("keep_open", True)
        menu.addSeparator()
        switches = []
        for col, label, _shown in OPTIONAL_COLUMNS:
            act = menu.addAction(tr(label))
            act.setCheckable(True)
            act.setChecked(col not in hidden)
            act.toggled.connect(lambda on, c=col: self.set_column_shown(c, on))
            switches.append(act)
        # Back to the starting point: last, apart, as a «reset» goes.
        menu.addSeparator()
        defaults = menu.addAction(tr("Default columns"))
        defaults.setProperty("keep_open", True)

        def all_on() -> None:
            for act in switches:
                act.setChecked(True)             # each switch applies itself

        def back_to_defaults() -> None:
            for (_col, _label, shown), act in zip(OPTIONAL_COLUMNS, switches):
                act.setChecked(shown)
        show_all.triggered.connect(all_on)
        defaults.triggered.connect(back_to_defaults)
        return menu

    def _on_header_menu(self, pos) -> None:
        menu = self._columns_menu()
        menu.exec(self.tree.header().mapToGlobal(pos))

    def _on_columns_button(self) -> None:
        b = self._columns_button
        self._columns_menu().exec(b.mapToGlobal(b.rect().bottomLeft()))

    def _place_columns_button(self) -> None:
        """At the header's right end, as tall as the header; its text says
        how many columns are left to turn on («+3»), or «⋮» when none."""
        b = getattr(self, "_columns_button", None)
        if b is None:
            return
        head = self.tree.header()
        n = len(self._hidden_columns())
        b.setText(f"+{n}" if n else "⋮")
        font = b.font()
        font.setBold(bool(n))               # more to turn on: say it
        b.setFont(font)
        side = max(head.height(), 18)
        width = max(side, b.sizeHint().width())
        tree = self.tree
        if tree.button_strip != width:
            tree.button_strip = width       # the strip it stands in
            tree.updateGeometries()
        # In the strip right of the header: fixed, whatever the columns do.
        g = head.geometry()
        b.setGeometry(g.right() + 1, g.top(), width, g.height())
        b.raise_()

    # ---- Several rows at once --------------------------------------------
    def assign_layer(self, groups, layer: str) -> None:
        """Put the rows on ``layer`` (made if new) — one undo step."""
        from core.history import AssignLayerCommand
        groups = [g for g in groups if g is not None]
        if not groups or not layer:
            return
        self._window.viewport.history.execute(
            AssignLayerCommand(groups, layer))
        self._window.viewport.update()
        self.refresh()

    def _ask_new_layer(self, groups) -> None:
        from views import prompts
        name, ok = prompts.get_text(self, tr("New layer"),
                                    tr("Layer name:"))
        if ok and name.strip():
            self.assign_layer(groups, name.strip())

    def _batch_menu(self, menu, targets) -> None:
        layers = menu.addMenu(tr("Layer"))
        current = {outliner.layer_name(g) for g in targets}
        from core.layers import path_label
        for ly in self._scene().layers:
            act = layers.addAction(path_label(self._scene(), ly.key),
                                   lambda n=ly.key:
                                   self.assign_layer(targets, n))
            act.setCheckable(True)
            act.setChecked(current == {ly.key})
        layers.addSeparator()
        from PySide6.QtCore import QTimer
        layers.addAction(tr("New layer…"), lambda: QTimer.singleShot(
            0, lambda: self._ask_new_layer(targets)))

    # ---- A plan of a group -------------------------------------------------
    def create_plan(self, group, cut_height: float, sheet: bool = True,
                    scale: float = 100.0, open_sheet: bool = False):
        """A floor plan of ``group``: a horizontal cut ``cut_height`` above
        its base (made the active cut), a scene looking straight down on it
        in parallel projection, and — with ``sheet`` — a new sheet with
        that scene at 1:``scale``. One undo step. Returns the scene, or
        ``None`` when the group holds no geometry."""
        import math
        from ingetrazo_core import Vec3 as QVector3D
        from core.camera import OrbitCamera
        from core.section import SectionPlane as _Plane
        from core.section import next_symbol
        scene = self._scene()
        bounds = outliner.world_bounds(scene.groups, group)
        if bounds is None:
            self._window.viewport.flash_status(
                tr("«{name}» holds no geometry to make a plan of",
                   name=group.name), 3500)
            return None
        (x0, y0, z0), (x1, y1, z1) = bounds
        z = z0 + float(cut_height)
        cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
        title = outliner.unique_name(
            tr("Plan {name}", name=group.name),
            [v.name for v in scene.saved_views])
        plane = _Plane(QVector3D(cx, cy, z), QVector3D(0, 0, 1),
                       name=title, symbol=next_symbol(scene.section_planes))
        cam = OrbitCamera()
        cam.target = QVector3D(cx, cy, z)
        cam.set_view("top")
        cam.perspective = False
        span = max(x1 - x0, y1 - y0, 1.0)
        cam.distance = span * 1.2 / (2 * math.tan(math.radians(
            cam.fov_deg) / 2))
        view = self._place_view(title, cam, plane, True, sheet, scale,
                                open_sheet)
        if z > z1 or z < z0:
            self._window.viewport.flash_status(tr(
                "The cut is outside «{name}»: the plan shows it whole",
                name=group.name), 4000)
        return view

    def _place_view(self, title, cam, plane, add_plane, sheet, scale,
                    open_sheet):
        """The part a plan of a group and a view of a cut share: the scene
        from ``cam`` cut by ``plane``, a sheet with it if asked — one undo
        step — and then the way to it: the sheet opened, or Sheets 2D with
        the scene marked, and the status bar saying what was made."""
        from core.composition import Composicion
        from core.history import CreatePlanCommand
        from core.saved_views import SavedView
        scene = self._scene()
        view = SavedView.capture(title, scene, cam)
        view.section = {"active": plane.uid, "planes_shown": False,
                        "cuts_shown": True}
        page = None
        if sheet:
            page = Composicion(name=outliner.unique_name(
                title, [c.name for c in scene.compositions]), paper="A3")
            frame = page.default_frame()
            frame.view_key = f"scene:{title}"
            frame.scale_n = float(scale)
            frame.show_title = True
            page.frames.append(frame)
        self._window.viewport.end_group_edit()
        self._window.viewport.history.execute(
            CreatePlanCommand(plane, view, page, add_plane=add_plane))
        self._window._refresh_sheet_tabs()
        self._window.viewport.update()
        self.refresh()
        what = (tr("«{name}»: a scene and a sheet — in the composer",
                   name=title) if page is not None
                else tr("«{name}»: a scene — drag it onto a sheet in the "
                        "composer", name=title))
        self._window.viewport.flash_status(what, 6000)
        if page is not None and open_sheet:
            self._window._show_sheet(
                self._scene().compositions.index(page))
        panel = getattr(getattr(self._window, "_composer", None), "views_panel", None)
        if panel is not None:
            panel.show_scene_row(title)
        return view

    def create_cut_view(self, plane, sheet: bool = True,
                        scale: float = 50.0, open_sheet: bool = False):
        """A view of an existing section plane: looking at the cut face on
        (from the side the plane takes away, towards what stays), in
        parallel projection, the plane made the active cut — a section for
        a vertical plane, a plan for a horizontal one. With ``sheet``, on a
        new sheet at 1:``scale``. One undo step; returns the scene."""
        import math
        from ingetrazo_core import Vec3 as QVector3D
        from core.camera import OrbitCamera
        scene = self._scene()
        lo, hi = scene.bounds()
        n = QVector3D(plane.normal).normalized()
        if lo is None:
            centre, span = QVector3D(plane.point), 10.0
        else:
            centre = (lo + hi) * 0.5
            size = hi - lo
            span = max(size.x(), size.y(), size.z(), 1.0)
        # The target on the plane, facing the model's middle.
        target = centre - n * QVector3D.dotProduct(centre - plane.point, n)
        cam = OrbitCamera()
        cam.target = target
        cam.yaw = math.atan2(n.y(), n.x())
        cam.pitch = math.asin(max(-1.0, min(1.0, n.z())))
        cam.perspective = False
        cam.distance = span * 1.2 / (2 * math.tan(math.radians(
            cam.fov_deg) / 2))
        base = plane.name or tr("Section {symbol}", symbol=plane.symbol)
        title = outliner.unique_name(base, [v.name for v in scene.saved_views])
        return self._place_view(title, cam, plane, False, sheet, scale,
                                open_sheet)

    def ask_cut_view(self, plane) -> None:
        """The dialog for a view of a cut: a sheet or not, its scale, and
        whether to open it."""
        from PySide6.QtWidgets import (QCheckBox, QDialog, QDialogButtonBox,
                                       QFormLayout)
        from core.composition import format_scale, parse_scale
        dlg = QDialog(self)
        dlg.setWindowTitle(tr("View of the cut {name}",
                              name=outliner.section_label(plane)))
        form = QFormLayout(dlg)
        on_sheet = QCheckBox(tr("Also place it on a new sheet"))
        on_sheet.setChecked(True)
        form.addRow("", on_sheet)
        scale = QLineEdit(format_scale(50.0))
        form.addRow(tr("Scale:"), scale)
        open_it = QCheckBox(tr("Open the sheet when done"))
        open_it.setChecked(True)
        form.addRow("", open_it)
        on_sheet.toggled.connect(scale.setEnabled)
        on_sheet.toggled.connect(open_it.setEnabled)
        buttons = QDialogButtonBox(
            QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(dlg.accept)
        buttons.rejected.connect(dlg.reject)
        form.addRow(buttons)
        self._cut_view_dialog = dlg         # reachable from the tests
        if dlg.exec() != QDialog.Accepted:
            return
        self.create_cut_view(plane, on_sheet.isChecked(),
                             parse_scale(scale.text(), 50.0),
                             open_sheet=open_it.isChecked())

    def ask_scale(self, group) -> None:
        self._window._on_scale_dialog(group)
        self.refresh()

    def ask_plan(self, group) -> None:
        """The dialog: cut height above the group's base, a sheet or not,
        and its scale."""
        from PySide6.QtWidgets import (QCheckBox, QDialog, QDialogButtonBox,
                                       QFormLayout)
        from core.composition import format_scale, parse_scale
        from core.units import fmt_len
        from views.document_interaction import _parse_length_field
        dlg = QDialog(self)
        dlg.setWindowTitle(tr("Plan of {name}", name=group.name))
        form = QFormLayout(dlg)
        height = QLineEdit(fmt_len(1.2))
        height.setToolTip(tr("Measured up from the lowest point of the "
                             "group — 1.20 m cuts through the windows"))
        form.addRow(tr("Cut height:"), height)
        on_sheet = QCheckBox(tr("Also place it on a new sheet"))
        on_sheet.setChecked(True)
        form.addRow("", on_sheet)
        scale = QLineEdit(format_scale(100.0))
        form.addRow(tr("Scale:"), scale)
        open_it = QCheckBox(tr("Open the sheet when done"))
        open_it.setChecked(True)
        form.addRow("", open_it)
        on_sheet.toggled.connect(scale.setEnabled)
        on_sheet.toggled.connect(open_it.setEnabled)
        buttons = QDialogButtonBox(
            QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(dlg.accept)
        buttons.rejected.connect(dlg.reject)
        form.addRow(buttons)
        self._plan_dialog = dlg             # reachable from the tests
        if dlg.exec() != QDialog.Accepted:
            return
        cut = _parse_length_field(height.text() or "")
        if cut is None:
            self._window.viewport.flash_status(
                tr("Not a length: {text}", text=height.text()), 3000)
            return
        self.create_plan(group, cut, on_sheet.isChecked(),
                         parse_scale(scale.text(), 100.0),
                         open_sheet=open_it.isChecked())

    def _isolate_targets(self) -> list:
        from core.group import Group
        picked = self._selected_groups()
        if not picked:
            picked = [g for g in self._scene().selection
                      if isinstance(g, Group)]
        return picked

    def _isolated_now(self) -> list:
        """The groups Isolate hid that are hidden still — what leaving the
        isolation brings back. Empty: not isolated (left, undone, or each
        one shown by hand since)."""
        remembered = [g for g in getattr(self, "_isolated", None) or ()
                      if g.hidden]
        if not remembered:
            return []                   # the walk below is not cheap
        live = {id(g) for g, _p, _d in outliner.walk(self._scene().groups)}
        return [g for g in remembered if id(g) in live]

    def _alone_now(self) -> list:
        """The groups the target column shows as alone: the ones Isolate
        kept in view, still visible. Empty when the isolation ended (left,
        undone, or each one shown by hand since)."""
        if not self._isolated_now():
            return []
        live = {id(g) for g, _p, _d in outliner.walk(self._scene().groups)}
        return [g for g in getattr(self, "_kept", None) or ()
                if not g.hidden and id(g) in live]

    def exit_isolation(self) -> None:
        """Show again exactly what Isolate hid — one undoable step; what
        was hidden before isolating stays hidden."""
        from core.history import HideCommand
        back = self._isolated_now()
        self._isolated = []
        self._kept = []
        if back:
            self._window.viewport.history.execute(HideCommand(back,
                                                              hidden=False))
            self._window.viewport.update()
        self.refresh()

    def isolate(self) -> None:
        """Hide every group but the selected ones and those holding them —
        one undoable step; Exit isolation (or undo) brings them back.
        Loose geometry, dimensions and texts stay (#272)."""
        keep = self._isolate_targets()
        if not keep:
            self._window.viewport.flash_status(tr(
                "Select the groups to keep in view first"), 3000)
            return
        self.isolate_groups(keep)

    def isolate_groups(self, keep) -> None:
        """``isolate`` for the groups given — the row's target keeps just
        its own group in view, without touching the selection."""
        from core.history import HideCommand
        keep = [g for g in keep if g is not None]
        if not keep:
            return
        hide = outliner.isolate_hides(self._scene().groups, keep)
        if hide:
            self._window.viewport.history.execute(HideCommand(hide,
                                                              hidden=True))
            # Remembered, to give back exactly these when leaving it (an
            # isolation inside an isolation adds to it).
            before = self._isolated_now()
            self._isolated = before + [g for g in hide if g not in before]
            self._kept = list(keep)
            self._window.viewport.update()
        self.refresh()

    def toggle_isolated(self, groups) -> None:
        """The row's target: show only ``groups``, or — the second click,
        or a click while they are what Isolate left — bring back what it
        hid."""
        groups = [g for g in groups if g is not None]
        if not groups:
            return
        alone = self._alone_now()
        if any(any(g is a for a in alone) for g in groups):
            self.exit_isolation()
        else:
            self.isolate_groups(groups)

    def _on_header_clicked(self, col: int) -> None:
        if col == COL_ISOLATE:
            if self._isolated_now():
                self.exit_isolation()
            else:
                self.isolate()
            return

    def show_all(self) -> None:
        """Show every hidden group of the tree, at every level — one
        undoable step; an isolation ends with it."""
        from core.history import HideCommand
        hidden = [g for g, _p, _d in outliner.walk(self._scene().groups)
                  if g.hidden]
        self._isolated = []
        self._kept = []
        if hidden:
            self._window.viewport.history.execute(HideCommand(hidden,
                                                              hidden=False))
            self._window.viewport.update()
        self.refresh()

    def _on_item_changed(self, item, column) -> None:
        if self._updating or column != COL_NAME:
            return
        from core.protocol import commands
        group = self._group_of(item)
        if group is None:
            return
        name = item.text(COL_NAME).strip()
        if name and name != group.name:
            commands.rename_placement(self._window.viewport.history, group, name)
            self._window.viewport.update()
        self.refresh()

    def _on_context_menu(self, pos) -> None:
        from PySide6.QtCore import QTimer
        from PySide6.QtWidgets import QMenu
        item = self.tree.itemAt(pos)
        group = self._group_of(item)
        menu = QMenu(self)
        plane = self._plane_of(item)
        if plane is not None:
            menu.addAction(tr("Rename… (F2)"),
                           lambda: QTimer.singleShot(
                               0, lambda: self.rename_plane(plane)))
            menu.addAction(tr("Create view of this cut…"),
                           lambda: QTimer.singleShot(
                               0, lambda: self.ask_cut_view(plane)))
            menu.addSeparator()
            current = outliner.section_mode(plane)
            for mode in ("both", "cut", "frame", "off"):
                act = menu.addAction(tr(self._SECTION_MODE_TEXT[mode]),
                                     lambda m=mode: self.set_section_mode(
                                         plane, m))
                act.setCheckable(True)
                act.setChecked(mode == current)
            menu.addSeparator()
            self._fill_actions(menu)
            self._plane_fill_menu(menu, plane)
            menu.exec(self.tree.viewport().mapToGlobal(pos))
            return
        if item is not None and item is self._sec_root:
            scene = self._scene()
            for text, attr, key in (
                    (tr("Section Planes"), "show_section_planes", "planes"),
                    (tr("Section Cuts"), "show_section_cuts", "cuts")):
                on = bool(getattr(scene, attr, True))
                act = menu.addAction(text, lambda k=key, v=not on:
                                     self.set_section_globals(**{k: v}))
                act.setCheckable(True)
                act.setChecked(on)
            self._fill_actions(menu)
            menu.exec(self.tree.viewport().mapToGlobal(pos))
            return
        if group is not None:
            menu.addAction(tr("Rename…"), lambda: self.tree.editItem(
                item, COL_NAME))
            selected = self._selected_groups()
            targets = selected if group in selected else [group]
            if len(targets) > 1:
                menu.addAction(tr("Rename selected…"), lambda: QTimer.singleShot(
                    0, lambda: self.ask_batch_rename(targets)))
            menu.addAction(tr("Show") if group.hidden else tr("Hide"),
                           lambda: self.toggle_hidden(targets))
            menu.addAction(tr("Unlock") if getattr(group, "locked", False)
                           else tr("Lock"),
                           lambda: self.toggle_locked(targets))
            menu.addAction(tr("Open for editing"),
                           lambda: self.open_item(item))
            scale_act = menu.addAction(
                tr("Scale…"), lambda: QTimer.singleShot(
                    0, lambda: self.ask_scale(group)))
            scale_act.setEnabled(len(targets) == 1)
            menu.addSeparator()
            self._batch_menu(menu, targets)
            menu.addSeparator()
            menu.addAction(tr("Create plan of this group…"),
                           lambda: QTimer.singleShot(
                               0, lambda: self.ask_plan(group)))
        if self._scene().edit_group is not None:
            menu.addAction(tr("Close group"), self.step_out)
        if self._scene().selection:
            menu.addAction(tr("Zoom Selection"),
                           self._window._on_zoom_selection)
        menu.addSeparator()
        if group is not None or self._isolate_targets():
            menu.addAction(tr("Isolate"), self.isolate)
        if self._isolated_now():
            menu.addAction(tr("Exit isolation"), self.exit_isolation)
        # The way out of everything: every hidden group, at every level.
        menu.addAction(tr("Show everything hidden (every level)"),
                       self.show_all)
        menu.addSeparator()
        menu.addAction(tr("Expand all"), self._expand_all)
        menu.addAction(tr("Collapse all"), self._collapse_all)
        menu.exec(self.tree.viewport().mapToGlobal(pos))


# ---- The dock ------------------------------------------------------------------

class ProjectBrowser(QDockWidget):
    """Sidebar tab **Browser**: the model's tree. The sheets are the
    Sheets workspace's (views/wgpu_sheets.py)."""

    def __init__(self, window) -> None:
        super().__init__(tr("Browser"), window)
        self.setObjectName("tray_project_browser")
        self.setAllowedAreas(Qt.LeftDockWidgetArea | Qt.RightDockWidgetArea)
        self.setFeatures(QDockWidget.DockWidgetMovable
                         | QDockWidget.DockWidgetClosable)
        self._window = window
        self.outliner = OutlinerPanel(window)
        self.setWidget(self.outliner)
        self.setMinimumWidth(240)

    def on_scene_changed(self) -> None:
        self.outliner.on_scene_changed()
        panel = getattr(getattr(self._window, "_composer", None), "views_panel", None)
        if panel is not None:
            panel.on_scene_changed()
