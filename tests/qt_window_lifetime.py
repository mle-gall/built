"""Dispose an owned test window while its Qt application is still running."""
from PySide6.QtCore import QCoreApplication, QEvent


def close_window(window):
    assert window.close()
    window.deleteLater()
    QCoreApplication.sendPostedEvents(window, QEvent.DeferredDelete)
