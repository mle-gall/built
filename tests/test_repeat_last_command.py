# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Repeat last command: Blender's Shift+R, and right-click ▸ Repeat.

A tool comes back as if its key were pressed; a one-shot command (Reverse
Faces, Make Group, Intersect…) runs again on the CURRENT selection.

Only on deliberate gestures. Space stays the usual Select and Enter keeps
doing nothing on Select: a key pressed out of habit that repeated could
bring back the Eraser — the next click to select would erase — or run
Explode on whatever happens to be selected (Marco, 2026-09-26: «solo
Shift+R y clic derecho»).
"""
from __future__ import annotations

import pytest
from ingetrazo_core import Vec3 as V
from ingetrazo_render import context_menu_action, shortcut_text
from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.fixture
def win(native_host):
    native_host.attach_surface()
    return native_host.window


def repeat_row(win):
    return win.show_viewport_context_menu().find("repeat")


def _face(scene, x0=0.0):
    return scene.mesh.add_face([V(x0, 0, 0), V(x0 + 2, 0, 0),
                                V(x0 + 2, 2, 0), V(x0, 2, 0)])


def test_nothing_to_repeat_on_a_new_document(win):
    assert not repeat_row(win).enabled
    assert win.repeat_last_command() is False
    assert win.viewport.active_tool is win._tools["select"]


def test_shift_r_is_the_only_owner_of_its_key(win):
    """An ambiguous shortcut fires NEITHER action (the F2 lesson)."""
    owners = [key for key, act in win._registry_actions.items() if "Shift+R" in act.portable_keys()]
    assert owners == ["menu:repeat_last_command"]


def test_a_tool_comes_back_after_space(win):
    win._activate_tool("line")
    win._activate_tool("select")          # Space
    assert repeat_row(win).label == "Repeat Line"
    assert win.repeat_last_command()
    assert win.viewport.active_tool is win._tools["line"]
    assert win._tool_actions["line"].isChecked()


def test_select_and_context_tools_are_not_remembered(win):
    win._activate_tool("rectangle")
    win._activate_tool("select")
    win._activate_tool("texture_position")
    win._activate_tool("select")
    win.repeat_last_command()
    assert win.viewport.active_tool is win._tools["rectangle"]


def test_a_command_runs_again_on_the_new_selection(win):
    """The point of it: reverse this face, select the next, repeat."""
    scene = win.viewport.scene
    a, b = _face(scene), _face(scene, 5.0)
    na, nb = a.normal(), b.normal()
    scene.select([a])
    win._registry_actions["menu:reverse_faces"].trigger()
    assert V.dotProduct(a.normal(), na) < -0.99
    assert repeat_row(win).label == "Repeat Reverse Faces"
    scene.select([b])
    assert win.repeat_last_command()
    assert V.dotProduct(b.normal(), nb) < -0.99
    assert V.dotProduct(a.normal(), na) < -0.99, "a is left alone"


def test_the_right_click_entry_is_repeatable_after_its_menu_dies(win):
    scene = win.viewport.scene
    a, b = _face(scene), _face(scene, 5.0)
    nb = b.normal()
    scene.select([a])
    assert win.show_viewport_context_menu().find("reverse").enabled
    assert context_menu_action(win, win.view, "reverse")
    assert win.view._ctx_menu is None
    scene.select([b])
    assert win.repeat_last_command()
    assert V.dotProduct(b.normal(), nb) < -0.99


def test_intersect_repeats_with_the_same_mode(win, monkeypatch):
    import core.intersect as inter
    seen = []
    monkeypatch.setattr(inter, "segments_for",
                        lambda scene, mode: seen.append(mode) or [])
    scene = win.viewport.scene
    scene.select([_face(scene)])
    win._on_intersect_faces(inter.WITH_SELECTION)
    win.repeat_last_command()
    assert seen == [inter.WITH_SELECTION, inter.WITH_SELECTION]


def test_the_last_one_wins_tool_or_command(win):
    scene = win.viewport.scene
    scene.select([_face(scene)])
    win._activate_tool("move")
    win._activate_tool("select")
    win._on_reverse_faces()
    assert repeat_row(win).label == "Repeat Reverse Faces"
    win._activate_tool("circle")
    assert repeat_row(win).label == "Repeat Circle"


def test_enter_on_select_is_not_a_repeat(win):
    """Rhino and AutoCAD repeat on Enter; here it was tried and taken out."""
    scene = win.viewport.scene
    face = _face(scene)
    normal = face.normal()
    scene.select([face])
    win._on_reverse_faces()
    win._activate_tool("line")
    win._activate_tool("select")
    from ingetrazo_core import Input as Qt
    win._host.key(True, Qt.Key_Return, text="\r")
    win._host.key(False, Qt.Key_Return)
    assert win.viewport.active_tool is win._tools["select"]
    assert V.dotProduct(face.normal(), normal) < -0.99,         "the reversed face was not reversed back"


# ---- Where you see it: the right-click menu and the status bar ------------

def test_right_click_offers_the_repeat_first(win):
    scene = win.viewport.scene
    a, b = _face(scene), _face(scene, 5.0)
    nb = b.normal()
    scene.select([a])
    win._on_reverse_faces()
    scene.select([b])
    first = win.show_viewport_context_menu().verbs()[0]
    assert first.label == "Repeat Reverse Faces" and first.enabled
    assert context_menu_action(win, win.view, first.id)
    assert V.dotProduct(b.normal(), nb) < -0.99


def test_right_click_disables_repeat_until_a_command_runs(win):
    row = repeat_row(win)
    assert not row.enabled and not context_menu_action(win, win.view, row.id)


def test_keyboard_settings_identify_repeat_independently_of_its_label(win):
    from ingetrazo_render.keyboard_bindings import action_key
    action = win._registry_actions["menu:repeat_last_command"]
    before = action_key(action)
    win._activate_tool("line")
    assert repeat_row(win).label == "Repeat Line"
    assert action_key(action) == before == "menu:repeat_last_command"


def test_shift_r_is_not_a_key_the_viewport_reads_itself():
    assert shortcut_text("Shift+R")[3] is None


def test_the_repeat_hint_tracks_shortcut_edits_and_removal(win):
    win._activate_tool("line")
    win._activate_tool("select")
    action = win._registry_actions["menu:repeat_last_command"]
    action.setShortcuts(["Ctrl+Shift+Y"])
    assert repeat_row(win).key == shortcut_text("Ctrl+Shift+Y")[1]
    action.setShortcuts([])
    row = repeat_row(win)
    assert row.label == "Repeat Line" and row.key == ""
