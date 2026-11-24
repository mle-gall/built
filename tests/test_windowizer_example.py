# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Windowizer example extension (Bane Andreev's port of Rick Wilson's
Windowizer 3): windows from faces, as one container group each — and the
extension API it needed: add_menu, add_context_menu and group.ext."""
from __future__ import annotations

from pathlib import Path

import pytest
from ingetrazo_core import Vec3 as V
from core import extensions
from core.history import AddFaceCommand
from examples.extensions import windowizer
from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
from views import chrome_dialogs, chrome_forms


@pytest.fixture
def win(monkeypatch, request):
    examples = Path(windowizer.__file__).parent
    monkeypatch.setattr(extensions, "plugin_dirs", lambda: [examples])
    return request.getfixturevalue("native_host").window


def _wall(vp):
    """A 4 m × 3 m wall, 25 cm thick, and a 1.5 × 1.2 m rectangle drawn on
    its front face — what a user does before Windowize."""
    m = vp.scene.mesh
    p = [V(0, 0, 0), V(4, 0, 0), V(4, .25, 0), V(0, .25, 0),
         V(0, 0, 3), V(4, 0, 3), V(4, .25, 3), V(0, .25, 3)]
    for loop in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4),
                 (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)):
        m.add_face([p[i] for i in loop])
    vp.history.execute(AddFaceCommand(
        [V(1, 0, 1), V(2.5, 0, 1), V(2.5, 0, 2.2), V(1, 0, 2.2)]))
    return next(f for f in m.faces if abs(f.area() - 1.8) < 1e-6)


def _windowize(win, monkeypatch, **params):
    mod = windowizer
    vp = win.viewport
    face = _wall(vp)
    vp.scene.selection.clear()
    vp.scene.selection.add(face)
    monkeypatch.setattr(chrome_dialogs, "async_host", lambda parent: win)
    mod.windowize(vp)
    form = chrome_forms.forms(win)[-1]
    for key, value in params.items():
        form.receive(key, value)
    form.press("ok")
    assert not form.error
    wins = [g for g in vp.scene.groups if mod.window_data(g) is not None]
    assert len(wins) == 1
    return mod, vp, wins[0]


def test_a_face_on_a_wall_becomes_a_window_in_one_undo(native_window, monkeypatch):
    mod, vp, g = _windowize(native_window, monkeypatch, rows="1", cols="2")
    assert g.ifc["class"] == "IfcWindow"
    names = sorted(c.name for c in g.children)
    assert names == ["Frame", "Glass 1", "Glass 2"]
    assert mod.KEY in g.ext and "windowizer" not in g.ifc
    assert vp.history.undo()
    assert g not in vp.scene.groups
    assert vp.history.redo()
    assert g in vp.scene.groups


def test_retagging_in_bim_keeps_the_window_editable(native_window, monkeypatch):
    mod, vp, g = _windowize(native_window, monkeypatch)
    g.ifc = {"class": "IfcWindow", "name": "V-01"}     # what the BIM panel does
    assert mod.window_data(g) is not None


def test_copies_keep_their_own_parameters(native_window, monkeypatch):
    from core.group import copy_group
    mod, vp, g = _windowize(native_window, monkeypatch)
    c = copy_group(g)
    assert mod.window_data(c) == mod.window_data(g)
    c.ext[mod.KEY]["params"]["rows"] = "5"
    assert mod.window_data(g)["params"]["rows"] != "5"


def test_edit_rebuilds_in_place(native_window, monkeypatch):
    mod, vp, g = _windowize(native_window, monkeypatch, rows="1", cols="1")
    vp.scene.selection.clear()
    vp.scene.selection.add(g)
    mod.edit(vp)
    form = chrome_forms.forms(native_window)[-1]
    form.receive("rows", "2")
    form.receive("cols", "3")
    form.press("ok")
    assert not form.error
    assert sum(c.name.startswith("Glass") for c in g.children) == 6
    assert vp.history.undo()
    assert sum(c.name.startswith("Glass") for c in g.children) == 1


def test_a_saved_window_reopens_editable(native_window, monkeypatch, tmp_path):
    from core.scene import Scene
    from formats import igz
    mod, vp, g = _windowize(native_window, monkeypatch)
    path = tmp_path / "wall.igz"
    igz.save_scene(vp.scene, path)
    again = Scene()
    igz.load_into(again, path)
    wins = [x for x in again.groups if mod.window_data(x) is not None]
    assert len(wins) == 1
    assert wins[0].ext[mod.KEY]["params"] == g.ext[mod.KEY]["params"]


def test_erase_closes_the_wall_again(native_window, monkeypatch):
    mod, vp, g = _windowize(native_window, monkeypatch)
    vp.scene.selection.clear()
    vp.scene.selection.add(g)
    mod.erase(vp)
    assert g not in vp.scene.groups
    # Back to what the user drew: the wall face with the rectangle in it,
    # and the back of the wall whole again (the cut went through it).
    front = sorted(round(f.area(), 3) for f in vp.scene.mesh.faces
                   if abs(f.normal().y() + 1) < 1e-6)
    assert front == [1.8, 12.0]
    back = [f for f in vp.scene.mesh.faces if abs(f.normal().y() - 1) < 1e-6]
    assert [len(f.holes) for f in back] == [0]


def test_its_menus_come_from_the_extension_api(win, monkeypatch):
    from unittest.mock import Mock
    from plugins._context_menu import context_menu_action
    rows = win._extension_menus.rows()
    assert any(path == ["Windowizer"] and label == "Windowize selected faces"
               for _, path, label, *_ in rows)
    vp = win.viewport
    vp.scene.select([_wall(vp)])
    view = win.view
    monkeypatch.setattr(view, "renderer", Mock())
    monkeypatch.setattr(chrome_dialogs, "async_host", lambda parent: win)
    win.show_viewport_context_menu()
    rendered = view.renderer.ui_set_context_menu.call_args.args[-1]
    action = next(key for _, verbs in rendered for key, name, *_ in verbs
                  if name == "Windowizer ▸ Windowize selected faces")
    assert context_menu_action(win, view, action)
    assert chrome_forms.forms(win)[-1].field("frame_w") is not None


def test_the_project_browser_reads_a_window_as_a_window(win, monkeypatch):
    mod, vp, g = _windowize(win, monkeypatch, rows="2", cols="3")
    from unittest.mock import Mock
    from views.wgpu_ui import _tree
    view = win.view
    monkeypatch.setattr(view, "renderer", Mock())
    view._ui_exp = {"n:"}

    def window_row():
        _tree(view, win)
        tree = view._ui_tree["tree"]
        key = "i:" + tree.row_id(g)
        rows = view.renderer.ui_set_tree.call_args.args[0]
        return next(text for text, flags in rows if text[0] == key)

    label, _, tip = window_row()[2].partition("\x1e")
    assert label == "Window"
    assert "Panes: 3 × 2" in tip and "Width:" in tip and "Height:" in tip
    view._ui_filter = "window"
    assert window_row()[1] == g.name
