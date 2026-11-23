# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Window ▸ Preferences: the scattered QSettings gathered in one dialog.

OK writes and flushes; Cancel touches nothing; the rest-of-model mode goes
through the viewport (the same live path the Camera menu uses).
"""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from PySide6.QtCore import QSettings
from PySide6.QtWidgets import QApplication, QWidget

_inst = QApplication.instance()
if _inst is None:
    _app = QApplication([])
elif not isinstance(_inst, QApplication):
    pytest.skip("a non-widget QGuiApplication is already active",
                allow_module_level=True)

from tests.preferences_chrome import Controls, settings_file


class _Win(QWidget):
    """Just enough main window: a viewport with the rest-mode contract."""

    def __init__(self):
        super().__init__()

        class _VP:
            edit_rest_mode = "fade"
            edit_rest_fade = 0.75

            def set_edit_rest_mode(self, mode):
                self.edit_rest_mode = mode

            def set_edit_rest_fade(self, strength):
                self.edit_rest_fade = strength

        self.viewport = _VP()
        self._rest_actions = {}


def _fresh(path):
    return QSettings(str(path), QSettings.IniFormat)


def test_ok_writes_the_settings(settings_file):
    win = _Win()
    ui = Controls(win, "import")
    ui.set("obj_unit", "cm")
    ui.select("import").set("dxf_unit", "m")
    ui.select("import").set("coord_mode", "utm")
    ui.select("ai").set("ai_model", "llama-3.3-70b")
    ui.select("ai").set("ai_shots", False)
    ui.select("general").set("autosave_min", 10)
    ui.select("general").set("backup", False)
    ui.select("navigation").set("invert_wheel", True)
    ui.select("navigation").set("invert_orbit", True)
    ui.select("navigation").set("look_sens", 40)
    ui.select("display").set("msaa", 8)
    st = _fresh(settings_file)
    assert st.value("import/obj_unit") == "cm"
    assert st.value("import/dxf_unit") == "m"
    assert st.value("georef/coord_mode") == "utm"
    assert st.value("ia/modelo") == "llama-3.3-70b"
    assert st.value("ia/capturas") == "0"
    assert st.value("General/autosave") == "1"
    assert int(st.value("General/autosave_min")) == 10
    assert st.value("General/backup") == "0"
    assert st.value("nav/invert_wheel") == "1"
    assert st.value("nav/invert_orbit_y") == "1"
    assert int(st.value("walk/look_sensitivity")) == 40
    assert int(st.value("display/msaa")) == 8
    # The live pieces reach the viewport immediately.
    assert win.viewport._invert_wheel is True
    assert win.viewport._invert_orbit_y is True
    assert win.viewport._msaa == 8
    assert win.viewport._fbo_size is None   # next paint rebuilds the FBO


def test_cancel_touches_nothing(settings_file):
    win = _Win()
    ui = Controls(win, "import")
    ui.wire()
    ui.select("display").wire()
    ui.sheet.closed(ui.ctx)
    assert _fresh(settings_file).value("import/obj_unit") is None
    assert win.viewport.edit_rest_mode == "fade"
    assert win.viewport.edit_rest_fade == 0.75


def test_rest_mode_applies_through_the_viewport(settings_file):
    win = _Win()
    ui = Controls(win, "display")
    assert ui.value("rest_mode") == "fade"        # mirrors the viewport
    assert ui.value("rest_fade") == 75
    ui.set("rest_fade", 42)
    ui.set("rest_mode", "hide")
    assert not ui.enabled("rest_fade")
    assert win.viewport.edit_rest_mode == "hide"
    assert win.viewport.edit_rest_fade == 0.42


def test_dialog_reloads_saved_values(settings_file):
    st = _fresh(settings_file)
    st.setValue("import/dxf_unit", "in")
    st.setValue("ia/proveedor", "auto")
    st.sync()
    win = _Win()                 # referenced: see the theme test below
    ui = Controls(win, "import")
    assert ui.value("dxf_unit") == "in"
    assert ui.select("ai").value("ai_provider") == "auto"


def test_toolbar_icon_size_lives_in_preferences_and_reaches_every_toolbar(settings_file):
    from views.icons import save_toolbar_icon_px, toolbar_icon_px
    st = _fresh(settings_file)
    st.setValue("ui/large_toolbar_icons", "1")
    st.sync()
    assert toolbar_icon_px() == 32
    win = _Win()
    seen = []
    win.set_toolbar_icon_size = lambda px: (seen.append(px), save_toolbar_icon_px(px))
    ui = Controls(win, "general")
    assert ui.value("icon_px") == 32
    ui.set("icon_px", 40)
    assert seen == [40]
    assert int(st.value("ui/toolbar_icon_px")) == 40
    assert st.value("ui/large_toolbar_icons") is None
    assert toolbar_icon_px() == 40


def test_theme_choice_applies_at_once(settings_file):
    from views import theme
    app = QApplication.instance()
    before = theme.saved_theme()
    try:
        win = _Win()
        ui = Controls(win, "general")
        ui.set("theme", theme.LIGHT)
        assert theme.saved_theme() == theme.LIGHT
        assert app.palette().window().color().lightness() > 128
    finally:
        theme.save_theme(before)
        theme.apply_theme(app, theme.DARK)


def test_slate_is_offered_and_applies(settings_file):
    from views import theme
    app = QApplication.instance()
    before = theme.saved_theme()
    try:
        win = _Win()
        ui = Controls(win, "general")
        assert theme.SLATE in ui.options("theme")
        ui.set("theme", theme.SLATE)
        assert theme.saved_theme() == theme.SLATE
        assert "QPushButton" in app.styleSheet()
    finally:
        theme.save_theme(before)
        theme.apply_theme(app, theme.DARK)
    assert app.styleSheet() == ""


def test_the_3d_mouse_page_shows_the_device_live(settings_file, monkeypatch):
    """Nobody on the team has a SpaceMouse: the page shows the six numbers
    as the device sends them and how IngeTrazo reads them, so an owner can
    report a wrong sign by looking (#108)."""
    from core.ndof import NdofSample
    from views import ndof_input

    dev = ndof_input.NdofInput()
    monkeypatch.setattr(ndof_input, "_shared", dev)
    monkeypatch.setattr(type(dev), "backend_name",
                        property(lambda self: "rawinput"))
    from views import chrome_sheet, settings_sheet
    win = _Win()
    settings_sheet.show(win, page="ndof")
    sheet = chrome_sheet.current(win)
    context = chrome_sheet.Ctx(win, None)
    sheet.build(context)
    readings = sheet._readings
    dev.raw.emit((12, -340, 0, 0, 0, 7))
    dev.motion.emit(NdofSample(forward=0.97), 0.01)
    from tests.test_settings_sheet import fields
    rows = fields(sheet.build(context).wire())
    assert "Y  -340" in rows["ndof_raw"][3]
    assert "RZ    +7" in rows["ndof_raw"][3]
    assert "+0.97" in rows["ndof_read"][3]
    chrome_sheet.close(win)
    dev.raw.emit((0, 0, 0, 0, 0, 0))             # closed: no longer listens
    assert "Y  -340" in readings.text(str)[0]


def test_the_tool_cursor_is_chosen_in_preferences(settings_file):
    """#858: the crosshair or the plain arrow instead of the tool's icon is a
    choice in Preferences ▸ General, saved where ``tool_cursor`` reads it and
    applied to the active tool at once; the tool's icon stays the default."""
    win = _Win()
    applied = []
    win.viewport._apply_tool_cursor = lambda: applied.append(True)
    ui = Controls(win, "general")
    ui.select("navigation")
    assert ui.value("cursor_style") == "tool"
    ui.set("cursor_style", "crosshair")
    assert _fresh(settings_file).value("General/cursor_style") == "crosshair"
    assert applied == [True]
    assert Controls(win, "navigation").value("cursor_style") == "crosshair"


def test_the_graphics_server_is_asked_only_where_there_is_a_choice(
        settings_file, monkeypatch):
    """Wayland or X11: Linux. On Windows the row spoke of KDE (#1179)."""
    from views import settings_apply
    for platform, expected in [("linux", True), ("win32", False), ("darwin", False)]:
        assert settings_apply.graphics_server_applies(platform) is expected
    ui = Controls(_Win(), "engine")
    monkeypatch.setattr(settings_apply, "graphics_server_applies", lambda: True)
    assert ui.row("platform").label == "Graphics server"
    monkeypatch.setattr(settings_apply, "graphics_server_applies", lambda: False)
    with pytest.raises(KeyError):
        ui.row("platform")
    assert _fresh(settings_file).value("general/platform", "auto") == "auto"
