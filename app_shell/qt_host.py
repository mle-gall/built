"""Headless Qt for the shell: value types, painting and timers, no widgets.

``start`` must run before anything imports ``views`` or ``core`` modules that
read QSettings at import time. It points QSettings and the app's data dir at
a throwaway directory unless ``BUILT_SETTINGS_DIR`` names one, so a shell
run never touches the user's preferences or autosave slots.
"""
from __future__ import annotations

import os
import sys
import tempfile

_APP = None
_SETTINGS_DIR = None


def settings_dir() -> str | None:
    return _SETTINGS_DIR


def start(settings: str | None = None) -> str:
    """Create the offscreen QGuiApplication once; return the settings dir."""
    global _APP, _SETTINGS_DIR
    if _APP is not None:
        return _SETTINGS_DIR
    # Offscreen: Qt opens no window and never talks to AppKit; winit does.
    os.environ["QT_QPA_PLATFORM"] = "offscreen"
    _SETTINGS_DIR = settings or os.environ.get("BUILT_SETTINGS_DIR") or tempfile.mkdtemp(
        prefix="built-shell-settings-")
    os.makedirs(_SETTINGS_DIR, exist_ok=True)
    data = os.path.join(_SETTINGS_DIR, "data")
    os.makedirs(data, exist_ok=True)
    os.environ["XDG_DATA_HOME"] = data
    from PySide6.QtCore import QCoreApplication, QSettings, QStandardPaths
    from PySide6.QtGui import QGuiApplication
    QSettings.setDefaultFormat(QSettings.IniFormat)
    for scope in (QSettings.UserScope, QSettings.SystemScope):
        QSettings.setPath(QSettings.IniFormat, scope, _SETTINGS_DIR)
    # Autosave and caches resolve AppDataLocation through QStandardPaths:
    # test mode sends them under ~/.qttest instead of the real app folders.
    QStandardPaths.setTestModeEnabled(True)
    QCoreApplication.setOrganizationName("IngeTrazo")
    QCoreApplication.setApplicationName("IngeTrazo")
    _APP = QGuiApplication.instance() or QGuiApplication([sys.argv[0] if sys.argv else "built"])
    _answer_input_state()
    return _SETTINGS_DIR


#: The input state winit last reported (app_shell.host keeps it current).
INPUT = {"modifiers": 0, "buttons": 0, "pos": (0.0, 0.0)}


def _answer_input_state():
    """The offscreen platform sees no keyboard or mouse, so Qt's global
    input queries (Push/Pull's keep-base Ctrl, Dimension's Shift, Walk,
    Tape's cursor, autosave's "a button is down") would always answer
    "nothing". They answer from what winit reported instead."""
    from PySide6.QtCore import QPoint, Qt
    from PySide6.QtGui import QCursor, QGuiApplication

    def modifiers():
        return Qt.KeyboardModifier(INPUT["modifiers"])

    def buttons():
        return Qt.MouseButton(INPUT["buttons"])

    def pos(*_):
        x, y = INPUT["pos"]
        return QPoint(int(x), int(y))

    QGuiApplication.keyboardModifiers = staticmethod(modifiers)
    QGuiApplication.queryKeyboardModifiers = staticmethod(modifiers)
    QGuiApplication.mouseButtons = staticmethod(buttons)
    QCursor.pos = staticmethod(pos)


def pump() -> None:
    """Deliver posted events and due QTimers (QTimer.singleShot callers in
    core/tools/views). The Rust loop calls this once per turn; Qt's own
    loop (exec) never runs."""
    if _APP is not None:
        from PySide6.QtCore import QCoreApplication, QEventLoop
        QCoreApplication.sendPostedEvents()
        QCoreApplication.processEvents(QEventLoop.AllEvents, 0)
