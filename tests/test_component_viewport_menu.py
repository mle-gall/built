"""Definition actions in the native selection menu."""
from built_core import Vec3 as V
from built_render import context_menu_build, context_menu_run

from built_core.group import Group, copy_group
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_replace_component import _component, _model, _quad


def test_the_menu_offers_both_and_replace_swaps_the_selection(native_host, monkeypatch):
    from built_render import chrome_forms
    win = native_host.window
    scene = win.viewport.scene
    _, _, chairs, tables, group, holder = _model(moved=True)
    scene.groups.extend(chairs + tables + [group, holder])
    scene.select(chairs[:2])
    menu = context_menu_build(win, win.view)
    assert menu.find("selinstances") and menu.find("replace")
    assert context_menu_run(win, win.view, menu.kind, "selinstances", list(scene.selection))
    assert set(scene.selection) == set(chairs)
    assert [g.name for _, g in win._replacement_choices(chairs)] == ["Mesa"]
    monkeypatch.setattr(win, "_ask_keep_orphans", lambda orphans: False)
    assert context_menu_run(win, win.view, menu.kind, "replace", chairs)
    form = chrome_forms.current(win)
    assert "3" in form.field("item").label
    form.press("ok")
    assert all(g.mesh is tables[0].mesh for g in chairs)
    win.viewport.history.undo()
    assert all(g.name == "Silla" for g in chairs)


def test_no_replace_entry_without_another_definition(native_host):
    win = native_host.window
    scene = win.viewport.scene
    chair = _component(_quad(1.0), "Silla")
    scene.groups.extend([chair, copy_group(chair, V(4, 0, 0)),
                         Group(_quad(2.0), name="Muro")])
    scene.select([chair])
    menu = context_menu_build(win, win.view)
    assert menu.find("instances")
    assert menu.find("replace") is None


def test_select_instances_stays_inside_the_group_being_edited(native_host):
    from built_core.mesh import Mesh

    win = native_host.window
    scene = win.viewport.scene
    chair = _component(_quad(1.0), "Chair")
    twin = copy_group(chair, V(2, 0, 0))
    room = Group(Mesh(), name="Room")
    room.adopt([chair, twin])
    scene.groups.extend([room] + [copy_group(chair, V(i, 5, 0)) for i in range(3)])
    scene.begin_group_edit(room)
    scene.select([chair])
    menu = context_menu_build(win, win.view)
    assert context_menu_run(win, win.view, menu.kind, "instances", menu.sel)
    assert scene.edit_group is room and set(scene.selection) == {chair, twin}


def test_keep_in_file_from_the_viewport_menu(native_host):
    win = native_host.window
    scene = win.viewport.scene
    _, _, chairs, tables, group, holder = _model(moved=True)
    scene.groups.extend(chairs + tables + [group, holder])
    picked = [chairs[0], tables[1]]
    scene.select(picked)
    for checked in (0, 1):
        menu = context_menu_build(win, win.view)
        assert menu.find("keep").checked == checked
        assert context_menu_run(win, win.view, menu.kind, "keep", picked, facts=menu.facts)
        assert sorted(g.name for g in scene.kept_components) == ([] if checked else ["Mesa", "Silla"])
    win.viewport.history.undo()
    assert len(scene.kept_components) == 2
