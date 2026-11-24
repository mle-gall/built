"""Headless Qt for the shell: value types, painting and timers, no widgets.

``start`` must run before anything imports ``views`` or ``core`` modules that
read QSettings at import time. Interactive launches share the classic app's
profile. Automation and tests use isolated settings.

The application object is an offscreen ``QApplication`` (no window ever
reaches AppKit; winit owns the screen). It used to be a QGuiApplication,
but borrowed code that still builds a Qt widget (a QMessageBox fallback, a
QMenu, a dialog an extension opens) then aborted the process: "Must
construct a QApplication before a QWidget". Now such a widget can exist,
and :class:`WidgetGuard` stops it from waiting for an answer nobody can
see: a top-level widget that shows is rejected or closed at once, and the
host is told, so the status says "needs the classic app" instead of the
app hanging behind an invisible modal dialog.
"""
from __future__ import annotations

import os
import sys
import tempfile
import time
import weakref

_APP = None
_SETTINGS_DIR = None
_GUARD = None


def settings_dir() -> str | None:
    return _SETTINGS_DIR


def configure_settings(profile: str | None) -> None:
    """Select an isolated settings backend before either host reads preferences."""
    from PySide6.QtCore import QSettings, QStandardPaths
    if profile:
        data = os.path.join(profile, "data")
        os.makedirs(data, exist_ok=True)
        os.environ["XDG_DATA_HOME"] = data
        QSettings.setDefaultFormat(QSettings.IniFormat)
        for scope in (QSettings.UserScope, QSettings.SystemScope):
            QSettings.setPath(QSettings.IniFormat, scope, profile)
    QStandardPaths.setTestModeEnabled(bool(profile))


def start(settings: str | None = None, *, temporary: bool = False) -> str | None:
    """Create the offscreen QApplication once; return the settings dir."""
    global _APP, _SETTINGS_DIR
    profile = settings or os.environ.get("BUILT_SETTINGS_DIR")
    if not profile and not temporary and ("pytest" in sys.modules or "PYTEST_CURRENT_TEST" in os.environ):
        raise RuntimeError("pytest must supply an isolated shell profile")
    if _APP is not None:
        return _SETTINGS_DIR
    # Offscreen: Qt opens no window and never talks to AppKit; winit does.
    os.environ["QT_QPA_PLATFORM"] = "offscreen"
    from PySide6.QtCore import QCoreApplication
    from PySide6.QtWidgets import QApplication
    if temporary and not profile:
        profile = tempfile.mkdtemp(prefix="built-shell-settings-")
    _SETTINGS_DIR = profile
    configure_settings(profile)
    QCoreApplication.setOrganizationName("IngeTrazo")
    QCoreApplication.setApplicationName("IngeTrazo")
    _APP = QApplication.instance() or QApplication([sys.argv[0] if sys.argv else "built"])
    from core import i18n
    from PySide6.QtCore import QSettings
    i18n.set_language(i18n.resolve_language(QSettings().value("language")))
    _answer_input_state()
    _track_timers()
    install_widget_guard()
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


# ---- Qt timers: when the loop must wake ----------------------------------------
#: QTimers started from Python (weak) and single-shot deadlines.
_TIMERS: "weakref.WeakSet" = weakref.WeakSet()
_SHOTS: list = []


def _track_timers():
    """Qt's timers fire only while :func:`pump` runs, and winit sleeps
    until an event. To wake for them, the starts made from Python are
    noted (``QTimer.start`` and ``QTimer.singleShot``) and
    :func:`next_timer_ms` answers how long until the first is due. Timers
    started from C++ alone are not seen. Native presentation and autosave
    deadlines are owned by the window's Rust timer queue."""
    from PySide6.QtCore import QTimer
    if getattr(QTimer, "_shell_tracked", False):
        return
    start = QTimer.start
    single = QTimer.singleShot

    def tracked_start(self, *args):
        _TIMERS.add(self)
        return start(self, *args)

    def tracked_single(msec, *args, **kwargs):
        try:
            _SHOTS.append(time.monotonic() + max(0, int(msec)) / 1000.0)
        except (TypeError, ValueError):
            pass
        return single(msec, *args, **kwargs)

    QTimer.start = tracked_start
    QTimer.singleShot = staticmethod(tracked_single)
    QTimer._shell_tracked = True


def next_timer_ms() -> float | None:
    """Milliseconds until the first Python-started Qt timer is due (0 when
    one is), or None when none is pending."""
    now = time.monotonic()
    _SHOTS[:] = [t for t in _SHOTS if t > now - 0.05]
    best = None
    if _SHOTS:
        best = max(0.0, (min(_SHOTS) - now) * 1000.0)
    for timer in list(_TIMERS):
        try:
            if not timer.isActive():
                _TIMERS.discard(timer)
                continue
            left = max(0.0, float(timer.remainingTime()))
        except RuntimeError:          # its C++ object is gone
            _TIMERS.discard(timer)
            continue
        best = left if best is None else min(best, left)
    return best


# ---- Qt widgets that would wait for an answer ------------------------------------
class _Guard:
    """Rejects any top-level Qt widget the borrowed code shows (see the
    module's docstring). ``on_blocked(title)`` hears which."""

    def __init__(self):
        from PySide6.QtCore import QObject

        guard = self

        class Filter(QObject):
            def eventFilter(self, obj, event):  # noqa: N802 — Qt API
                from PySide6.QtCore import QEvent
                if event.type() == QEvent.Show:
                    guard._shown(obj)
                return False

        self.filter = Filter()
        self.on_blocked = None
        self.blocked = []

    def _shown(self, obj):
        from PySide6.QtCore import QTimer
        from PySide6.QtWidgets import QDialog, QMenu, QWidget
        if not isinstance(obj, QWidget) or not obj.isWindow():
            return
        title = obj.windowTitle() or type(obj).__name__
        self.blocked.append(title)
        if self.on_blocked is not None:
            try:
                self.on_blocked(title)
            except Exception:  # noqa: BLE001 — a report never stops the reject
                pass

        def drop(w=obj):
            try:
                if isinstance(w, QDialog):
                    w.reject()
                elif isinstance(w, QMenu):
                    w.close()
                else:
                    w.close()
            except RuntimeError:
                pass
        # Inside the dialog's own exec() loop: a posted reject ends it.
        QTimer.singleShot(0, drop)


def install_widget_guard():
    global _GUARD
    if _GUARD is None and _APP is not None:
        _GUARD = _Guard()
        _APP.installEventFilter(_GUARD.filter)
    return _GUARD


def widget_guard():
    return _GUARD


def pump() -> None:
    """Deliver posted events and due QTimers (QTimer.singleShot callers in
    core/tools/views). The Rust loop calls this once per turn; Qt's own
    loop (exec) never runs."""
    if _APP is not None:
        from PySide6.QtCore import QCoreApplication, QEventLoop
        QCoreApplication.sendPostedEvents()
        QCoreApplication.processEvents(QEventLoop.AllEvents, 0)
