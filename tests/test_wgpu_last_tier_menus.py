"""Modelling commands and Browser selection in one native document."""
from ingetrazo_core import Vec3
from core.group_scale import group_scale
from tests.shell_isolation import native_question_host, restore_shell_patches  # noqa: F401
from tests.native_tree import action
from tests.test_project_drawing_filters import Project
from views import scale_editor
from ingetrazo_render import chrome_forms
from ingetrazo_render import context_menu_build, context_menu_run


def test_shared_menus_and_dock_selection(native_question_host, monkeypatch):
    host = native_question_host
    win = host.window
    vp = win.viewport
    scene = vp.scene
    face = scene.mesh.add_face([Vec3(0, 0, 0), Vec3(2, 0, 0), Vec3(0, 2, 0)])
    scene.select([face])
    win._on_make_group()
    group = scene.groups[0]
    assert group in scene.selection
    win._on_make_component()
    form = chrome_forms.current(win)
    form.receive("text", "R3c component")
    form.press("ok")
    assert group.is_component()
    vp.begin_group_edit(group)
    assert scene.edit_group is group
    vp.end_one_group_edit()
    assert scene.edit_group is None
    scene.select([group])
    win._on_hide()
    assert group.hidden
    win._on_unhide_all()
    assert not group.hidden
    scene.select([group])
    menu = context_menu_build(win)
    assert context_menu_run(win, host.view, menu.kind, "lock", menu.sel, facts=menu.facts)
    assert group.locked
    win._unlock_group(group)
    assert not group.locked

    key = "i:" + group.uid
    browser = Project(scene=scene)
    assert browser.action("filter", "*")[key][1][9] == 1
    scene.clear_selection()
    action(host, "pickrow", key)
    assert group in scene.selection

    from views import chrome_dialogs
    monkeypatch.setattr(chrome_dialogs, "in_shell", lambda: True)
    scale_editor.ask_scale(win, group)
    form = chrome_forms.current(win)
    form.receive("x", "2")
    assert group_scale(group)[0] == 2
    form.press("undo")
    assert group_scale(group)[0] == 1
    form.press("redo")
    assert group_scale(group)[0] == 2
    form.press("ok")
    scene.select([group])
    win._on_explode_group()
    assert not scene.groups and scene.mesh.faces
    assert vp.history.undo() and group in scene.groups
