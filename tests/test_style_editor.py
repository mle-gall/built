# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The style editor: the user style library (core) and the tray panel (UI).

The library lives in QSettings ("styles/user", a JSON list) like the custom
basemap sources; ``style_by_name`` resolves built-ins first, then the
library — that is what lets a composer frame reference a saved style by
``"style:<name>"``. The panel edits ``scene.display_style`` in place, live.
"""
from __future__ import annotations

import pytest
from ingetrazo_core import NativeSettings
from tests.preferences_chrome import Controls
from tests.shell_isolation import restore_shell_patches  # noqa: F401

import core.style as style_mod  # noqa: E402
from core.scene import Scene  # noqa: E402
from core.style import (  # noqa: E402
    Style,
    delete_user_style,
    save_user_style,
    style_by_name,
    user_styles,
)


@pytest.fixture
def settings_file(tmp_path, monkeypatch):
    """Point the style library at a throwaway INI file."""
    store = NativeSettings(str(tmp_path))
    monkeypatch.setattr(style_mod, "_settings", store.fresh)
    return store


def test_save_load_round_trip(settings_file):
    st = Style(name="Obra", face_mode="hidden_line",
               edge_color=(0.1, 0.2, 0.3), sky=False)
    save_user_style(st)
    loaded = user_styles()
    assert [s.name for s in loaded] == ["Obra"]
    assert loaded[0].face_mode == "hidden_line"
    assert loaded[0].edge_color == (0.1, 0.2, 0.3)
    assert loaded[0].sky is False


def test_save_replaces_by_name_and_delete(settings_file):
    save_user_style(Style(name="Obra", face_mode="shaded"))
    save_user_style(Style(name="Obra", face_mode="wireframe"))
    save_user_style(Style(name="Croquis"))
    assert [s.name for s in user_styles()] == ["Obra", "Croquis"]
    assert user_styles()[0].face_mode == "wireframe"   # replaced, not doubled

    assert delete_user_style("Obra") is True
    assert delete_user_style("Obra") is False          # already gone
    assert [s.name for s in user_styles()] == ["Croquis"]


def test_builtin_name_is_refused(settings_file):
    with pytest.raises(ValueError):
        save_user_style(Style(name="Hidden line"))
    assert user_styles() == []


def test_style_by_name_resolves_user_styles_as_copies(settings_file):
    save_user_style(Style(name="Obra", face_mode="monochrome"))
    got = style_by_name("Obra")
    assert got is not None and got.face_mode == "monochrome"
    got.face_mode = "xray"                             # a COPY: no write-back
    assert style_by_name("Obra").face_mode == "monochrome"
    # Built-ins still win and still resolve.
    assert style_by_name("Wireframe").face_mode == "wireframe"
    assert style_by_name("no existe") is None


def test_broken_settings_entry_is_dropped(settings_file):
    s = settings_file
    s.setValue("styles/user", "{esto no es json")
    s.sync()
    assert user_styles() == []


@pytest.fixture
def editor(native_host, settings_file):
    return Controls(native_host.window, "appearance")


def test_panel_pick_applies_a_preset(editor):
    assert "Hidden line" in editor.options("style_preset")
    editor.set("style_preset", "Hidden line")
    assert editor.env.scene.display_style.face_mode == "hidden_line"
    assert not editor.env.scene.display_style.sky


def test_panel_edits_the_active_style_live(editor):
    editor.set("edges", False)
    editor.set("face_mode", "xray")
    assert not editor.env.win.active_style().edges
    assert editor.env.win.active_style().face_mode == "xray"


def test_every_swatch_stays_editable_and_hints_when_invisible(editor):
    swatches = ("edge_color", "front_color", "back_color", "sky_color", "ground_color", "background", "section_fill_color")
    assert all(editor.enabled(k) for k in swatches)
    assert {k for k in swatches if editor.row(k).tip} == {"background"}
    for k in ("sky", "edges", "section_fill"):
        editor.set(k, False)
    assert {k for k in swatches if editor.row(k).tip} == {"sky_color", "ground_color", "edge_color", "section_fill_color"}
    editor.set("face_mode", "wireframe")
    assert editor.row("front_color").tip and not editor.row("edge_color").tip
    assert all(editor.enabled(k) for k in swatches)


def test_sky_gradient_ramps_from_haze_to_the_picked_tones():
    """The backdrop gradient: the user's sky colour rules the zenith and the
    ground colour rules the foreground, both hazing toward white at the
    horizon. Pure math (no GL) — the quads only colour their corners, so
    these ARE the rendered colours."""
    from pytest import approx
    from tests.controller_surface import ControllerSurface as Viewport
    sky = (0.2, 0.4, 0.9)                       # a deep blue
    gnd = (0.4, 0.3, 0.2)
    s0, s1, g0, g1, line = Viewport._sky_gradient(sky, gnd, 0.0)
    # Near the horizon both halves are pulled toward white (haze)…
    assert s0[0] > sky[0] and g0[0] > gnd[0]
    # …and away from it they approach the picked tones (blue: red channel
    # falls toward 0.2 going up; ground darkens going down).
    assert s1[0] < s0[0] and g1[0] < g0[0]
    assert all(0.0 <= v <= 1.0 for c in (s0, s1, g0, g1, line) for v in c)

    # Camera pitched up: the whole visible sky is zenith colour.
    s0, s1, _g0, _g1, _l = Viewport._sky_gradient(sky, gnd, -3.0)
    assert s0 == approx(sky) and s1 == approx(sky)
    # Pitched down: the visible ground is fully the ground colour.
    _s0, _s1, g0, g1, _l = Viewport._sky_gradient(sky, gnd, 3.0)
    assert g0 == approx(gnd) and g1 == approx(gnd)


def test_sky_and_ground_colors_are_style_data(settings_file):
    """The backdrop tones live in the Style (serialised, library-saved) and
    the viewport reads them per frame — Marco asked for the sky colour and
    it was a hardcoded constant."""
    st = Style(name="Tarde", sky_color=(0.9, 0.7, 0.5),
               ground_color=(0.4, 0.3, 0.2))
    save_user_style(st)
    got = style_by_name("Tarde")
    assert got.sky_color == (0.9, 0.7, 0.5)
    assert got.ground_color == (0.4, 0.3, 0.2)
    # An old style dict without the fields gets the historical defaults —
    # which are the viewport's former constants, so old documents render
    # identically.
    from tests.controller_surface import ControllerSurface as Viewport
    legacy = Style.from_dict({"name": "vieja"})
    assert legacy.sky_color == Viewport._SKY_RGB
    assert legacy.ground_color == Viewport._GROUND_RGB


def test_panel_combo_lists_saved_styles_and_arms_delete(editor):
    save_user_style(Style(name="Obra", face_mode="hidden_line"))
    assert "Obra" in editor.options("style_preset")
    editor.set("style_preset", "Obra")
    assert editor.value("style_preset") == "Obra" and editor.enabled("delete_style")
    editor.row("delete_style").press(editor.env)
    assert "Obra" not in editor.options("style_preset")
    assert editor.value("style_preset") == "Default" and not editor.enabled("delete_style")


# ---- Back color (the usual Front/Back color pair) ----------------------------

def test_back_color_is_style_data_and_round_trips(settings_file):
    """The back-face tint lives in the Style next to the front colour:
    serialised, library-saved, copied. Unset (``None``) stays automatic."""
    from core.style import DEFAULT_BACK_COLOR, effective_back_color
    assert Style().back_color is None
    assert effective_back_color(Style()) == DEFAULT_BACK_COLOR

    st = Style(name="Interior", back_color=(0.9, 0.2, 0.1))
    again = Style.from_dict(st.to_dict())
    assert again.back_color == (0.9, 0.2, 0.1)
    assert Style.from_dict(Style().to_dict()).back_color is None
    save_user_style(st)
    assert style_by_name("Interior").back_color == (0.9, 0.2, 0.1)
    # Old documents (no key) and junk values fall back to automatic.
    assert Style.from_dict({}).back_color is None
    assert Style.from_dict({"back_color": "rojo"}).back_color is None
    assert Style.from_dict({"back_color": [1, 2]}).back_color is None


def test_back_color_precedence_style_then_document_then_default():
    """A picked Back color wins over the tint adopted from an imported .skp
    (``scene.back_face_color``); with none picked the adopted tint still
    shows, exactly as before this option existed."""
    from core.style import DEFAULT_BACK_COLOR, effective_back_color
    scene = Scene()
    style = scene.display_style
    assert effective_back_color(style, scene) == DEFAULT_BACK_COLOR
    scene.back_face_color = (0.5, 0.5, 0.5)
    assert effective_back_color(style, scene) == (0.5, 0.5, 0.5)
    style.back_color = (0.1, 0.8, 0.3)
    assert effective_back_color(style, scene) == (0.1, 0.8, 0.3)


def test_panel_back_swatch_shows_the_tint_that_draws(editor):
    assert editor.value("back_color") == "#9eb2c7"
    editor.env.scene.back_face_color = (0.5, 0.5, 0.5)
    assert editor.value("back_color") == "#808080"
    editor.set("back_color", "#ff0000")
    assert editor.env.win.active_style().back_color == (1., 0., 0.)
    assert editor.value("back_color") == "#ff0000"
    assert editor.value("front_color") == "#ffffff"


def test_back_color_hint_only_where_back_faces_cannot_show(editor):
    for mode in ("textures", "shaded", "monochrome", "xray", "hidden_line", "wireframe"):
        editor.set("face_mode", mode)
        assert bool(editor.row("back_color").tip) == (mode in ("hidden_line", "wireframe"))
