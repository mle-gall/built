# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Bridge (issue #614) in the window: Tools menu, right-click menu, one
undo step."""
from __future__ import annotations

import pytest
from ingetrazo_core import Vec3 as QVector3D
from ingetrazo_render import rail_layout
from core.orient import is_closed
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_render import context_menu_build


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


@pytest.fixture
def win(native_host):
    return native_host.window


def _context_entries(win, selection):
    win.viewport.scene.select(selection)
    return context_menu_build(win, win.view).ids()


def _two_squares(scene):
    mesh = scene.mesh
    a = mesh.add_face([V(0, 0), V(1, 0), V(1, 1), V(0, 1)])
    b = mesh.add_face([V(0, 0, 2), V(1, 0, 2), V(1, 1, 2), V(0, 1, 2)])
    return a, b


def test_tools_menu_has_bridge(win):
    assert win._command_paths["menu:bridge"] == "Tools"
    assert win._registry_actions["menu:bridge"].text() == "Bridge"


def test_modify_toolbar_has_a_bridge_button_after_fillet(win):
    tools = next(family for family in rail_layout() if ("bridge", "_act_bridge") in family)
    assert tools[tools.index(("fillet", "")) + 1] == ("bridge", "_act_bridge")
    assert win._act_bridge is win._registry_actions["menu:bridge"]
    assert win._act_bridge.portable_keys() == []


def test_the_button_bridges_the_selection(win):
    scene = win.viewport.scene
    a, b = _two_squares(scene)
    scene.select([a, b])
    win._act_bridge.trigger()
    assert len(scene.mesh.faces) == 6 and is_closed(scene.mesh)


def test_bridge_from_the_window_is_one_undo_step(win):
    scene = win.viewport.scene
    a, b = _two_squares(scene)
    scene.select([a, b])
    win._on_bridge()
    assert len(scene.mesh.faces) == 6 and is_closed(scene.mesh)
    # The new sides end up selected.
    assert len([e for e in scene.selection if e in scene.mesh.faces]) == 4
    win.viewport.history.undo()
    assert len(scene.mesh.faces) == 2
    win.viewport.history.redo()
    assert len(scene.mesh.faces) == 6


def test_a_wrong_selection_changes_nothing(win):
    scene = win.viewport.scene
    a, _b = _two_squares(scene)
    scene.select([a])
    win._on_bridge()
    assert len(scene.mesh.faces) == 2
    assert not win.viewport.history.undo_stack


def test_right_click_offers_bridge_only_when_it_can(win):
    a, b = _two_squares(win.viewport.scene)
    assert "bridge" in _context_entries(win, [a, b])
    assert "bridge" not in _context_entries(win, [a])


def test_right_click_keeps_texture_on_one_textured_face(win, monkeypatch):
    # The Bridge check went in above the Texture submenu once and took it
    # along: one textured face (never bridgeable) lost Texture ▸ Position.
    a, _b = _two_squares(win.viewport.scene)
    monkeypatch.setattr(win, "_single_textured_face", lambda: (a, "front"))
    entries = _context_entries(win, [a])
    assert "texpos" in entries and "bridge" not in entries


def test_a_big_selection_is_not_walked(win, monkeypatch):
    # Every right-click asks; with Ctrl+A on thousands of faces walking the
    # selection took seconds. A third face is enough to say no.
    import core.bridge

    def walked(*_args):
        raise AssertionError("profiles_from_selection walked the selection")
    monkeypatch.setattr(core.bridge, "profiles_from_selection", walked)
    mesh = win.viewport.scene.mesh
    faces = [mesh.add_face([V(i * 2, 0), V(i * 2 + 1, 0), V(i * 2 + 1, 1),
                            V(i * 2, 1)]) for i in range(3)]
    assert not win._bridgeable(faces)
