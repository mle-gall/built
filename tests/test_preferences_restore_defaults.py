# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Window ▸ Preferences ▸ Restore defaults: the controls go back to what a
fresh profile gets; OK saves that, Cancel still touches nothing, and the
language, the AI Assistant and the shortcuts stay as they were."""
from __future__ import annotations

from core.settings import settings
from tests.preferences_chrome import Controls
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_render import chrome_forms


def _customised():
    """A profile someone has tuned: nothing at its default."""
    st = settings()
    st.setValue("General/autosave", "0")
    st.setValue("General/autosave_min", 30)
    st.setValue("General/backup", "0")
    st.setValue("General/undo_steps", 50)
    st.setValue("General/cursor_style", "arrow")
    st.setValue("General/ext_menus", "flat")
    st.setValue("section/ask_name", "0")
    st.setValue("new_document/scale_figure", "0")
    st.setValue("nav/invert_wheel", "1")
    st.setValue("nav/invert_orbit_y", "1")
    st.setValue("walk/look_sensitivity", 80)
    st.setValue("display/msaa", 0)
    st.setValue("import/obj_unit", "ft")
    st.setValue("import/dxf_unit", "in")
    st.setValue("georef/coord_mode", "utm")
    st.setValue("language", "fr")
    st.setValue("ia/api_key", "test-key-123")
    st.setValue("ia/modelo", "my-model")
    st.sync()


def confirmation(ui):
    ui.sheet.act(ui.ctx, "verb", ["restore"])
    return chrome_forms.current(ui.ctx.win)


def test_the_tab_picture_option_is_saved_and_restored(native_host):
    ui = Controls(native_host.window, "general")
    assert ui.value("hover_picture")
    ui.set("hover_picture", False)
    assert settings().value("tabs/hover_picture") == "0"
    reopened = Controls(native_host.window, "general")
    assert not reopened.value("hover_picture")
    reopened.restore()
    assert reopened.value("hover_picture")


def test_the_dialog_has_a_restore_defaults_button(native_host):
    footer = Controls(native_host.window, "general").wire()["footer"]
    assert [v["id"] for v in footer["links"]] == ["restore"]


def test_ok_stays_the_highlighted_button_after_restoring(native_host):
    ui = Controls(native_host.window, "general")
    form = confirmation(ui)
    assert form.default == "ok"
    form.press("ok")
    assert chrome_forms.current(ui.ctx.win) is None
    assert not ui.wire().get("default_verb")


def test_restore_puts_every_control_back_to_its_default(native_host):
    from core.ndof import AXES
    _customised()
    ui = Controls(native_host.window, "import")
    assert ui.value("dxf_unit") == "in"
    assert not ui.select("general").value("autosave")
    ui.select("display").set("rest_mode", "fade")
    ui.set("rest_fade", 30)
    ui.select("ndof").set("ndof_inv_right", True)
    ui.set("ndof_speed_right", 200)
    assert ui.value("ndof_preset") == "custom"
    ui.restore()
    expected = {
        "general": dict(autosave=True, autosave_min=5, backup=True, undo_steps=200,
                        scale_figure=True),
        "navigation": dict(ask_section=True, invert_wheel=False, invert_orbit=False,
                           look_sens=25, cursor_style="tool"),
        "display": dict(rest_mode="fade", rest_fade=75),
        "import": dict(dxf_unit="mm"),
        "ndof": dict(ndof_on=True, ndof_speed=100, ndof_preset="original"),
    }
    for section, values in expected.items():
        ui.select(section)
        for key, value in values.items():
            assert ui.value(key) == value, key
    for axis in AXES:
        assert not ui.value("ndof_inv_" + axis)
        assert ui.value("ndof_speed_" + axis) == 100
    ui.sheet.closed(ui.ctx)


def test_ok_saves_the_defaults_and_keeps_language_and_ai(native_host):
    _customised()
    win = native_host.window
    ui = Controls(win, "general")
    confirmation(ui).press("ok")
    st = settings()
    assert st.value("General/autosave") == "1"
    assert int(st.value("General/autosave_min")) == 5
    assert int(st.value("General/undo_steps")) == 200
    assert st.value("nav/invert_wheel") == "0"
    assert int(st.value("walk/look_sensitivity")) == 25
    assert st.value("import/obj_unit") == "ft"
    assert win.viewport.edit_rest_mode == "fade"
    assert st.value("language") == "fr"
    assert st.value("ia/api_key") == "test-key-123"
    assert st.value("ia/modelo") == "my-model"


def test_cancel_after_restore_touches_nothing(native_host):
    _customised()
    win = native_host.window
    win.viewport.set_edit_rest_mode("hide")
    confirmation(Controls(win, "general")).press("cancel")
    st = settings()
    assert st.value("import/obj_unit") == "ft"
    assert st.value("General/autosave") == "0"
    assert win.viewport.edit_rest_mode == "hide"


def test_restore_does_not_copy_the_document_units_to_new_documents(native_host):
    from core import units
    from ingetrazo_render.settings_sheet import SettingsSheet

    win = native_host.window
    units.apply_units(win.viewport.scene, {"length": "ft", "precision": 2})
    units.remember_new_document_units({"length": "cm", "precision": 1})
    settings().setValue("units/use_for_new_documents", "0")
    units.set_ask_at_start(False)
    SettingsSheet().restore_defaults(win)
    assert settings().value("units/use_for_new_documents") == "1"
    assert units.ask_at_start()
    assert units.new_document_units() == {"length": "cm", "precision": 1}
    assert win.viewport.scene.units == {"length": "ft", "precision": 2}


def test_the_button_asks_first(native_host):
    _customised()
    ui = Controls(native_host.window, "import")
    form = confirmation(ui)
    assert ui.value("dxf_unit") == "in"
    form.press("cancel")
    assert ui.value("dxf_unit") == "in"
    confirmation(ui).press("ok")
    assert ui.value("dxf_unit") == "mm"
