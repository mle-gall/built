# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Command search: type a few letters, press Enter, the command runs.

Blender's F3 menu search, Rhino's command line: a
single box that filters every action the window already has — menus,
tools, trays, plugins — so nobody has to remember in which menu «Orient
Faces» lives. Nothing is registered twice: the list is the window's own
``QAction`` objects (the same :func:`views.shortcuts.collect_actions` the
shortcut editor lists), read each time the box opens, so a plugin loaded a
minute ago is already in it.

The matching is Blender's (:mod:`core.string_search`): word beginnings,
initials («rf» → Reverse Faces), a typing error or two, in the language
of the menus AND in English (a tutorial says «Push/Pull», the menu says
«Empujar/Tirar»), by menu path too, accents and case aside. As in Blender,
the path is drawn faded, a right click offers to change the command's
shortcut, and typing a letter in an open menu searches that menu.
"""
from __future__ import annotations

from PySide6.QtCore import (QAbstractListModel, QModelIndex, QObject,
                            QPoint, QRect, QSize, Qt, QTimer)
from PySide6.QtGui import (QAction, QColor, QCursor, QGuiApplication, QIcon,
                           QPalette)
from PySide6.QtWidgets import (QApplication, QFrame, QHBoxLayout, QLabel,
                               QLineEdit, QListView, QMenu,
                               QStyle,
                               QStyledItemDelegate, QStyleOptionViewItem,
                               QVBoxLayout)

from core.i18n import tr

from views.command_metadata import (
    OBJECT_NAME, Command, _keys_in, _submenus, _top_menus, commands,
    menu_paths, menu_trail, recent_keys, remember, search, tooltip_html,
)


_PATH_ROLE = Qt.UserRole + 1
_KEYS_ROLE = Qt.UserRole + 2
#: Rows shown at once, as Blender's box (SEARCH_ITEMS); the rest scroll.
VISIBLE_ROWS = 10
_ICON = 16


BOX_WIDTH = 560


def _clamp_axis(pos: int, size: int, lo: int, hi: int,
                far_lo: int, far_hi: int) -> int:
    """Where a box of ``size`` starting near ``pos`` goes on one axis:
    inside ``lo..hi`` when it fits there, else inside ``far_lo..far_hi``."""
    if size > hi - lo:
        lo, hi = far_lo, far_hi
    return max(lo, min(pos, hi - size))


def fit_box(cursor: QPoint, size: QSize, edit_h: int, work: QRect,
            screen: QRect) -> QRect:
    """The box's place: its text field under ``cursor``, as Blender's, but
    never over the side trays or the toolbars (Marco: «siempre se vea en
    el espacio de modelado»). Each axis on its own:

    1. inside the modelling area ``work`` when the box fits in it;
    2. otherwise over its edge — the trays — keeping its size, inside the
       ``screen``;
    3. smaller only when not even the screen holds it (the rows scroll)."""
    width = min(size.width(), screen.width())
    height = min(size.height(), screen.height())
    x = _clamp_axis(cursor.x() - width // 2, width,
                    work.left(), work.left() + work.width(),
                    screen.left(), screen.left() + screen.width())
    y = _clamp_axis(cursor.y() - edit_h // 2, height,
                    work.top(), work.top() + work.height(),
                    screen.top(), screen.top() + screen.height())
    return QRect(x, y, width, height)


class _Results(QAbstractListModel):
    """The matches, drawn on demand: a keystroke resets a list, and only
    the rows on screen are ever asked for their text and icon."""

    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        self.rows: list = []

    def set_rows(self, rows: list) -> None:
        self.beginResetModel()
        self.rows = rows
        self.endResetModel()

    def rowCount(self, parent=QModelIndex()) -> int:
        return 0 if parent.isValid() else len(self.rows)

    def flags(self, index):
        if self.rows[index.row()].action.isEnabled():
            return Qt.ItemIsEnabled | Qt.ItemIsSelectable
        return Qt.NoItemFlags                  # shown, greyed, never run

    def data(self, index, role=Qt.DisplayRole):
        cmd = self.rows[index.row()]
        act = cmd.action
        if role == Qt.DisplayRole:
            checked = act.isCheckable() and act.isChecked()
            return ("✓ " if checked else "") + cmd.name
        if role == Qt.DecorationRole:
            return act.icon()
        if role == _PATH_ROLE:
            return cmd.path
        if role == _KEYS_ROLE:
            return cmd.keys
        if role == Qt.ToolTipRole:
            return tooltip_html(cmd)
        return None


class _RowDelegate(QStyledItemDelegate):
    """One line per command, as Blender draws its search: the menu path
    faded, «▸», the icon, the name in full colour, the keys at the right.
    The path gives way (cut from the left) before the name does."""

    def paint(self, painter, option, index) -> None:
        opt = QStyleOptionViewItem(option)
        self.initStyleOption(opt, index)
        # A copy: PySide hands out the option's own icon, which the reset
        # below would blank.
        name, icon = opt.text, QIcon(opt.icon)
        opt.text, opt.icon = "", QIcon()
        widget = opt.widget
        style = widget.style() if widget is not None else QApplication.style()
        style.drawControl(QStyle.CE_ItemViewItem, opt, painter, widget)

        pal = opt.palette
        selected = bool(opt.state & QStyle.State_Selected)
        enabled = bool(opt.state & QStyle.State_Enabled)
        if not enabled:
            main = pal.color(QPalette.Disabled, QPalette.Text)
        else:
            main = pal.color(QPalette.HighlightedText if selected
                             else QPalette.Text)
        faded = QColor(main)
        faded.setAlphaF(0.55 if enabled else 0.35)
        fm = opt.fontMetrics
        rect = opt.rect.adjusted(6, 0, -6, 0)
        mid = rect.center().y()

        painter.save()
        painter.setPen(faded)
        keys = index.data(_KEYS_ROLE) or ""
        if keys:
            painter.drawText(rect, Qt.AlignRight | Qt.AlignVCenter, keys)
            rect.setRight(rect.right() - fm.horizontalAdvance(keys) - 12)
        path = index.data(_PATH_ROLE) or ""
        icon_w = _ICON + 4 if not icon.isNull() else 0
        x = rect.left()
        if path:
            room = rect.width() - fm.horizontalAdvance(name) - icon_w
            text = fm.elidedText(path + " ▸ ", Qt.ElideLeft, max(room, 0))
            painter.drawText(x, rect.top(), rect.width(), rect.height(),
                             Qt.AlignLeft | Qt.AlignVCenter, text)
            x += fm.horizontalAdvance(text)
        if icon_w:
            mode = QIcon.Normal if enabled else QIcon.Disabled
            icon.paint(painter, x, mid - _ICON // 2, _ICON, _ICON,
                       Qt.AlignCenter, mode)
            x += icon_w
        painter.setPen(main)
        width = max(rect.right() - x, 0)
        painter.drawText(x, rect.top(), width, rect.height(),
                         Qt.AlignLeft | Qt.AlignVCenter,
                         fm.elidedText(name, Qt.ElideRight, width))
        painter.restore()

    def sizeHint(self, option, index) -> QSize:
        return QSize(option.rect.width(),
                     max(option.fontMetrics.height(), _ICON) + 6)


class CommandSearch(QFrame):
    """The box itself: a compact popup at the mouse, as Blender's. Type to
    filter, arrows to move, Enter to run, Esc (or a click outside) to
    leave; a right click on a row offers to change its shortcut."""

    def __init__(self, window) -> None:
        super().__init__(window, Qt.Popup)
        self._window = window
        self.setFrameShape(QFrame.StyledPanel)
        lay = QVBoxLayout(self)
        lay.setContentsMargins(2, 2, 2, 2)
        lay.setSpacing(2)
        top = QHBoxLayout()
        top.setContentsMargins(0, 0, 0, 0)
        self._scope_label = QLabel()
        self._scope_label.hide()
        top.addWidget(self._scope_label)
        self._edit = QLineEdit()
        self._edit.setPlaceholderText(tr("Search a command…"))
        self._edit.setClearButtonEnabled(True)
        from views.icons import tool_icon
        self._edit.addAction(tool_icon("zoom"), QLineEdit.LeadingPosition)
        self._edit.textChanged.connect(self._refill)
        self._edit.installEventFilter(self)
        top.addWidget(self._edit, 1)
        lay.addLayout(top)
        self._model = _Results(self)
        self._list = QListView()
        self._list.setModel(self._model)
        self._list.setItemDelegate(_RowDelegate(self._list))
        self._list.setUniformItemSizes(True)
        self._list.setFrameShape(QFrame.NoFrame)
        self._list.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        self._list.setFocusPolicy(Qt.NoFocus)   # the typing stays in the box
        self._list.setContextMenuPolicy(Qt.CustomContextMenu)
        self._list.customContextMenuRequested.connect(self._context_menu)
        self._list.activated.connect(self._run)
        self._list.clicked.connect(self._run)
        lay.addWidget(self._list, 1)
        self._empty = QLabel(tr("No results found"))
        self._empty.setAlignment(Qt.AlignLeft | Qt.AlignTop)
        self._empty.setContentsMargins(8, 4, 8, 4)
        self._empty.setEnabled(False)          # faded, as Blender's
        self._empty.hide()
        lay.addWidget(self._empty, 1)
        self._pool: list = []
        self._recent: dict = {}
        self._scope = ""

    # ---- showing ----------------------------------------------------------------
    def _place(self) -> None:
        """Ten rows high, the text field under the mouse pointer — where
        Blender opens it — but inside the modelling area (see fit_box)."""
        row = max(self.fontMetrics().height(), _ICON) + 6
        edit_h = self._edit.sizeHint().height()
        size = QSize(BOX_WIDTH, edit_h + row * VISIBLE_ROWS + 10)
        cursor = QCursor.pos()
        screen = QGuiApplication.screenAt(cursor) or self._window.screen()
        area = getattr(self._window, "command_search_area", None)
        work = area() if callable(area) else self._window.centralWidget()
        work_rect = QRect(work.mapToGlobal(QPoint(0, 0)), work.size())
        self.setGeometry(fit_box(cursor, size, edit_h, work_rect,
                                 screen.availableGeometry()))

    def popup(self, text: str = "", scope: str = "") -> None:
        self._place()
        self._set_scope(scope)
        keys = recent_keys()
        self._recent = {k: len(keys) - i for i, k in enumerate(keys)}
        # The list never has the focus, so it is never "active": paint its
        # current row as if it were, or the row Enter would run is barely
        # told apart. Taken on each opening — the theme may have changed.
        pal = QApplication.palette(self._list)
        for role in (QPalette.Highlight, QPalette.HighlightedText):
            pal.setColor(QPalette.Inactive, role,
                         pal.color(QPalette.Active, role))
        self._list.setPalette(pal)
        self._edit.blockSignals(True)
        self._edit.setText(text)
        self._edit.blockSignals(False)
        self._refill(text)
        self.show()
        self.raise_()
        self._edit.setFocus(Qt.PopupFocusReason)

    def _set_scope(self, scope: str) -> None:
        """Search only under one menu (typed in it), or everywhere."""
        self._scope = scope
        self._pool = commands(self._window, scope)  # plugins come and go
        self._scope_label.setText(scope + " ▸")
        self._scope_label.setToolTip(
            tr("Backspace in the empty box searches everywhere"))
        self._scope_label.setVisible(bool(scope))

    def _refill(self, text: str) -> None:
        rows = search(self._window, text, self._pool, self._recent)
        self._model.set_rows(rows)
        self._list.setVisible(bool(rows))
        self._empty.setVisible(not rows)
        self._select(self._first_enabled(0, 1))

    def _first_enabled(self, start: int, step: int) -> int:
        rows = self._model.rows
        i = start
        while 0 <= i < len(rows):
            if rows[i].action.isEnabled():
                return i
            i += step
        return -1

    def _select(self, row: int) -> None:
        if row < 0:
            self._list.setCurrentIndex(QModelIndex())
            return
        index = self._model.index(row, 0)
        self._list.setCurrentIndex(index)
        self._list.scrollTo(index)

    # ---- keys ---------------------------------------------------------------------
    def eventFilter(self, obj, event) -> bool:
        if obj is self._edit and event.type() == event.Type.KeyPress:
            key = event.key()
            if key in (Qt.Key_Down, Qt.Key_Up, Qt.Key_PageDown,
                       Qt.Key_PageUp):
                self._move(key)
                return True
            if key in (Qt.Key_Return, Qt.Key_Enter):
                self._run(self._list.currentIndex())
                return True
            if key == Qt.Key_Escape:
                self.close()
                return True
            if key == Qt.Key_Backspace and self._scope \
                    and not self._edit.text():
                self._set_scope("")
                self._refill("")
                return True
        return super().eventFilter(obj, event)

    def _move(self, key) -> None:
        cur = self._list.currentIndex()
        i = cur.row() if cur.isValid() else -1
        step = 1 if key in (Qt.Key_Down, Qt.Key_PageDown) else -1
        jump = 10 if key in (Qt.Key_PageDown, Qt.Key_PageUp) else 1
        n = len(self._model.rows)
        target = max(0, min(n - 1, i + step * jump))
        row = self._first_enabled(target, step)
        if row < 0:
            row = self._first_enabled(target, -step)
        if row >= 0:
            self._select(row)

    # ---- running --------------------------------------------------------------------
    def _action_at(self, index):
        if not index.isValid() or index.row() >= len(self._model.rows):
            return None
        return self._model.rows[index.row()].action

    def _run(self, index) -> None:
        act = self._action_at(index)
        if act is None or not act.isEnabled():
            return
        self.close()
        remember(act)
        # After the popup is gone: a command that opens a dialog must not
        # find this popup still grabbing the keyboard.
        QTimer.singleShot(0, act.trigger)

    def _context_menu(self, pos) -> None:
        act = self._action_at(self._list.indexAt(pos))
        if act is None:
            return
        if self._preferences_owner() is None:
            return
        menu = QMenu(self)
        change = menu.addAction(tr("Change shortcut…"))
        if menu.exec(self._list.viewport().mapToGlobal(pos)) is change:
            self.change_shortcut(act)

    def change_shortcut(self, act: QAction) -> None:
        """Preferences on this command's shortcut (Blender's right click
        ▸ Change Shortcut)."""
        owner = self._preferences_owner()
        self.close()
        if owner is not None:
            QTimer.singleShot(
                0, lambda: owner.open_preferences(shortcut_of=act))

    def _preferences_owner(self):
        """The window whose Preferences hold the shortcuts: this one, or
        the main window the composer belongs to."""
        w = self._window
        while w is not None and not hasattr(w, "open_preferences"):
            w = w.parentWidget()
        return w


def warm_up(window, delay_ms: int = 2000) -> None:
    """Work out F3's command list while the window idles after opening,
    so even the first F3 shows its list at once.

    Only the list — pure Python. The box and its native popup are made on
    the first F3, as they always were up to 0.5.6.1: made ahead, the popup
    left the main window flickering under GNOME's Wayland, a black band
    and the toolbar drawn half over the menu bar (Marco, 30-09, bisected
    to ``box.winId()`` here; it only shows with two monitors, when
    IngeTrazo runs on Wayland). Kept off on every platform until it has
    been tried on each."""
    def ready() -> None:
        commands(window)
    QTimer.singleShot(delay_ms, window, ready)


def open_search(window, text: str = "", scope: str = "") -> CommandSearch:
    """Show the window's search box (made once, kept)."""
    box = getattr(window, "_command_search", None)
    if box is None:
        box = window._command_search = CommandSearch(window)
    box.popup(text, scope)
    return box


class _MenuTyping(QObject):
    """Blender 4's type-to-search: a letter pressed in an open menu of the
    menu bar closes it and searches that menu, the letter already typed.
    The menus carry no «&» mnemonics, so no letter meant anything there."""

    def __init__(self, window) -> None:
        super().__init__(window)
        self._window = window

    def watch(self, menu: QMenu, subs: dict | None = None) -> None:
        if subs is None:
            subs = _submenus(self._window)
        menu.installEventFilter(self)          # once: Qt drops repeats
        for act in menu.actions():
            sub = subs.get(act)
            if sub is not None:
                self.watch(sub, subs)

    def eventFilter(self, menu, event) -> bool:
        if event.type() != event.Type.KeyPress or not isinstance(menu, QMenu):
            return False
        text = event.text()
        mods = event.modifiers() & ~(Qt.ShiftModifier | Qt.KeypadModifier)
        if len(text) != 1 or not text.isprintable() or text.isspace() \
                or mods != Qt.NoModifier:
            return False
        scope = menu_trail(self._window, menu)
        if scope is None:
            return False
        for _ in range(16):                    # the menu and its parents
            popup = QApplication.activePopupWidget()
            if popup is None:
                break
            popup.close()
        QTimer.singleShot(0, lambda: open_search(self._window, text, scope))
        return True


def install_menu_typing(window) -> None:
    """Watch every menu of the menu bar — and, as each opens, the submenus
    a plugin or a rebuild may have added since."""
    typing = _MenuTyping(window)
    for menu in _top_menus(window):
        typing.watch(menu)
        menu.aboutToShow.connect(lambda m=menu: typing.watch(m))
    window._menu_typing = typing
    warm_up(window)
