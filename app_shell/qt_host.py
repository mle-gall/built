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
    return _SETTINGS_DIR


def pump() -> None:
    """Deliver posted events and due QTimers (QTimer.singleShot callers in
    core/tools/views). The Rust loop calls this once per turn; Qt's own
    loop (exec) never runs."""
    if _APP is not None:
        from PySide6.QtCore import QCoreApplication, QEventLoop
        QCoreApplication.sendPostedEvents()
        QCoreApplication.processEvents(QEventLoop.AllEvents, 0)
