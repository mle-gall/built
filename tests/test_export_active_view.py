# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Of several views, the exports of the view take the one worked in.

A 0.6.2 review on Windows: with four views and Top the active one, File ▸
Export ▸ Image wrote the Perspective view — the window's own — as the
view-line exports (DXF, SVG) would. Styles, Shift+P and the standard
views already went to the view clicked; now the picture, the lines and
the SVG do too."""
from __future__ import annotations

from tests.raster_samples import solid_raster
from tests.model_panes import grid  # noqa: F401
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def _spy(grid, monkeypatch, name, result, display=False):
    """Record which view's ``name`` runs; ``display``: on the view's
    display (``keys_stand_in``) instead of the view itself."""
    called = []
    for pane in grid.panes().values():
        def fake(*args, vp=pane.source_viewport, **kwargs):
            called.append(vp)
            return result() if callable(result) else result
        target = pane.source_viewport.keys_stand_in if display else pane.source_viewport
        monkeypatch.setattr(target, name, fake)
    return called


def test_export_image_renders_the_active_view(grid, monkeypatch, tmp_path):
    from ingetrazo_render import chrome_forms
    from views.filedialogs import file_dialogs
    from ingetrazo_render import shell_services as native
    monkeypatch.setattr(native, "available", lambda: True)
    win = grid.win
    top = grid.click("Top")

    out = tmp_path / "view.png"
    monkeypatch.setattr(file_dialogs, "getSaveFileName",
                        lambda *a, **k: (str(out), ""))

    called = _spy(grid, monkeypatch, "render_raster", lambda: solid_raster(64, 40, (0, 0, 0)))
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
    called = _spy(grid, monkeypatch, "render_pixels",
                  (8, 8, bytes(8 * 8 * 4)), display=True)
    builder = win.view_svg_builder()
    picture, _rect = builder._image_fn(64)
    assert called == [top]
    assert (picture.width, picture.height) == (8, 8)
