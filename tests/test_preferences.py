# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Window ▸ Preferences: the scattered QSettings gathered in one dialog.

OK writes and flushes; Cancel touches nothing; the rest-of-model mode goes
through the viewport (the same live path the Camera menu uses).
"""
from __future__ import annotations


import pytest
from ingetrazo_core import NativeSettings


from tests.preferences_chrome import Controls, settings_file


class _Win:
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
    return NativeSettings(path.parent.parent)


def test_ok_writes_the_settings(settings_file):
    win = _Win()
    ui = Controls(win, "import")
    ui.select("import").set("dxf_unit", "m")
    ui.select("general").set("autosave_min", 10)
    ui.select("general").set("backup", False)
    ui.select("navigation").set("invert_wheel", True)
    ui.select("navigation").set("invert_orbit", True)
    ui.select("navigation").set("look_sens", 40)
    st = _fresh(settings_file)
    assert st.value("import/obj_unit") is None
    assert st.value("import/dxf_unit") == "m"
    assert st.value("georef/coord_mode") is None
    assert st.value("General/autosave") == "1"
    assert int(st.value("General/autosave_min")) == 10
    assert st.value("General/backup") == "0"
    assert st.value("nav/invert_wheel") == "1"
    assert st.value("nav/invert_orbit_y") == "1"
    assert int(st.value("walk/look_sensitivity")) == 40
    assert st.value("display/msaa") is None
    # The live pieces reach the viewport immediately.
    assert win.viewport._invert_wheel is True
    assert win.viewport._invert_orbit_y is True


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
    assert st.value("ia/proveedor") == "auto"


def test_toolbar_sizes_are_fixed_even_with_old_keys(settings_file):
    st = _fresh(settings_file)
    st.setValue("ui/toolbar_icon_px", 40)
    st.setValue("ui/large_toolbar_icons", "1")
    with pytest.raises(KeyError):
        Controls(_Win(), "general").row("icon_px")
    assert st.value("ui/large_toolbar_icons") == "1"


def test_old_theme_is_ignored_by_the_default_palette(settings_file):
    st = _fresh(settings_file)
    st.setValue("General/theme", "light")
    with pytest.raises(KeyError):
        Controls(_Win(), "general").row("theme")


def test_restore_leaves_retired_appearance_keys_intact(settings_file):
    st = _fresh(settings_file)
    st.setValue("General/theme", "slate")
    st.setValue("General/accent", "orange")
    Controls(_Win(), "general").restore()
    assert st.value("General/theme") == "slate"
    assert st.value("General/accent") == "orange"


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
    from ingetrazo_render import chrome_sheet
    from ingetrazo_render import settings_sheet
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


def test_no_display_server_switch_in_native_settings(settings_file):
    side = Controls(_Win(), "general").wire()["side"]
    assert all(row.get("id") != "engine" for row in side)
