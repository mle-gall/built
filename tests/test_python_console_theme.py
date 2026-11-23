"""The Qt console uses the application’s dark colours for output and headings."""
from __future__ import annotations

from PySide6.QtWidgets import QApplication


_app = QApplication.instance() or QApplication([])


def test_the_console_uses_the_dark_palette():
    from plugins.python_console import _PALETTES, PythonConsoleDialog
    from views.main_window import MainWindow
    win = MainWindow()
    try:
        dlg = PythonConsoleDialog(win.viewport, win)
        dark = _PALETTES["dark"]
        assert dark["bg"] in dlg._output.styleSheet()
        html = dlg._output.document().toHtml()
        assert dark["title"] in html
        dlg.close()
    finally:
        win._saved_version = win.viewport.scene.version
        win.close()
