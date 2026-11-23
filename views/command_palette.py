# SPDX-License-Identifier: GPL-3.0-or-later
"""Host the Rust palette over the document in classic or detached windows."""
from PySide6.QtCore import QEvent, QObject, QPoint, QRect, Qt, QTimer
from PySide6.QtGui import QCursor, QImage, QPainter
from PySide6.QtWidgets import QApplication, QMenu, QWidget

from ingetrazo_render import CommandCanvas
from views.command_metadata import _submenus, _top_menus, commands, menu_trail


def warm_up(window, delay_ms=2000):
    QTimer.singleShot(delay_ms, window, lambda: commands(window))


def fit_box(cursor, size, edit_h, work, screen):
    return QRect(*CommandCanvas.geometry(cursor.toTuple(), size.toTuple(), edit_h,
                                         work.getRect(), screen.getRect()))


class _Canvas(QWidget):
    """Legacy compositor binding. Egui owns text, rows, navigation and hit testing."""

    def __init__(self, window, bounds):
        from views.wgpu_ui import FONT
        super().__init__(window)
        self.setGeometry(bounds)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setMouseTracking(True)
        self.setAttribute(Qt.WA_InputMethodEnabled)
        self._scale = self.devicePixelRatioF()
        self.renderer = CommandCanvas(self.width(), self.height(), self._scale, FONT)
        self._image = QImage()
        self._previous = None
        self._tick = QTimer(self, interval=120)
        self._tick.timeout.connect(self.draw)

    def start(self, text, scope):
        self._previous = QApplication.focusWidget()
        self.renderer.open(text, scope)
        self.show()
        self.raise_()
        self.setFocus()
        self._tick.start()
        QApplication.instance().installEventFilter(self)
        self.draw()
        self.draw()

    def draw(self):
        from views.wgpu_ui import command_action
        opened = self.renderer.state()[0]
        if not opened:
            self.hide()
        for message in self.renderer.actions():
            verb, _, arg = message.partition(":")
            command_action(self, self.parentWidget(), verb, arg)
        if not opened:
            return
        scale = self._scale
        w, h = round(self.width() * scale), round(self.height() * scale)
        self._pixels = self.renderer.frame()
        self._image = QImage(self._pixels, w, h, 4 * w, QImage.Format_RGBA8888)
        self._image.setDevicePixelRatio(scale)
        self.update()
        if not self.renderer.state()[0]:
            QTimer.singleShot(0, self, self.draw)

    def paintEvent(self, event):
        painter = QPainter(self)
        painter.drawImage(0, 0, self._image)

    def eventFilter(self, obj, event):
        if event.type() == QEvent.MouseButtonPress and obj is not self and self.isVisible():
            self.hide()
        return False

    def hideEvent(self, event):
        from shiboken6 import isValid
        QApplication.instance().removeEventFilter(self)
        self._tick.stop()
        if self._previous is not None and isValid(self._previous) and self.hasFocus():
            self._previous.setFocus()
        super().hideEvent(event)

    def event(self, event):
        if event.type() == QEvent.ShortcutOverride:
            event.accept()
            return True
        return super().event(event)

    def keyPressEvent(self, event):
        from views.wgpu_ui import _KEYS, _mods
        text = event.text()
        self.renderer.key(_KEYS.get(event.key(), ""), True, _mods(event),
                          text if text.isprintable() else "")
        self.draw()

    def keyReleaseEvent(self, event):
        from views.wgpu_ui import _KEYS, _mods
        self.renderer.key(_KEYS.get(event.key(), ""), False, _mods(event), "")
        self.draw()

    def inputMethodEvent(self, event):
        self.renderer.key("", True, 0, event.commitString())
        self.draw()
        event.accept()

    def _pointer(self, event, kind):
        from views.wgpu_ui import _BUTTONS, _mods
        point = event.position()
        self.renderer.pointer(kind, point.x(), point.y(), _BUTTONS.get(event.button(), 0), _mods(event))
        self.draw()

    def mouseMoveEvent(self, event):
        self._pointer(event, "move")

    def mousePressEvent(self, event):
        self._pointer(event, "press")

    def mouseReleaseEvent(self, event):
        self._pointer(event, "release")

    def wheelEvent(self, event):
        p, d = event.position(), event.angleDelta()
        self.renderer.wheel(p.x(), p.y(), d.x(), d.y())
        self.draw()


def open_search(window, text="", scope=""):
    from PySide6.QtCore import QSize
    view = getattr(getattr(window, "_doc", None), "stand_in", None)
    if getattr(view, "_ui_attached", False):
        view.renderer.ui_open_commands(text, scope)
        view.update()
        return view
    work = window.command_search_area()
    work_rect = QRect(work.mapToGlobal(QPoint()), work.size())
    outer = QRect(window.mapToGlobal(QPoint()), window.size())
    position = fit_box(QCursor.pos(), QSize(560, 310), 34, work_rect, outer)
    position.moveTopLeft(window.mapFromGlobal(position.topLeft()))
    canvas = getattr(window, "_command_canvas", None)
    if canvas is None or canvas.size() != position.size() or canvas._scale != window.devicePixelRatioF():
        if canvas is not None:
            canvas.hide()
            canvas.deleteLater()
        canvas = window._command_canvas = _Canvas(window, position)
    canvas.move(position.topLeft())
    canvas.start(text, scope)
    return canvas


class _MenuTyping(QObject):
    """Blender 4's type-to-search: a letter pressed in an open menu of the
    menu bar closes it and searches that menu, the letter already typed.
    The menus carry no «&» mnemonics, so no letter meant anything there."""

    def watch(self, menu: QMenu, subs: dict | None = None) -> None:
        if subs is None:
            subs = _submenus(self.parent())
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
        scope = menu_trail(self.parent(), menu)
        if scope is None:
            return False
        for _ in range(16):                    # the menu and its parents
            popup = QApplication.activePopupWidget()
            if popup is None:
                break
            popup.close()
        QTimer.singleShot(0, self, lambda: open_search(self.parent(), text, scope))
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
