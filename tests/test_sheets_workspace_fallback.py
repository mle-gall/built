def test_standalone_composer_escape_hatch(monkeypatch):
    from views import wgpu_sheets
    from views.composer import ComposerWindow
    from views.main_window import MainWindow
    monkeypatch.setattr(wgpu_sheets, "FORCE", False)
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
