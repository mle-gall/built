"""Without the Built UI (``BUILT_UI=0``), or on the GL view, the sheet
composer stays the Qt window it always was (views/wgpu_sheets.py decides).
Kept apart from tests/test_sheets_workspace.py: the native shell's host
patches the renderer choice for the module it runs in."""
import pytest  # noqa: F401
from PySide6.QtCore import Qt  # noqa: F401


# ---- BUILT_UI=0 -------------------------------------------------------------------------
def test_without_the_built_ui_the_composer_is_the_qt_window(monkeypatch):
    from views import wgpu_sheets, wgpu_ui
    from views.composer import ComposerWindow
    from views.main_window import MainWindow
    monkeypatch.setattr(wgpu_ui, "ENABLED", False)
    monkeypatch.setattr(wgpu_sheets, "FORCE", None)
    monkeypatch.setattr(ComposerWindow, "render_frame", lambda self, f: None)
    win = MainWindow()
    try:
        assert not wgpu_sheets.embeds(win)
        comp = win._ensure_composer()
        assert type(comp) is ComposerWindow and not comp.EMBEDDED
        win.show()
        win._to_sheets()
        assert comp.isVisible() and comp.isWindow()
        win._show_model()
        comp.close()
    finally:
        win._saved_version = win.viewport.scene.version
        win.close()
