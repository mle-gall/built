# SPDX-License-Identifier: GPL-3.0-or-later
"""Classic window services used by the document interaction bindings."""

def window_base():
    from PySide6.QtCore import QObject
    return QObject


def application():
    from PySide6.QtCore import QCoreApplication
    return QCoreApplication.instance()


def widget_event(widget, event):
    from PySide6.QtCore import QEvent, QObject
    return QObject.event(widget, event) if isinstance(event, QEvent) else False


def can_warp_pointer():
    from PySide6.QtGui import QGuiApplication
    name = QGuiApplication.platformName().lower()
    return not (name.startswith("wayland") or name in ("offscreen", "minimal"))


def socket_notifier(sock, owner, callback):
    from PySide6.QtCore import QSocketNotifier
    notifier = QSocketNotifier(sock.fileno(), QSocketNotifier.Read, owner)
    notifier.activated.connect(callback)
    return notifier


def raw_filter(backend):
    from PySide6.QtCore import QAbstractNativeEventFilter

    class Filter(QAbstractNativeEventFilter):
        def nativeEventFilter(self, event_type, message):
            try:
                if bytes(event_type) == b"windows_generic_MSG":
                    backend._on_msg(int(message))
            except Exception:
                pass
            return False, 0

    result = Filter()
    application().installNativeEventFilter(result)
    return result


def shortcut_type():
    from PySide6.QtGui import QKeySequence
    return QKeySequence
