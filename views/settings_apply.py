# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Settings persistence and live application shared by the chrome and Qt host."""
from __future__ import annotations

from PySide6.QtCore import QSettings

from core import _native
from core import general_settings as _gs
from core.i18n import SYSTEM as SYSTEM_LANGUAGE
from core.i18n import current_language, resolve_language


#: The import dialogs' unit vocabularies (must match the dialogs in
#: main_window — the setting is their preselected answer).
_OBJ_UNITS = (("m", "Metres"), ("cm", "Centimetres"), ("mm", "Millimetres"),
              ("in", "Inches"), ("ft", "Feet"))
_DXF_UNITS = (("mm", "Millimetres"), ("cm", "Centimetres"), ("m", "Metres"),
              ("in", "Inches"), ("ft", "Feet"))
_UNITS_FOR_NEW_KEY = "units/use_for_new_documents"


def graphics_server_applies(platform: str | None = None) -> bool:
    """Whether choosing the display server means anything here: on Linux."""
    import sys
    return (platform or sys.platform).startswith("linux")


# ---- Applying one setting ---------------------------------------------------
# What OK does for each setting, one function each: the dialog calls them on
# OK, the Settings sheet (views/settings_sheet.py) the moment a value changes.
# ``st`` defaults to a fresh ``QSettings()``.

def viewports_of(window) -> list:
    """Every project's viewport (one per tab, #1066): a view setting
    reaches the tabs in the back too."""
    every = getattr(window, "all_viewports", None)
    if callable(every):
        return every()
    vp = getattr(window, "viewport", None)
    return [vp] if vp is not None else []


def native_views_of(window) -> list:
    """Every project's Metal view (``doc.stand_in``), if any."""
    docs = getattr(window, "_documents", None) or ()
    return [v for v in (getattr(d, "stand_in", None) for d in docs)
            if v is not None]


def _update(obj) -> None:
    update = getattr(obj, "update", None)
    if callable(update):
        update()


def apply_language(code, st=None) -> bool:
    """Language: same contract as the menu (persists; applies on restart).
    True when Built must restart to show it. Reverting a still-pending
    change back to the running language just rewrites the stored value —
    no restart note for a no-op (nor for "Same as the system" when the
    system's is the running language)."""
    st = st if st is not None else QSettings()
    saved = str(st.value("language") or SYSTEM_LANGUAGE)
    if code == saved:
        return False
    st.setValue("language", code)
    lang = resolve_language(code)
    if lang != current_language():
        from core.i18n import set_language
        set_language(lang)
        return True
    return False


def apply_theme_choice(theme) -> None:
    """Theme: applies at once (a no-op re-apply is cheap)."""
    from views.theme import apply_theme, save_theme, saved_theme
    if theme != saved_theme():
        save_theme(theme)
        from PySide6.QtWidgets import QApplication
        apply_theme(QApplication.instance(), theme)


def apply_accent_choice(accent) -> None:
    from views.theme import apply_accent, save_accent, saved_accent
    if accent != saved_accent():
        save_accent(accent)
        from PySide6.QtWidgets import QApplication
        apply_accent(QApplication.instance())


def apply_window_glass(on: bool) -> bool:
    """The Liquid Glass chrome (macOS 26+). True when it changed: it
    applies after a restart."""
    from views import mac_glass
    if not mac_glass.supported() or bool(on) == mac_glass.enabled():
        return False
    mac_glass.set_enabled(bool(on))
    return True


def apply_rest_mode(window, mode, fade: float) -> None:
    """Rest-of-model mode and fade: through each viewport (it persists the
    setting), then the Camera menu checkmark follows. Every project's
    viewport, so a tab in the back follows too (#1066)."""
    vp = getattr(window, "viewport", None)
    mode_changed = vp is not None and mode != vp.edit_rest_mode
    for v in viewports_of(window):
        set_fade = getattr(v, "set_edit_rest_fade", None)
        if callable(set_fade):
            set_fade(fade)
        if mode != v.edit_rest_mode:
            v.set_edit_rest_mode(mode)
    if mode_changed:
        act = getattr(window, "_rest_actions", {}).get(mode)
        if act is not None:
            act.setChecked(True)


def apply_autosave(window, on: bool, minutes: int, backup: bool | None = None,
                   st=None) -> None:
    """Auto-save and its pace (and the .bak), the timer re-armed."""
    st = st if st is not None else QSettings()
    _gs.set_value("autosave", "1" if on else "0", st)
    _gs.set_value("autosave_min", int(minutes), st)
    if backup is not None:
        _gs.set_value("backup", "1" if backup else "0", st)
    setup = getattr(window, "_setup_autosave", None)
    if callable(setup):
        setup()                     # re-arm the timer with the new pace


def apply_undo_steps(window, steps: int, st=None) -> None:
    st = st if st is not None else QSettings()
    _gs.set_value("undo_steps", int(steps), st)
    history = getattr(getattr(window, "viewport", None), "history", None)
    if history is not None:
        history.max_steps = int(steps)


def apply_rust_engine(on: bool, st=None) -> bool:
    """The Rust engine, read once at start. True when the choice changed
    (Built must restart to switch)."""
    st = st if st is not None else QSettings()
    rust = "1" if on else "0"
    if _native.available() is None or rust == str(
            _gs.value(_native.SETTING, _native.DEFAULT, st)):
        return False
    _gs.set_value(_native.SETTING, rust, st)
    _native.chosen_by_user(st)
    return True


def apply_invert_wheel(window, on: bool, st=None) -> None:
    st = st if st is not None else QSettings()
    st.setValue("nav/invert_wheel", "1" if on else "0")
    for v in viewports_of(window):
        v._invert_wheel = bool(on)


def apply_invert_orbit(window, on: bool, st=None) -> None:
    st = st if st is not None else QSettings()
    st.setValue("nav/invert_orbit_y", "1" if on else "0")
    for v in viewports_of(window):
        v._invert_orbit_y = bool(on)


def apply_slides(window, trackpad: str, touch: str, inertia: bool,
                 st=None) -> None:
    """The trackpad's two-finger slide, the touch screen's one-finger
    slide and the inertia after the fingers lift."""
    st = st if st is not None else QSettings()
    st.setValue("nav/trackpad_slide", trackpad)
    st.setValue("nav/touch_slide", touch)
    st.setValue("nav/trackpad_inertia", "1" if inertia else "0")
    for v in viewports_of(window):
        v._trackpad_slide = trackpad
        v._touch_slide = touch
        v._trackpad_inertia = bool(inertia)


def apply_cursor_style(window, style: str, st=None) -> None:
    st = st if st is not None else QSettings()
    _gs.set_value("cursor_style", style, st)
    reapply = getattr(getattr(window, "viewport", None),
                      "_apply_tool_cursor", None)
    if callable(reapply):
        reapply()                  # the active tool's pointer at once


def apply_msaa(window, msaa: int, st=None) -> None:
    st = st if st is not None else QSettings()
    st.setValue("display/msaa", msaa)
    for vp in viewports_of(window):
        if getattr(vp, "_msaa", None) != msaa:
            vp._msaa = msaa
            # Void the FBO size so the next paint rebuilds it at the new
            # sample count (the rebuild happens with the context current).
            vp._fbo_size = None
            _update(vp)


def apply_soft_shadows(window, on: bool, level: int, st=None) -> None:
    st = st if st is not None else QSettings()
    level = int(level)
    soft = level if on else 0
    st.setValue("display/soft_shadows", soft)
    st.setValue("display/soft_shadows_level", level)
    for vp in viewports_of(window):
        if getattr(vp, "_soft_shadows", None) != soft:
            vp._soft_shadows = soft
            _update(vp)
    # The Metal view over each project (its stand-in) reads the level
    # from its viewport on its next frame: ask for that frame now, so a
    # still view shows the new level without waiting for its repaint
    # tick or a camera move.
    for view in native_views_of(window):
        _update(view)


def apply_toolbar_icon_px(window, px: int) -> None:
    from views.icons import toolbar_icon_px
    px = int(px)
    if px != toolbar_icon_px():
        apply_px = getattr(window, "set_toolbar_icon_size", None)
        if callable(apply_px):
            apply_px(px)             # persists and resizes every toolbar
        else:
            from views.icons import save_toolbar_icon_px
            save_toolbar_icon_px(px)


def apply_document_units(window, chosen: dict) -> bool:
    """Units of the open document (issue #33): saved with it, the
    dimension style following. True when they changed (the document is
    edited)."""
    from core import units as _units
    vp = getattr(window, "viewport", None)
    scene = getattr(vp, "scene", None)
    if scene is None or chosen == _units.model_units_of(scene):
        return False
    _units.apply_units(scene, chosen)
    scene.version += 1
    notify = getattr(vp, "notify_scene_changed", None)
    if callable(notify):
        notify()
    else:
        vp.update()
    return True


def read_named_ndof_profiles() -> dict:
    """The saved, named SpaceMouse profiles: {name: NdofSettings}."""
    import json
    from core.ndof_profiles import profile_from_dict
    raw = QSettings().value("ndof/named_profiles", "{}")
    try:
        parsed = json.loads(str(raw))
        if not isinstance(parsed, dict):
            return {}
        return {name: profile_from_dict(data) for name, data in parsed.items()
                if isinstance(name, str) and 0 < len(name) <= 80}
    except (ValueError, TypeError):
        return {}


def ndof_preset_values(choice, named: dict):
    """``(mapping, inversion, speeds, global_speed)`` of a preset
    ("original", "freecad", "named:<name>"), or None ("custom", unknown)."""
    from core.ndof import (AXES, DEFAULT_MAPPING, FREECAD_INVERSION,
                           FREECAD_MAPPING)
    if choice == "original":
        return (DEFAULT_MAPPING, {axis: False for axis in AXES},
                {axis: 1.0 for axis in AXES}, 1.0)
    if choice == "freecad":
        return (FREECAD_MAPPING, FREECAD_INVERSION,
                {axis: 1.0 for axis in AXES}, 1.0)
    if isinstance(choice, str) and choice.startswith("named:"):
        profile = named.get(choice[6:])
        if profile is None:
            return None
        return (profile.axis_mapping, profile.axis_inversion,
                profile.axis_sensitivity, profile.sensitivity)
    return None


def legacy_ndof_inversion(nd) -> dict:
    """The per-axis inversion an Original configuration of the first
    settings meant (its pair switches folded into each axis)."""
    return {
        "right": nd.invert_pan != nd.invert_pan_x,
        "up": nd.invert_pan != nd.invert_pan_y,
        "forward": nd.invert_zoom,
        "tilt": nd.invert_rotate != nd.invert_tilt,
        "spin": nd.invert_rotate != nd.invert_spin,
        "roll": False,
    }


def save_ndof(nd, named: dict, selected, st=None) -> None:
    """Store the 3D mouse settings ``nd`` (its axes filled in), the named
    profiles and the preset picked (``selected``); every window reads
    them live."""
    import json
    from core.ndof_profiles import profile_to_dict
    from views.ndof_input import save_settings
    st = st if st is not None else QSettings()
    # Named presets are materialised into Custom at runtime. Their name
    # is stored separately so the dropdown restores on the next launch.
    nd.mapping_preset = selected if selected in ("original", "freecad") else "custom"
    # Original's legacy inversion is migrated into the new per-axis
    # representation. Old QSettings inversion keys are cleared on save.
    if selected == "original":
        nd.mapping_preset = "custom"
    st.setValue("ndof/named_profiles", json.dumps({
        name: profile_to_dict(profile) for name, profile in named.items()}))
    st.setValue("ndof/selected_profile", selected[6:]
                if isinstance(selected, str) and selected.startswith("named:")
                else "")
    save_settings(nd)                # every window reads it live


def reload_ai_panel(window) -> None:
    panel = getattr(window, "_ai_assistant", None)
    if panel is not None:          # the AI tab lives on: show the change
        panel._load_settings()
