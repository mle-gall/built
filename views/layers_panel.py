# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The **Layers** tab of the side tray: the document's layers as a tree,
beside the Browser.

One row per layer, filed under folders when it has one: the pencil marks
the ACTIVE layer (new objects go there, core.layers.tag_new_objects), the
eye and the padlock show and lock, and the swatch is the layer's colour. Clicking the eye or the padlock of one of several
selected rows acts on all of them; clicking a column header sorts, with the
default layer always first. Visibility, lock, colour and folders are
display state and stay off the undo stack, like before; deleting a layer
is one undo step (core.history.DeleteLayerCommand).
"""
from __future__ import annotations

import json

from PySide6.QtCore import (QItemSelectionModel, QPoint, QPointF, QRectF, Qt,
                            QTimer, Signal)
from PySide6.QtGui import QColor, QIcon, QPainter, QPen, QPixmap
from PySide6.QtGui import QAction, QBrush
from PySide6.QtWidgets import (QAbstractItemView, QApplication,
                               QDockWidget,
                               QHBoxLayout, QHeaderView, QMenu,
                               QStyledItemDelegate,
                               QToolButton, QTreeWidget, QTreeWidgetItem,
                               QTreeWidgetItemIterator, QVBoxLayout, QWidget)

from core import layers as L
from core.i18n import tr
from core.outliner import natural_key  # noqa: F401 — the Browser's too
from views.tree_icons import isolate_icon, row_icon
from views.side_tree import SideTree

(COL_NAME, COL_ACTIVE, COL_EYE, COL_LOCK, COL_COLOR, COL_COUNT,
 COL_ISOLATE, COL_LINETYPE, COL_WEIGHT, COL_OPACITY, COL_PRINT,
 COL_CUT, COL_HATCH) = range(13)

#: The row kinds kept in ``Qt.UserRole`` of the name column.
KIND_LAYER, KIND_FOLDER = "layer", "folder"
#: Name-column role: the row holds what is selected in the model (its
#: layer, or a folded folder with that layer inside).
ROLE_HOLDS = Qt.UserRole + 2
#: Line-type column role: the layer's line type key, drawn by
#: :class:`_LineDelegate`.
ROLE_LINETYPE = Qt.UserRole + 3
#: Line-type column role: the dash pattern itself, in paper mm — the
#: document's own line types too.
ROLE_LINEPATTERN = Qt.UserRole + 5
#: Name-column role: where the row stands in the document's own order (the
#: order the layers and folders were made in).
ROLE_ORDER = Qt.UserRole + 4



from views.tree_icons import layer_swatch, none_swatch, swatch_icon


def _submenu(menu, title):
    """A submenu parented to ``menu``. ``menu.addMenu(title)`` makes one
    Python holds no reference to, and inside a helper it can be deleted
    with the helper's locals before it ever opens."""
    from PySide6.QtWidgets import QMenu
    sub = QMenu(title, menu)
    menu.addMenu(sub)
    return sub


def _pencil_icon(on: bool) -> QIcon:
    """The active layer's mark: the toolbar's «Make active» pencil, drawn
    in the chosen accent on the active row and as a faint outline on the
    others (where clicking makes it active)."""
    from PySide6.QtWidgets import QApplication
    from views.icons import pencil_glyph
    from views.theme import accent_color
    ink = QApplication.palette().text().color()
    pm = QPixmap(18, 18)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    # The glyph's 16.5-unit box in the 48 canvas onto the 18 px cell.
    k = 15.0 / 16.5
    p.translate(1.5, 1.5)
    p.scale(k, k)
    p.translate(-27, -25.5)
    if on:
        pen = QPen(ink, 1.6 / k)
        p.setBrush(accent_color())
    else:
        faint = QColor(ink)
        faint.setAlpha(80)
        pen = QPen(faint, 1.2 / k)
        p.setBrush(Qt.NoBrush)
    pen.setJoinStyle(Qt.RoundJoin)
    pen.setCapStyle(Qt.RoundCap)
    p.setPen(pen)
    pencil_glyph(p)
    p.end()
    return QIcon(pm)


def printer_icon(on: bool) -> QIcon:
    """The print mark of a row: a printer in ink when the layer goes onto
    the sheets; faint and struck through when it stays off them."""
    from PySide6.QtWidgets import QApplication
    ink = QColor(QApplication.palette().text().color())
    if not on:
        ink.setAlpha(110)
    pm = QPixmap(18, 18)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    pen = QPen(ink, 1.2)
    pen.setJoinStyle(Qt.RoundJoin)
    p.setPen(pen)
    p.setBrush(Qt.NoBrush)
    p.drawRect(QRectF(5.5, 2.5, 7, 4))              # the paper going in
    p.drawRoundedRect(QRectF(2.5, 6.5, 13, 6), 1.5, 1.5)   # the body
    p.drawRect(QRectF(5.5, 10.5, 7, 5))             # the sheet coming out
    if not on:
        strike = QPen(ink, 1.6)
        strike.setCapStyle(Qt.RoundCap)
        p.setPen(strike)
        p.drawLine(QPointF(2.5, 15.5), QPointF(15.5, 2.5))
    p.end()
    return QIcon(pm)


def layers_signature(scene) -> tuple:
    """Everything the Layers rows are drawn from — each layer's own state,
    the folders and the active layer. The same value
    twice means the rows would come out the same: an edit of the model
    (a push, a move) leaves it as it was and does not rebuild the list."""
    folders = tuple(L.folders_of(scene))
    known = set(folders)
    return (tuple(repr(sorted(L.layer_values(ly).items())) for ly in scene.layers),
            folders, tuple(L.folder_parent(scene, f, known=known) for f in folders),
            tuple(L.folder_label(scene, f) for f in folders),
            tuple(sorted(scene.folder_visibility.items())),
            L.active_layer(scene))


#: The orders of the panel's rows: by name, as they were made, the visible
#: ones first, the locked ones first (the «⋯» menu; remembered).
SORT_MODES = ("name", "created", "visible", "locked")

#: The orders that group the rows by a state, and the column holding it.
_STATE_SORTS = {"visible": COL_EYE, "locked": COL_LOCK}


class _Row(QTreeWidgetItem):
    """A row that sorts the way the panel reads: the default layer first,
    then the folders, then the layers — whichever way the column runs."""

    def __lt__(self, other) -> bool:
        tree = self.treeWidget()
        col = tree.sortColumn() if tree is not None else COL_NAME
        asc = (tree is None
               or tree.header().sortIndicatorOrder() == Qt.AscendingOrder)
        a, b = self._rank(), other._rank()
        if a != b:
            # Ranks hold in both directions: undo the reversal Qt applies.
            return (a < b) if asc else (a > b)
        panel = getattr(tree, "_panel", None)
        how = getattr(panel, "sort_by", "name") if col == COL_NAME else ""
        if how in _STATE_SORTS:
            # Visible (or locked) rows first, then the rest — a group that
            # holds both ways, like the ranks; by name inside each group.
            state_col = _STATE_SORTS[how]
            sa = bool(self.data(state_col, Qt.UserRole + 1))
            sb = bool(other.data(state_col, Qt.UserRole + 1))
            if sa != sb:
                return sa if asc else sb
        if how == "created":
            oa, ob = self.data(COL_NAME, ROLE_ORDER), other.data(COL_NAME,
                                                                 ROLE_ORDER)
            if oa is not None and ob is not None and oa != ob:
                return oa < ob
        ka, kb = self.data(col, Qt.UserRole + 1), other.data(col, Qt.UserRole + 1)
        if ka is None or kb is None or ka == kb:
            return (natural_key(self.text(COL_NAME))
                    < natural_key(other.text(COL_NAME)))
        return ka < kb

    def _rank(self) -> int:
        kind, name = self.data(COL_NAME, Qt.UserRole) or (KIND_LAYER, "")
        if kind == KIND_LAYER and name == L.DEFAULT_LAYER:
            return 0
        return 1 if kind == KIND_FOLDER else 2


class _HoldsDelegate(QStyledItemDelegate):
    """The name column with the selection's mark: a bar of the accent at
    the row's left edge (the row's tint and its name in bold do the
    rest)."""

    def paint(self, painter, option, index) -> None:
        super().paint(painter, option, index)
        if index.data(ROLE_HOLDS):
            from PySide6.QtWidgets import QApplication
            r = option.rect
            painter.fillRect(r.left(), r.top() + 1, 4, r.height() - 2,
                             QApplication.palette().highlight())


#: Room kept between a dropped list and the panel's sides, in pixels.
MENU_MARGIN = 6


def place_menu(menu, anchor, inside):
    """Where ``menu`` drops under ``anchor`` (a global QRect): always the
    same way — its right edge on the anchor's, so every list of the panel
    opens alike — and kept inside ``inside`` (the panel, global) with a
    small margin, never out to the screen's edge; above the anchor when the
    screen has no room below."""
    size = menu.sizeHint()
    x = anchor.right() + 1 - size.width()
    x = min(x, inside.right() + 1 - MENU_MARGIN - size.width())
    x = max(x, inside.left() + MENU_MARGIN)
    y = anchor.bottom() + 1
    screen = (QApplication.screenAt(anchor.center())
              or QApplication.primaryScreen())
    avail = screen.availableGeometry()
    if y + size.height() > avail.bottom():
        y = max(avail.top(), anchor.top() - size.height())
    return QPoint(x, y)


def global_rect(widget, rect=None):
    """``rect`` (``widget``'s whole rect by default) in screen coordinates."""
    from PySide6.QtCore import QRect
    r = widget.rect() if rect is None else rect
    return QRect(widget.mapToGlobal(r.topLeft()), r.size())


def line_pen(key: str, ink, width: float, px_per_mm: float,
             pattern=None) -> QPen:
    """A pen in the dash pattern of line type ``key`` — or ``pattern``,
    given (a custom one) — in millimetres on paper shown at ``px_per_mm``
    screen pixels each."""
    pen = QPen(ink, width)
    pen.setCapStyle(Qt.FlatCap)
    if pattern is None:
        pattern = L.LINETYPES.get(key, ())
    # A dot is a dash as long as the pen is wide; Qt wants every piece > 0.
    pattern = [abs(mm) if mm else width / px_per_mm for mm in pattern]
    if pattern:
        pen.setDashPattern([max(0.3, mm * px_per_mm / width)
                            for mm in pattern])
    return pen


#: The swatch box shared by the colour and the hatch columns, in px of an
#: 18 px icon: the same rounded rectangle, so both columns read alike.
SWATCH_BOX = QRectF(2.5, 3.5, 13, 11)


def hatch_icon(key, ground=None, ink=None, angle: float = 0.0,
               families=None) -> QIcon:
    """A row's hatch swatch (the Layers panel's Hatch column) — the very
    shape of the colour swatch beside it: the pattern inside the rounded
    box, on the layer's ground colour and in its ink (RGB 0–1) when it has
    them, turned as the layer turns it; ``families``: an imported pattern's.
    No pattern: the «none» swatch."""
    if not key:
        return none_swatch()
    from PySide6.QtGui import QPainterPath
    from views.hatch_paint import paint_hatch
    app = QApplication.instance()
    dpr = app.devicePixelRatio() if app is not None else 1.0
    pal = QApplication.palette()
    pm = QPixmap(int(18 * dpr), int(18 * dpr))
    pm.setDevicePixelRatio(dpr)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    box = QPainterPath()
    box.addRoundedRect(SWATCH_BOX, 2, 2)
    p.setPen(Qt.NoPen)
    p.setBrush(QColor.fromRgbF(*ground[:3]) if ground is not None
               else QColor(255, 255, 255))
    p.drawPath(box)
    # The pattern at 2 px per paper mm: a few repeats in the little box.
    k = 2.0
    p.save()
    p.scale(k, k)
    inner = QPainterPath()
    inner.addRoundedRect(QRectF(SWATCH_BOX.x() / k, SWATCH_BOX.y() / k,
                                SWATCH_BOX.width() / k,
                                SWATCH_BOX.height() / k), 1, 1)
    paint_hatch(p, inner, key,
                QColor.fromRgbF(*ink[:3]) if ink is not None
                else QColor(30, 36, 44), 0.9 / k, 1.0, None, angle, families)
    p.restore()
    p.setPen(QPen(pal.text().color(), 1.0))
    p.setBrush(Qt.NoBrush)
    p.drawRoundedRect(SWATCH_BOX, 2, 2)
    p.end()
    return QIcon(pm)


class _LineDelegate(QStyledItemDelegate):
    """The line-type column: a stroke across the cell in the layer's dash
    pattern, as the sheets draw its lines."""

    #: Screen pixels per paper millimetre of the dash pattern.
    PX_PER_MM = 2.0

    def paint(self, painter, option, index) -> None:
        super().paint(painter, option, index)
        key = index.data(ROLE_LINETYPE)
        if key is None:
            return
        own = index.data(ROLE_LINEPATTERN)
        pattern = list(own) if own else list(L.LINETYPES.get(key, ()))
        from PySide6.QtWidgets import QApplication, QStyle
        pal = QApplication.palette()
        ink = (pal.highlightedText() if option.state & QStyle.State_Selected
               else pal.text()).color()
        r = option.rect
        # One whole repeat at least, however narrow the cell: a long-dash
        # pattern drawn at full scale would show a single dash and read as
        # any other.
        period = sum(abs(v) for v in pattern)
        room = max(1.0, r.width() - 10.0)
        ppm = (min(self.PX_PER_MM, room / (1.5 * period)) if period
               else self.PX_PER_MM)
        pen = line_pen(key, ink, 1.6, ppm, pattern)
        y = r.center().y() + 0.5
        painter.save()
        painter.setRenderHint(QPainter.Antialiasing)
        painter.setPen(pen)
        painter.drawLine(QPointF(r.left() + 5, y), QPointF(r.right() - 5, y))
        painter.restore()


class _LayerTree(SideTree):
    """Classic layer list with row-wide selection and cell actions."""

    #: The cells a press acts on directly, as in the Browser: the row's
    #: selection is left as it is, so a click on the eye of one of several
    #: selected rows acts on all of them.
    CELLS = (COL_ACTIVE, COL_EYE, COL_LOCK, COL_ISOLATE, COL_COLOR,
             COL_LINETYPE, COL_WEIGHT, COL_OPACITY, COL_PRINT, COL_CUT,
             COL_HATCH)


class LayersPanel(QWidget):
    """The layer tree, its filter and its buttons. See the module doc."""

    def __init__(self, window) -> None:
        super().__init__()
        self._window = window
        self._updating = False
        self._collapsed: set[str] = set()
        lay = QVBoxLayout(self)
        lay.setContentsMargins(6, 6, 6, 6)
        lay.setSpacing(4)

        bar = QHBoxLayout()
        bar.addStretch(1)
        # Expand / collapse every folder, as the Browser's. Isolating is per
        # row (the target beside the padlock), as in the Layers drop-down.
        #: What Isolate hid, to show back exactly that on the way out.
        # Drawn icons, not text glyphs: the same size and ink as the rest
        # of the bar.
        self.expand_btn = self._tool_button(
            QIcon(), tr("Expand all"), self._expand_or_collapse)
        bar.addWidget(self.expand_btn)
        from views.icons import tool_icon
        self.more_btn = QToolButton()
        self.more_btn.setAutoRaise(True)
        self.more_btn.setIcon(tool_icon("layers_templates"))
        self.more_btn.setToolTip(tr("Sort by"))
        self.more_menu = QMenu(self.more_btn)
        self.more_btn.clicked.connect(self._open_more_menu)
        bar.addWidget(self.more_btn)
        lay.addLayout(bar)

        self.tree = _LayerTree(self)
        self.tree.setDragDropMode(QAbstractItemView.NoDragDrop)
        self.tree.setColumnCount(11)
        self.tree.setHeaderLabels([tr("Name"), "", "", "", "", "#", "",
                                   tr("Line"), "mm", "%", "", tr("Cut pen"),
                                   ""])
        head = self.tree.header()
        head.setStretchLastSection(False)
        head.setSectionResizeMode(COL_NAME, QHeaderView.Stretch)
        for col, width in ((COL_ACTIVE, 26), (COL_EYE, 26), (COL_LOCK, 26),
                           (COL_ISOLATE, 26), (COL_COLOR, 26),
                           (COL_LINETYPE, 40), (COL_WEIGHT, 40),
                           (COL_CUT, 44), (COL_HATCH, 26),
                           (COL_OPACITY, 40), (COL_PRINT, 26),
                           (COL_COUNT, 44)):
            head.setSectionResizeMode(col, QHeaderView.Fixed)
            if col == COL_CUT:
                # Its title whole, in any language («Cut pen», «Corte»).
                width = max(width, head.fontMetrics().horizontalAdvance(
                    tr("Cut pen")) + 18)
            head.resizeSection(col, width)
        hi = self.tree.headerItem()
        self._header_icons()
        hi.setToolTip(COL_ACTIVE, tr(
            "Active layer: new groups, components, dimensions, texts and "
            "guides go to it"))
        hi.setToolTip(COL_EYE, tr("Visible"))
        hi.setToolTip(COL_LOCK, tr("Locked: drawn, but it cannot be picked"))
        hi.setToolTip(COL_ISOLATE, tr(
            "Isolate — a row's target shows only that layer, a second click "
            "shows back what it hid; click here to end the isolation"))
        hi.setToolTip(COL_COLOR, tr("Colour"))
        hi.setToolTip(COL_LINETYPE, tr(
            "Line type the layer's lines take on the sheets and in DXF — "
            "click a row's line to change it"))
        hi.setToolTip(COL_WEIGHT, tr(
            "Line weight in mm (—: default, from the sheet) — click a "
            "row's to change it"))
        hi.setTextAlignment(COL_WEIGHT, Qt.AlignCenter)
        hi.setToolTip(COL_CUT, tr(
            "Cut weight in mm: the pen of the layer's section cut on the "
            "sheets (—: the sheet's cut pen) — click a row's to change it"))
        hi.setTextAlignment(COL_CUT, Qt.AlignCenter)
        # Like the colour column: no word, a swatch — here, a hatched one.
        hi.setIcon(COL_HATCH, hatch_icon("diagonal"))
        hi.setToolTip(COL_HATCH, tr(
            "Hatch of the layer's section cut on the sheets — brick, "
            "concrete, insulation… (a red slash: the frame's fill) — click a "
            "row's to pick one"))
        hi.setIcon(COL_PRINT, printer_icon(True))
        hi.setToolTip(COL_PRINT, tr(
            "Print — a layer switched off here is seen in the model but left "
            "out of the sheets, their PDF and DXF; click here for every "
            "layer"))
        hi.setToolTip(COL_OPACITY, tr(
            "Transparency of the layer's faces in the model (—: opaque) — "
            "click a row's to change it"))
        hi.setTextAlignment(COL_OPACITY, Qt.AlignCenter)
        hi.setToolTip(COL_COUNT, tr(
            "Entities on the layer: faces, edges, objects, dimensions, texts, "
            "guides and images"))
        # As in the Browser: eye and padlock first (then the isolate
        # target, as in the drop-down), the name next (the tree lines and
        # folds hang from it), then the rest.
        for visual, logical in enumerate(self.COLUMN_ORDER):
            head.moveSection(head.visualIndex(logical), visual)
        self.tree.setTreePosition(COL_NAME)
        self.tree.setIndentation(self.INDENT)
        hi.setToolTip(COL_EYE, tr("Visible — click here to show or hide "
                                  "every layer"))
        hi.setToolTip(COL_LOCK, tr("Locked — click here to lock or unlock "
                                   "every layer"))
        head.setSectionsClickable(True)
        head.setSortIndicatorShown(True)
        head.sectionClicked.connect(self._on_header_clicked)
        self._sort = (COL_NAME, Qt.AscendingOrder)
        self.tree.setUniformRowHeights(True)
        self.tree.setAlternatingRowColors(True)
        self.tree.setSelectionMode(QAbstractItemView.ExtendedSelection)
        self.tree.setEditTriggers(QAbstractItemView.DoubleClicked
                                  | QAbstractItemView.EditKeyPressed)
        # Sorted by hand: a click on the eye or padlock header acts on every
        # layer (the Browser's rule), the other headers sort.
        self.tree.setSortingEnabled(False)
        # Qt makes the header unclickable when sorting is switched off:
        # clickable again, or the eye and padlock headers do nothing.
        head.setSectionsClickable(True)
        self.tree.sortItems(COL_NAME, Qt.AscendingOrder)
        self.tree.itemChanged.connect(self._on_item_changed)
        names = _HoldsDelegate(self.tree)
        self.tree.setItemDelegateForColumn(COL_NAME, names)
        self.tree.setItemDelegateForColumn(COL_LINETYPE,
                                           _LineDelegate(self.tree))
        self.tree.itemExpanded.connect(self._on_expanded)
        self.tree.itemCollapsed.connect(self._on_collapsed)
        self.tree.setContextMenuPolicy(Qt.CustomContextMenu)
        self.tree.customContextMenuRequested.connect(self._on_context_menu)
        lay.addWidget(self.tree, 1)

        # No row of buttons under the list (the Browser has none): delete
        # is the Delete key and the right-click menu, assigning is the
        # Layers toolbar's drop-down and the right-click menu.

        for col in (COL_WEIGHT, COL_PRINT, COL_OPACITY, COL_COUNT, COL_CUT):
            self.tree.setColumnHidden(col, True)
        self.refresh()

    def _header_icons(self) -> None:
        """The header's pencil, eye, padlock and target, in the palette's
        ink."""
        hi = self.tree.headerItem()
        self._drawn_ink = QApplication.palette().text().color().rgba()
        hi.setIcon(COL_ACTIVE, _pencil_icon(True))
        hi.setIcon(COL_EYE, row_icon("eye"))
        hi.setIcon(COL_LOCK, row_icon("lock"))
        hi.setIcon(COL_ISOLATE, isolate_icon(False))

    def changeEvent(self, event) -> None:
        """The theme changed while the app runs (Preferences ▸ theme): the
        icons are drawn in the palette's ink, so the rows, the header and
        the bar are drawn again at once — not black on the dark theme until
        the list happens to rebuild."""
        super().changeEvent(event)
        from PySide6.QtCore import QEvent
        if event.type() not in (QEvent.PaletteChange,
                                QEvent.ApplicationPaletteChange):
            return
        ink = QApplication.palette().text().color().rgba()
        if ink == getattr(self, "_drawn_ink", ink)                 or getattr(self, "_retheming", False):
            return
        self._retheming = True
        try:
            self.retheme()
        finally:
            self._retheming = False

    def retheme(self) -> None:
        from views.icons import tool_icon
        self._header_icons()
        self.more_btn.setIcon(tool_icon("layers_templates"))
        self._update_expand_button()
        self.refresh()

    @staticmethod
    def _tool_button(icon, tip: str, slot) -> QToolButton:
        btn = QToolButton()
        btn.setIcon(icon)
        btn.setToolTip(tip)
        btn.setAutoRaise(True)
        btn.clicked.connect(slot)
        return btn

    # ---- Model → view --------------------------------------------------------
    def _scene(self):
        return self._window.viewport.scene

    @staticmethod
    def _key(item):
        return item.data(COL_NAME, Qt.UserRole) or (KIND_LAYER, "")

    def refresh(self) -> None:
        """Rebuild the rows, keeping the selection, folded folders and filter."""
        # This rebuild covers whatever the deferred one was waiting for.
        timer = getattr(self, "_refresh_timer", None)
        if timer is not None and timer.isActive():
            timer.stop()
        scene = self._scene()
        selected = {self._key(it) for it in self.tree.selectedItems()}
        current = self.tree.currentItem()
        current_key = self._key(current) if current is not None else None
        self._updating = True
        scroll = (self.tree.horizontalScrollBar().value(), self.tree.verticalScrollBar().value())
        self.tree.clear()
        folders = {}
        for order, name in enumerate(L.folders_of(scene)):
            # Rows know folders and tags by KEY; the text is the label.
            item = _Row([L.folder_label(scene, name)])
            item.setData(COL_NAME, Qt.UserRole, (KIND_FOLDER, name))
            item.setData(COL_NAME, ROLE_ORDER, order)
            item.setIcon(COL_NAME, row_icon("folder"))
            item.setFlags(item.flags() | Qt.ItemIsEditable)
            folders[name] = item
        for name, item in folders.items():
            parent = folders.get(L.folder_parent(scene, name, known=folders))
            if parent is not None:
                parent.addChild(item)
            else:
                self.tree.addTopLevelItem(item)
        active = L.active_layer(scene)
        self._alone = self.isolated_layers()         # whose target is filled
        folder_layers = {name: [] for name in folders}
        for order, ly in enumerate(scene.layers):
            item = self._new_layer_row(ly, order, active)
            parent = folders.get(ly.folder)
            if parent is not None:
                parent.addChild(item)
            else:
                self.tree.addTopLevelItem(item)
            while parent is not None:
                folder_layers[self._key(parent)[1]].append(ly)
                parent = parent.parent()
        for name, item in folders.items():
            self._paint_folder_row(item, folder_layers[name])
            item.setExpanded(name not in self._collapsed)
        self._mark_selection()
        for it in self._all_items():
            key = self._key(it)
            if key in selected:
                it.setSelected(True)
            if key == current_key:
                self.tree.setCurrentItem(it, 0, QItemSelectionModel.NoUpdate)
        col, order = getattr(self, "_sort", (COL_NAME, Qt.AscendingOrder))
        self.tree.sortItems(col, order)
        self._update_expand_button()
        self._updating = False
        self._drawn_signature = self._signature()
        self._drawn_structure = self._structure()
        self.tree.horizontalScrollBar().setValue(scroll[0])
        self.tree.verticalScrollBar().setValue(scroll[1])

    def refresh_icons(self) -> None:
        """Repaint the marks drawn in the palette's ink and accent after the
        theme or accent changes: the header's and every row's (#1086)."""
        hi = self.tree.headerItem()
        hi.setIcon(COL_ACTIVE, _pencil_icon(True))
        hi.setIcon(COL_EYE, row_icon("eye"))
        hi.setIcon(COL_LOCK, row_icon("lock"))
        hi.setIcon(COL_ISOLATE, isolate_icon(False))
        hi.setIcon(COL_PRINT, printer_icon(True))
        scene = self._scene()
        active = L.active_layer(scene)
        was = self._updating
        self._updating = True
        for item in self._all_items():
            kind, name = self._key(item)
            if kind == KIND_LAYER:
                ly = scene.layer(name)
                if ly is not None:
                    self._paint_layer_row(item, ly, name == active)
            else:
                self._paint_folder_row(item)
        self._updating = was

    #: Column order on screen (logical columns), and the indent per level —
    #: the Browser's.
    COLUMN_ORDER = (COL_EYE, COL_LOCK, COL_ISOLATE, COL_NAME, COL_ACTIVE,
                    COL_COLOR, COL_LINETYPE, COL_HATCH, COL_WEIGHT,
                    COL_CUT, COL_OPACITY, COL_PRINT, COL_COUNT)
    INDENT = 12

    def _on_header_clicked(self, col: int) -> None:
        """Eye or padlock header: every layer shown or hidden, locked or
        unlocked (all of them hidden or locked → all back). Any other
        header: sort by it, twice to turn the order."""
        scene = self._scene()
        names = [ly.key for ly in scene.layers]
        if col == COL_ISOLATE:
            from ingetrazo_render import isolate_tags
            isolate_tags(self._window.viewport, [])
            return
        if col == COL_PRINT:
            self.set_printable(names, not all(
                getattr(ly, "printable", True) for ly in scene.layers))
            return
        old_col, old_order = self._sort
        order = (Qt.DescendingOrder if (old_col == col
                                        and old_order == Qt.AscendingOrder)
                 else Qt.AscendingOrder)
        self._sort = (col, order)
        self.tree.sortItems(col, order)

    # ---- Isolate (the target column) -------------------------------------------
    def isolated_layers(self) -> set[str]:
        scene = self._scene()
        active, _ = getattr(scene, "_drawing_tag_isolation", None) or ((), {})
        return {ly.key for ly in scene.layers if ly.uid in active}

    # ---- ⊞ expand / ⊟ collapse ----------------------------------------------------
    def _folders_all_open(self) -> bool:
        return all(it.isExpanded() for it in self._all_items()
                   if self._key(it)[0] == KIND_FOLDER)

    def _update_expand_button(self) -> None:
        b = getattr(self, "expand_btn", None)
        if b is None:
            return
        has = any(self._key(it)[0] == KIND_FOLDER for it in self._all_items())
        b.setEnabled(has)
        opened = has and self._folders_all_open()
        from views.icons import tool_icon
        b.setIcon(tool_icon("layers_collapse" if opened
                            else "layers_expand"))
        b.setProperty("opened", opened)
        b.setToolTip(tr("Collapse all") if opened else tr("Expand all"))

    def _expand_or_collapse(self) -> None:
        open_ = not self._folders_all_open()
        for it in self._all_items():
            if self._key(it)[0] == KIND_FOLDER:
                it.setExpanded(open_)
        self._update_expand_button()

    def _on_expanded(self, item) -> None:
        if self._updating:
            return
        self._collapsed.discard(self._key(item)[1])
        if not self._updating:
            self._mark_selection()
            self._update_expand_button()

    def _on_collapsed(self, item) -> None:
        if self._updating:
            return
        self._collapsed.add(self._key(item)[1])
        if not self._updating:
            self._mark_selection()
            self._update_expand_button()

    def _mark_selection(self) -> None:
        """Tint the rows of the layers what is selected sits on — every one
        of them when it spans several — and a folded folder holding one, so
        the mark is always in view. Nothing selected: no mark. It never
        changes the active layer: what you draw next stays where it was."""
        from PySide6.QtWidgets import QApplication
        names = L.selection_layers(self._scene())
        if not names and getattr(self, "_marked_names", None) == names:
            return
        # Strong enough to find at a glance — next to a row picked in the
        # list, a faint tint went unseen — yet lighter than that pick.
        tint = QColor(QApplication.palette().highlight().color())
        tint.setAlpha(105)
        was = self._updating
        self._updating = True
        marked = set()
        for it in self._all_items():
            kind, name = self._key(it)
            if kind == KIND_LAYER and name in names:
                marked.add(id(it))
                parent = it.parent()
                while parent is not None:
                    if not parent.isExpanded():
                        marked.add(id(parent))
                    parent = parent.parent()
        first = None
        for it in self._all_items():
            on = id(it) in marked
            it.setData(COL_NAME, ROLE_HOLDS, on)
            for col in range(self.tree.columnCount()):
                it.setBackground(col, QBrush(tint) if on else QBrush())
            font = it.font(COL_NAME)
            if font.bold() != on:
                font.setBold(on)
                it.setFont(COL_NAME, font)
            if on and first is None:
                first = it
        # In view: the layer of what was just picked in the model.
        if first is not None and names != getattr(self, "_marked_names",
                                                  None):
            self.tree.scrollToItem(first)
        self._marked_names = names
        self._updating = was

    def _dim_eye(self, item, dim):
        """A tag (or folder) hidden by a folder above it: its own eye,
        faded. QIcon.Disabled looks the same as Normal on the dark theme,
        so the faded copy is painted here."""
        if dim:
            from PySide6.QtCore import QSize
            from PySide6.QtGui import QPainter, QPixmap
            icon = item.icon(COL_EYE)
            ratio = self.devicePixelRatioF() or 1.0
            src = icon.pixmap(QSize(20, 20), ratio)
            faded = QPixmap(src.size())
            faded.setDevicePixelRatio(src.devicePixelRatio())
            faded.fill(Qt.transparent)
            painter = QPainter(faded)
            painter.setOpacity(0.3)
            painter.drawPixmap(0, 0, src)
            painter.end()
            item.setIcon(COL_EYE, QIcon(faded))
        item.setData(COL_EYE, Qt.UserRole + 2, dim)

    def _structure(self):
        scene = self._scene()
        folders = L.folders_of(scene)
        known = set(folders)
        return (tuple((ly.key, ly.folder) for ly in scene.layers),
                tuple((f, L.folder_parent(scene, f, known=known)) for f in folders))

    def _update_rows(self):
        scene = self._scene()
        self._updating = True
        self._alone = self.isolated_layers()
        for item in self._all_items():
            kind, key = self._key(item)
            if kind == KIND_LAYER:
                ly = scene.layer_by_key(key)
                item.setText(COL_NAME, ly.name)
                self._paint_layer_row(item, ly, key == L.active_layer(scene))
            else:
                item.setText(COL_NAME, L.folder_label(scene, key))
                self._paint_folder_row(item)
        self._updating = False
        self._drawn_signature = self._signature()
        col, order = self._sort
        self.tree.sortItems(col, order)
        self._mark_selection()

    def _new_layer_row(self, ly, order, active):
        item = _Row([ly.name])
        item.setData(COL_NAME, Qt.UserRole, (KIND_LAYER, ly.key))
        item.setData(COL_NAME, ROLE_ORDER, order)
        if ly.key != L.DEFAULT_LAYER:
            item.setFlags(item.flags() | Qt.ItemIsEditable)
        self._paint_layer_row(item, ly, ly.key == active)
        return item

    def _paint_layer_row(self, item, ly, active: bool) -> None:
        item.setIcon(COL_ACTIVE, _pencil_icon(active))
        item.setToolTip(COL_ACTIVE, tr("Active layer") if active
                        else tr("Make this the active layer"))
        item.setIcon(COL_EYE, row_icon("eye" if ly.visible else "eye_off"))
        item.setData(COL_EYE, Qt.UserRole + 1, int(ly.visible))
        self._dim_eye(item, not L.folder_visible(self._scene(), ly.folder))
        item.setIcon(COL_LOCK, row_icon("lock" if ly.locked else "lock_open"))
        item.setData(COL_LOCK, Qt.UserRole + 1, int(ly.locked))
        item.setIcon(COL_COLOR, layer_swatch(ly))
        item.setToolTip(COL_COLOR, tr("Click to pick the layer's colour"))
        keys = [k for k, _l in self.LINETYPE_LABELS]
        own = ly.linetype.startswith(L.CUSTOM_LINETYPE)
        label = (ly.linetype[len(L.CUSTOM_LINETYPE):] if own
                 else tr(dict(self.LINETYPE_LABELS).get(ly.linetype,
                                                        "Continuous")))
        item.setData(COL_LINETYPE, ROLE_LINETYPE, ly.linetype)
        item.setData(COL_LINETYPE, ROLE_LINEPATTERN,
                     list(L.linetype_pattern(ly.linetype, self._scene()))
                     if own else None)
        item.setData(COL_LINETYPE, Qt.UserRole + 1,
                     keys.index(ly.linetype) if ly.linetype in keys
                     else len(keys))
        item.setToolTip(COL_LINETYPE, label)
        weight = ly.lineweight
        item.setText(COL_WEIGHT, "—" if weight is None else f"{weight:.2f}")
        item.setTextAlignment(COL_WEIGHT, Qt.AlignCenter)
        item.setData(COL_WEIGHT, Qt.UserRole + 1,
                     -1.0 if weight is None else float(weight))
        item.setToolTip(COL_WEIGHT, tr("Default (from the sheet)") if weight is None
                        else f"{weight:.2f} mm")
        cut = getattr(ly, "cut_weight", None)
        item.setText(COL_CUT, "—" if cut is None else f"{cut:.2f}")
        item.setTextAlignment(COL_CUT, Qt.AlignCenter)
        item.setData(COL_CUT, Qt.UserRole + 1,
                     -1.0 if cut is None else float(cut))
        item.setToolTip(COL_CUT, tr("The sheet's cut pen") if cut is None
                        else f"{tr('Cut pen')} {cut:.2f} mm")
        hatch = getattr(ly, "hatch", None)
        from core.hatches import HATCH_LABELS, HATCH_ORDER
        scene = self._scene()
        item.setIcon(COL_HATCH, hatch_icon(
            hatch, L.hatch_ground_rgb(ly), L.hatch_ink_rgb(ly),
            getattr(ly, "hatch_angle", 0.0), L.layer_hatch_families(scene, ly)))
        item.setData(COL_HATCH, Qt.UserRole + 1,
                     -1 if hatch is None else HATCH_ORDER.index(hatch)
                     if hatch in HATCH_ORDER else len(HATCH_ORDER))
        item.setToolTip(COL_HATCH, tr("The frame's fill") if hatch is None
                        else tr(HATCH_LABELS[hatch]) if hatch in HATCH_LABELS
                        else L.hatch_label(scene, hatch))
        printed = getattr(ly, "printable", True)
        item.setIcon(COL_PRINT, printer_icon(printed))
        item.setData(COL_PRINT, Qt.UserRole + 1, int(printed))
        item.setToolTip(COL_PRINT, tr("On the sheets — click to leave it "
                                      "out of them") if printed else
                        tr("Not printed: seen in the model, left out of the "
                           "sheets — click to print it"))
        clear = round((1.0 - ly.opacity) * 100)
        item.setText(COL_OPACITY, f"{clear} %" if clear else "—")
        item.setTextAlignment(COL_OPACITY, Qt.AlignCenter)
        item.setData(COL_OPACITY, Qt.UserRole + 1, clear)
        item.setToolTip(COL_OPACITY, tr("Opaque") if not clear else
                        f"{tr('Transparency')} {clear} %")
        if ly.key != L.DEFAULT_LAYER:               # always on: no target
            alone = ly.key in self._alone
            item.setIcon(COL_ISOLATE, isolate_icon(alone))
            item.setToolTip(COL_ISOLATE, tr(
                "Shown alone — click to show back what Isolate hid")
                if alone else tr("Isolate: show only this layer"))
        # What no column shows, on the name: the folder — and the
        # transparency while its column is off.
        look = []
        if ly.opacity < 1.0 and self.tree.isColumnHidden(COL_OPACITY):
            look.append(f"{tr('Transparency')} {round((1 - ly.opacity) * 100)} %")
        if ly.folder:
            look.append(L.folder_label(self._scene(), ly.folder))
        item.setToolTip(COL_NAME, "\n".join([ly.name, " · ".join(look)])
                        if look else ly.name)

    def _paint_folder_row(self, item, layers=None) -> None:
        scene = self._scene()
        if layers is None:
            layers = [scene.layer(n) for n in L.layers_in_folder(scene, self._key(item)[1])]
            layers = [ly for ly in layers if ly is not None]
        key = self._key(item)[1]
        shown = L.folder_visible(scene, key, ancestors=False)
        locked = bool(layers) and all(ly.locked for ly in layers)
        item.setIcon(COL_EYE, row_icon("eye" if shown else "eye_off"))
        item.setIcon(COL_LOCK, row_icon("lock" if locked else "lock_open"))
        # Read by «Sort by visible / locked», as a layer row's are.
        item.setData(COL_EYE, Qt.UserRole + 1, int(shown))
        item.setData(COL_LOCK, Qt.UserRole + 1, int(locked))
        item.setToolTip(COL_EYE, tr("Show or hide the folder"))
        self._dim_eye(item, not L.folder_visible(scene, self._key(item.parent())[1] if item.parent() else None))
        printed = (not layers
                   or any(getattr(ly, "printable", True) for ly in layers))
        item.setIcon(COL_PRINT, printer_icon(printed))
        names = {ly.key for ly in layers} - {L.DEFAULT_LAYER}
        alone = bool(names) and names == self._alone
        item.setIcon(COL_ISOLATE, isolate_icon(alone))
        item.setToolTip(COL_ISOLATE, tr(
            "Shown alone — click to show back what Isolate hid")
            if alone else tr("Isolate: show only the layers in this folder"))

    def _all_items(self):
        """Every row, folders inside folders included, depth first."""
        out = []

        def walk(item):
            out.append(item)
            for c in self._children(item):
                walk(c)

        for i in range(self.tree.topLevelItemCount()):
            walk(self.tree.topLevelItem(i))
        return out

    @staticmethod
    def _children(item):
        return [item.child(i) for i in range(item.childCount())]

    def _folder_path(self, key) -> str:
        """A folder's labels from the top down to it — «Deck/Steel» — so
        two folders labelled alike read apart in a menu."""
        scene = self._scene()
        parts, seen = [], set()
        while key and key not in seen:
            seen.add(key)
            parts.append(L.folder_label(scene, key))
            key = L.folder_parent(scene, key)
        return "/".join(reversed(parts))

    def item_for(self, name: str, kind: str = KIND_LAYER):
        return next((it for it in self._all_items()
                     if self._key(it) == (kind, name)), None)

    # ---- Selection helpers ---------------------------------------------------
    def _layers_under(self, items) -> list[str]:
        """Layer names of the rows ``items`` — a folder row stands for every
        layer in it — in order, each once."""
        out: list[str] = []
        for it in items:
            kind, name = self._key(it)
            names = (L.layers_in_folder(self._scene(), name)
                     if kind == KIND_FOLDER else [name])
            for n in names:
                if n not in out:
                    out.append(n)
        return out

    def _targets(self, item) -> list[str]:
        """The rows a click on ``item`` acts on: every selected row when
        it is one of them, else just it."""
        selected = self.tree.selectedItems()
        return self._layers_under(selected if item in selected else [item])

    def selected_layers(self) -> list[str]:
        return [n for n in self._layers_under(self.tree.selectedItems())]

    # ---- View → model --------------------------------------------------------
    def _touch(self, rebuild: bool = True) -> None:
        scene = self._scene()
        scene.version += 1
        # Announce first: the listeners' rebuild is deferred, and the one
        # below covers it (``refresh`` drops a pending one) — the row whose
        # name is about to be typed must not be rebuilt under the editor.
        self._window.viewport.notify_scene_changed()
        if rebuild:
            self._refresh_after_change()

    def _released(self, released: bool) -> None:
        if released:
            self._window.statusBar().showMessage(tr(
                "The active layer is hidden or locked: new objects go to "
                "'{default}' again.", default=L.DEFAULT_LAYER), 4000)

    def _on_item_clicked(self, item, column) -> None:
        """A click on one of the cells, for code and tests: the tree itself
        acts on the press (:class:`views.side_tree.SideTree`). The action
        runs once Qt is done: it rebuilds the tree."""
        self.cell_clicked(item, column)

    def cell_clicked(self, item, column) -> None:
        """A press on one of the tree's cells — the eye, the padlock or the
        target: the panel answers it, and the selection is left as it was,
        so it acts on every selected row. The row and its targets are read
        now: the action rebuilds the tree, and the item would be gone."""
        if self._updating or column == COL_NAME or column == COL_COUNT:
            return
        key, targets = self._key(item), self._targets(item)
        QTimer.singleShot(0, lambda: self.click_cell(key, column, targets))

    def click_cell(self, key, column, targets=None) -> None:
        """What a click on ``column`` of the row ``key`` (kind, name) does:
        make active, show or hide, lock or unlock (``targets``: every
        selected row when the clicked one is among them), pick the colour."""
        kind, name = key
        if targets is None:
            item = self.item_for(name, kind)
            targets = self._targets(item) if item is not None else [name]
        scene = self._scene()
        if column == COL_ACTIVE and kind == KIND_LAYER:
            self.set_active(name)
        elif column == COL_EYE and kind == KIND_FOLDER:
            self.set_folder_visible(name, not L.folder_visible(scene, name, ancestors=False))
        elif column in (COL_EYE, COL_LOCK):
            names = targets
            layers =[scene.layer(n) for n in names if scene.layer(n)]
            if not layers:
                return
            # The row clicked decides: its state flips, and every target
            # follows it (a folder reads as shown while any layer in it is).
            clicked = scene.layer(name) if kind == KIND_LAYER else None
            if column == COL_EYE:
                now = (clicked.visible if clicked is not None
                       else any(ly.visible for ly in layers))
                self.set_visible(names, not now)
            else:
                now = (clicked.locked if clicked is not None
                       else all(ly.locked for ly in layers))
                self.set_locked(names, not now)
        elif column == COL_ISOLATE:
            from ingetrazo_render import isolate_tags
            isolate_tags(self._window.viewport, ["preview-tag:" + key for key in targets])
        elif column == COL_PRINT:
            layers = [scene.layer(n) for n in targets if scene.layer(n)]
            if layers:
                clicked = scene.layer(name) if kind == KIND_LAYER else None
                now = (getattr(clicked, "printable", True) if clicked
                       else any(getattr(ly, "printable", True)
                                for ly in layers))
                self.set_printable(targets, not now)
        elif column == COL_COLOR and kind == KIND_LAYER:
            self._select_edited(name, targets)
            self.pick_color(targets)

    def _select_edited(self, name: str, targets) -> None:
        """Select the row whose colour, line or hatch is being edited, so
        the list shows which layer the picker works on: the clicked row
        alone, unless it is one of several selected — then they all stay
        selected, as they all take the pick."""
        item = self.item_for(name)
        if item is None or item.isSelected():
            return
        self.tree.clearSelection()
        self.tree.setCurrentItem(item)
        item.setSelected(True)

    @staticmethod
    def _popup(menu, at) -> None:
        """Run ``menu`` at ``at`` (global) — one place, so a test can stand
        in for the modal loop."""
        menu.exec(at)

    # ---- Menus dropped from the panel --------------------------------------
    def _drop(self, menu, anchor) -> None:
        """Open ``menu`` under ``anchor`` (a global QRect), kept inside the
        panel."""
        self._popup(menu, place_menu(menu, anchor, global_rect(self)))

    def _open_more_menu(self) -> None:
        self._fill_more_menu()
        self._drop(self.more_menu, global_rect(self.more_btn))

    #: QSettings key of the rows' order: one of ``SORT_MODES``.
    SORT_KEY = "layers/sort_by"

    @property
    def sort_by(self) -> str:
        from core.settings import settings as QSettings
        v = str(QSettings().value(self.SORT_KEY, "name") or "name")
        return v if v in SORT_MODES else "name"

    def set_sort_by(self, how: str) -> None:
        """Order the rows by name (numbers read as numbers), in the order
        they were made, the visible ones first or the locked ones first
        (by name inside each group) — Layer 0 first and folders before
        layers either way; remembered. A row whose eye or padlock is
        clicked moves to its group at once."""
        from core.settings import settings as QSettings
        QSettings().setValue(self.SORT_KEY,
                             how if how in SORT_MODES else "name")
        self._sort = (COL_NAME, Qt.AscendingOrder)
        self.refresh()

    #: How many layer states «Previous layer state» can go back through.
    PREVIOUS_DEPTH = 20

    def _remember(self) -> None:
        """Keep the layers' state before a change to it (visibility, lock,
        active layer), for «Previous layer state»."""
        stack = self.__dict__.setdefault("_previous", [])
        stack.append(L.layer_state(self._scene()))
        del stack[:-self.PREVIOUS_DEPTH]

    def previous_state(self) -> bool:
        """Back to the layers' state before the last change made here or
        from the Layers toolbar — visibility, lock and active layer, not
        the geometry (that is Ctrl+Z's). False when there is none."""
        stack = self.__dict__.get("_previous") or []
        if not stack:
            self._window.statusBar().showMessage(
                tr("No earlier layer state."), 2500)
            return False
        L.apply_layer_state(self._scene(), stack.pop())
        self._touch()
        self._window.statusBar().showMessage(
            tr("Previous layer state restored."), 2500)
        return True

    def set_active(self, name: str | None) -> None:
        self._remember()
        L.set_active(self._scene(), name)
        self._touch()
        self._window.statusBar().showMessage(tr(
            "Active layer: '{layer}'", layer=L.active_layer(self._scene())),
            2500)

    def set_visible(self, names, on: bool) -> None:
        self._remember()
        scene = self._scene()
        from core.history import SetLayerVisibilityCommand
        previous = scene.active_layer
        self._window.viewport.history.execute(SetLayerVisibilityCommand(names, on))
        released = previous != scene.active_layer
        if not on:
            self._prune_selection(names)
        self._released(released)
        self._touch()

    def set_folder_visible(self, key, on):
        from core.history import SetLayerVisibilityCommand
        self._remember()
        self._window.viewport.history.execute(SetLayerVisibilityCommand([], on, [key]))
        self._touch()

    def set_locked(self, names, on: bool) -> None:
        self._remember()
        scene = self._scene()
        released = L.set_locked(scene, names, on)
        if on:
            self._prune_selection(names)
        self._released(released)
        self._touch()

    def show_all(self) -> None:
        self.set_visible([ly.key for ly in self._scene().layers], True)

    def pick_color(self, names) -> None:
        """Pick one colour for the layers ``names`` (one name or several —
        every selected row takes it)."""
        from views.color_dialog import get_color
        names = [names] if isinstance(names, str) else list(names)
        scene = self._scene()
        layers = [scene.layer(n) for n in names if scene.layer(n)]
        if not layers:
            return
        color = get_color(QColor.fromRgbF(*L.layer_color(layers[0])), self,
                          tr("Layer colour"))
        if not color.isValid():
            return
        self.set_color(names, (color.redF(), color.greenF(), color.blueF()))

    def set_color(self, names, rgb) -> None:
        """The colour of the layers ``names`` (``None``: automatic)."""
        names = [names] if isinstance(names, str) else list(names)
        scene = self._scene()
        for name in names:
            ly = scene.layer(name)
            if ly is not None:
                ly.color = (tuple(float(c) for c in rgb[:3]) if rgb
                            else None)
        self._dirty_paint()
        self._touch()

    def _dirty_paint(self) -> None:
        from core.history import _dirty_group_chunks
        _dirty_group_chunks(self._scene())

    def _prune_selection(self, names) -> None:
        names = set(names)
        scene = self._scene()
        for s in [s for s in scene.selection if L.layer_of(s) in names]:
            scene.selection.discard(s)

    def _on_item_changed(self, item, column) -> None:
        if self._updating or column != COL_NAME:
            return
        # ``old`` is the row's KEY, which a rename keeps: only the label
        # changes, so the folded folders and the row just made still match.
        kind, old = self._key(item)
        new = item.text(COL_NAME).strip()
        scene = self._scene()
        if kind == KIND_FOLDER:
            label = L.folder_label(scene, old)
        else:
            ly = scene.layer_by_key(old)
            label = ly.name if ly is not None else old
        if not new or new == label:
            return
        if new and new != label:
            if kind == KIND_FOLDER:
                L.rename_folder(scene, old, new)
            else:
                L.rename_layer(scene, old, new)
        QTimer.singleShot(0, self._touch)       # not while the editor closes

    def move_to_folder(self, names, folder: str | None) -> None:
        L.move_to_folder(self._scene(), names, folder)
        self._touch()

    def move_folder(self, name: str, parent: str | None) -> None:
        L.move_folder(self._scene(), name, parent)
        self._touch()

    def _ask_folder(self, names) -> None:
        from views import prompts
        name, ok = prompts.get_text(self, tr("New folder"),
                                    tr("Folder name:"))
        if ok and name.strip():
            self.move_to_folder(names, name.strip())

    def _fill_more_menu(self) -> None:
        from PySide6.QtGui import QActionGroup
        menu = self.more_menu
        menu.clear()
        group = QActionGroup(menu)
        for key, label in (("name", "Sort by name"),
                           ("created", "Sort by order created"),
                           ("visible", "Sort by visible"),
                           ("locked", "Sort by locked")):
            act = menu.addAction(tr(label))
            act.setCheckable(True)
            act.setChecked(self.sort_by == key)
            group.addAction(act)
            act.triggered.connect(lambda _c=False, k=key: self.set_sort_by(k))

    def set_style(self, names, **look) -> None:
        """Line type, line weight, cut weight, cut hatch or opacity of the
        layers ``names``."""
        scene = self._scene()
        for name in names:
            ly = scene.layer(name)
            if ly is None:
                continue
            if "linetype" in look:
                ly.linetype = look["linetype"]
            if "lineweight" in look:
                ly.lineweight = look["lineweight"]
            if "cut_weight" in look:
                ly.cut_weight = look["cut_weight"]
            if "hatch" in look:
                from core.hatches import piece_kind
                if piece_kind(look["hatch"]) != piece_kind(ly.hatch):
                    ly.brick = None    # a brick's size is no block's
                ly.hatch = look["hatch"]
            if "hatch_ground" in look:
                ly.hatch_ground = L._ground(look["hatch_ground"])
            if "hatch_color" in look:
                ly.hatch_color = L._ground(look["hatch_color"])
            if "hatch_angle" in look:
                ly.hatch_angle = float(look["hatch_angle"]) % 360.0
            if "hatch_scale" in look:
                ly.hatch_scale = min(20.0, max(0.05,
                                               float(look["hatch_scale"])))
            if "hatch_offset" in look:
                ly.hatch_offset = L.clean_offset(look["hatch_offset"])
            if "hatch_on_paper" in look:
                ly.hatch_on_paper = bool(look["hatch_on_paper"])
            if "hatch_cross" in look:
                ly.hatch_cross = bool(look["hatch_cross"])
            if "joints" in look:
                from core.hatches import JOINTS
                ly.joints = (look["joints"] if look["joints"] in JOINTS
                             else None)
            if "mortar" in look:
                ly.mortar = bool(look["mortar"])
            if "brick" in look:
                from core.hatches import clean_brick
                ly.brick = (clean_brick(look["brick"])
                            if look["brick"] is not None else None)
            if "opacity" in look:
                ly.opacity = min(1.0, max(0.1, float(look["opacity"])))
        self._dirty_paint()
        self._touch()

    def set_printable(self, names, on: bool) -> None:
        """Put the layers ``names`` on the sheets, or leave them out (still
        seen in the model). The sheets redraw on their next refresh."""
        scene = self._scene()
        for name in names:
            ly = scene.layer(name)
            if ly is not None:
                ly.printable = bool(on)
        self._touch()

    def _on_assign(self) -> None:
        """Move the selection onto the HIGHLIGHTED layer — and say so when
        there is nothing to move or no layer picked: the button used to do
        nothing in silence, which is how it read as broken («Asignar
        selección funcionó a la segunda», Rafael, 2026-09-16)."""
        from core.history import AssignLayerCommand
        scene = self._scene()
        item = self.tree.currentItem()
        if item is None or self._key(item)[0] != KIND_LAYER:
            self._window.statusBar().showMessage(
                tr("Click a layer in the list first, then Assign."), 3000)
            return
        name = self._key(item)[1]
        # Annotations are tagged too: a "Anotaciones" layer a
        # scene hides gives a clean plan without duplicating the model.
        targets = L.taggable(scene.selection)
        if not targets:
            self._window.statusBar().showMessage(
                tr("Select something in the model first, then Assign."), 3000)
            return
        self._window.viewport.history.execute(AssignLayerCommand(targets, name))
        self._window.viewport.update()
        self._window.statusBar().showMessage(
            tr("{n} entities moved to '{layer}'", n=len(targets), layer=name),
            2500)

    def select_content(self, names) -> None:
        from core.select_ops import on_layers
        scene = self._scene()
        picks = on_layers(scene, names)
        scene.select(picks)
        self._window.viewport.notify_scene_changed()
        self._window.statusBar().showMessage(
            tr("{n} entities selected", n=len(picks)), 2500)

    # ---- Context menu ----------------------------------------------------------
    def _on_context_menu(self, pos) -> None:
        item = self.tree.itemAt(pos)
        menu = QMenu(self)
        self.fill_menu(menu, item)
        if not menu.isEmpty():
            menu.exec(self.tree.viewport().mapToGlobal(pos))

    def fill_menu(self, menu, item) -> None:
        """The right-click menu of ``item`` (``None``: the empty list)."""
        scene = self._scene()
        if item is None:
            menu.addAction(tr("Show all layers"), self.show_all)
            menu.addSeparator()
            return
        kind, name = self._key(item)
        names = self._targets(item)
        if kind == KIND_LAYER:
            act = menu.addAction(tr("Make active"),
                                 lambda: self.set_active(name))
            act.setEnabled(L.active_layer(scene) != name)
            act = menu.addAction(tr("Move the selection here"),
                                 self._assign_to(item))
            act.setEnabled(bool(L.taggable(scene.selection)))
        # Isolating is the row's target; no entry for it here.
        menu.addAction(tr("Show all layers"), self.show_all)
        menu.addAction(tr("Select content"),
                       lambda: self.select_content(names))
        menu.addSeparator()
        if kind == KIND_LAYER:
            menu.addAction(tr("Colour…"), lambda: self.pick_color(names))
            reset = menu.addAction(tr("Automatic colour"),
                                   lambda: self.set_color(names, None))
            ly = scene.layer(name)
            reset.setEnabled(any(scene.layer(n) is not None
                                 and scene.layer(n).color is not None
                                 for n in names))
            movable = [n for n in names if n != L.DEFAULT_LAYER]
            if movable:
                sub = _submenu(menu, tr("Move to folder"))
                for folder in L.folders_of(scene):
                    sub.addAction(self._folder_path(folder),
                                  lambda f=folder:
                                  self.move_to_folder(movable, f))
                sub.addSeparator()
                sub.addAction(tr("New folder…"), lambda: QTimer.singleShot(
                    0, lambda: self._ask_folder(movable)))
                out = sub.addAction(tr("Out of any folder"), lambda:
                                    self.move_to_folder(movable, None))
                out.setEnabled(any(scene.layer(n) and scene.layer(n).folder
                                   for n in movable))
        else:
            inside = set(L.subfolders(scene, name, deep=True)) | {name}
            sub = _submenu(menu, tr("Move to folder"))
            for folder in L.folders_of(scene):
                if folder not in inside:
                    sub.addAction(self._folder_path(folder), lambda f=folder:
                                  self.move_folder(name, f))
            sub.addSeparator()
            top = sub.addAction(tr("Out of any folder"),
                                lambda: self.move_folder(name, None))
            top.setEnabled(L.folder_parent(scene, name) is not None)
        menu.addSeparator()
        if kind == KIND_FOLDER or name != L.DEFAULT_LAYER:
            menu.addAction(tr("Rename"), lambda: self.tree.editItem(item,
                                                                    COL_NAME))
        menu.addSeparator()

    #: Line type labels, in menu order.
    LINETYPE_LABELS = (("continuous", "Continuous"), ("dashed", "Dashed"),
                       ("dashed_short", "Short dashes (hidden)"),
                       ("dashed_long", "Long dashes"),
                       ("dotted", "Dotted"), ("dashdot", "Dash-dot"),
                       ("dashdotdot", "Dash-dot-dot"),
                       ("center", "Centre line (long dash, dot)"),
                       ("phantom", "Phantom (long dash, two dots)"),
                       ("border", "Border (two dashes, dot)"))
    def _assign_to(self, item):
        def run():
            self.tree.setCurrentItem(item)
            self._on_assign()
        return run

    # ---- The window's scene-changed signal -------------------------------------
    def on_scene_changed(self) -> None:
        """The window's per-change call: rebuild once the event loop is
        back — never from inside a paint nor from a drop-down's own signal.

        ``sceneVersionChanged`` reaches here from ``_sync_edges``, that is,
        in the middle of ``paintGL``, and from the Layers toolbar's list
        while Qt is still closing it (handing a layer to the groups).
        Clearing and sorting the tree there was an access violation: the
        rows are rebuilt on the next turn instead, a burst of changes
        paying one rebuild. A window not shown yet (scripts, tests)
        refreshes at once, as before."""
        if self.tree.state() == QAbstractItemView.EditingState:
            return                          # never under an open name editor
        if self._window.isVisible() and not self.isVisible():
            self._stale = True
            return
        timer = getattr(self, "_refresh_timer", None)
        if timer is None:
            from PySide6.QtCore import QTimer
            timer = self._refresh_timer = QTimer(self)
            timer.setSingleShot(True)
            timer.timeout.connect(self._refresh_after_change)
        timer.start(0)

    def _refresh_after_change(self) -> None:
        """Update selection and filtering in place when the rows are unchanged."""
        signature = self._signature()
        previous = getattr(self, "_drawn_signature", None)
        if signature == previous and self.tree.topLevelItemCount():
            self._mark_selection()
            return
        if previous is not None and self._append_root_layers(previous, signature):
            return
        if self._structure() == getattr(self, "_drawn_structure", None):
            self._update_rows()
        else:
            self.refresh()

    def _append_root_layers(self, previous, signature):
        """Creating the first guide may append a tag to a large existing tree."""
        old, isolation = previous
        new, current_isolation = signature
        n = len(old[0])
        if (n >= len(new[0]) or new[0][:n] != old[0] or old[1:] != new[1:]
                or isolation != current_isolation):
            return False
        scene = self._scene()
        added = scene.layers[n:]
        if any(ly.folder for ly in added) or self.isolated_layers() != self._alone:
            return False
        self._updating = True
        try:
            active = L.active_layer(scene)
            for order, ly in enumerate(added, n):
                self.tree.addTopLevelItem(self._new_layer_row(ly, order, active))
            self.tree.sortItems(*self._sort)
        finally:
            self._updating = False
        self._mark_selection()
        self._drawn_signature = signature
        self._drawn_structure = self._structure()
        return True

    def _signature(self) -> tuple:
        return (layers_signature(self._scene()), frozenset(self.isolated_layers()))

    def showEvent(self, ev) -> None:
        super().showEvent(ev)
        if getattr(self, "_stale", False):
            self._stale = False
            self.refresh()


class LayersDock(QDockWidget):
    """Sidebar tab **Layers**, next to the Browser."""

    def __init__(self, window) -> None:
        super().__init__(tr("Layers"), window)
        self.setObjectName("tray_layers")
        self.setAllowedAreas(Qt.LeftDockWidgetArea | Qt.RightDockWidgetArea)
        self.setFeatures(QDockWidget.DockWidgetMovable
                         | QDockWidget.DockWidgetClosable)
        self.panel = LayersPanel(window)
        self.setWidget(self.panel)
        self.setMinimumWidth(240)

    def on_scene_changed(self) -> None:
        self.panel.on_scene_changed()
