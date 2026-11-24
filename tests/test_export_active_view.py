# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Of several views, the exports of the view take the one worked in.

A 0.6.2 review on Windows: with four views and Top the active one, File ▸
Export ▸ Image wrote the Perspective view — the window's own — as the
view-line exports (DXF, SVG) would. Styles, Shift+P and the standard
views already went to the view clicked; now the picture, the lines and
the SVG do too."""
from __future__ import annotations

from PySide6.QtGui import QImage
from tests.model_panes import grid  # noqa: F401
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def _spy(grid, monkeypatch, name, result):
    called = []
    for pane in grid.panes().values():
        def fake(*args, vp=pane.source_viewport, **kwargs):
            called.append(vp)
            return result() if callable(result) else result
        monkeypatch.setattr(pane.source_viewport, name, fake)
    return called


def test_export_image_renders_the_active_view(grid, monkeypatch, tmp_path):
    from ingetrazo_render import chrome_forms
    from views.filedialogs import file_dialogs
    win = grid.win
    top = grid.click("Top")

    out = tmp_path / "view.png"
    monkeypatch.setattr(file_dialogs, "getSaveFileName",
                        lambda *a, **k: (str(out), ""))

    def picture():
        img = QImage(64, 40, QImage.Format_RGB32)
        img.fill(0)
        return img
    called = _spy(grid, monkeypatch, "render_image", picture)
    win._on_export_image()
    chrome_forms.current(win).press("ok")
    assert called == [top]
    assert out.exists()


def test_the_view_lines_and_the_svg_take_the_active_view(grid, monkeypatch):
    win = grid.win
    top = grid.click("Top")
    called = _spy(grid, monkeypatch, "hlr_geometry", None)
    win._view_line_groups()
    assert called and set(called) == {top}
    # The SVG builder draws its picture and faces from the same view.
    called = _spy(grid, monkeypatch, "render_image",
                  lambda: QImage(8, 8, QImage.Format_RGB32))
    builder = win.view_svg_builder()
    builder._image_fn(64)
    assert called == [top]
