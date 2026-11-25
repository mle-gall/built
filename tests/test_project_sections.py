# SPDX-License-Identifier: GPL-3.0-or-later
"""Section frame, cut and fill controls in the Project tree and inspector."""
import pytest

from built_core.group import Group
from tests.test_project_drawing_filters import Project
from tests.test_project_occurrences import _two_planes
from views import view_commands, wgpu_ui
from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.fixture
def project():
    p = Project()
    a, b = _two_planes(p.scene)
    p.rows()
    p.action("toggle", "sections:all")
    return p, a, b


def key(plane):
    return f"section:{plane.uid}"


def edit(p, plane, field, value=""):
    return p.action("edit", f"{key(plane)}:{field}{wgpu_ui.SEP}{value}")


def inspector(p, plane):
    p.action("item", key(plane))
    wgpu_ui._inspector(p.view, p.win)
    _, sections = p.view.renderer.calls["ui_set_inspector"]
    return {title: dict(entries) for title, entries in sections}


def test_the_section_folder_lists_every_plane_with_its_mode(project):
    p, a, b = project
    rows = p.rows()
    assert rows["sections:all"][1][1] == 2
    assert rows[key(a)][0] == "A — Plan cut"
    assert rows[key(a)][1][0] == rows[key(b)][1][0] == 1
    assert inspector(p, a)["Section"]["Mode"] == "Cut and frame"
    assert inspector(p, b)["Section"]["Mode"] == "Frame only"


def test_the_section_folder_keeps_its_name_when_the_tree_is_rebuilt(project):
    p, a, b = project
    p.scene.groups.append(Group(name="New"))
    p.scene.version += 1
    name, state = p.rows()["sections:all"]
    assert name == "Section planes" and state[1] == 2 and state[8] == 1


def test_the_frame_goes_and_the_cut_stays(project):
    p, a, _ = project
    rows = p.action("eye", key(a))
    assert rows[key(a)][1][5] == 0 and a.hidden
    assert p.scene.active_section() is a
    assert inspector(p, a)["Section"]["Mode"] == "Cut only"
    assert p.win.viewport.history.undo() and not a.hidden and a.active


def test_the_cut_goes_and_the_frame_stays_and_one_plane_cuts_at_a_time(project):
    p, a, b = project
    edit(p, a, "cut")
    assert p.scene.active_section() is None and not a.hidden
    edit(p, b, "cut")
    assert p.scene.active_section() is b
    edit(p, a, "cut")
    p.action("eye", key(a))
    assert a.active and not b.active and a.hidden
    edit(p, a, "cut")
    assert inspector(p, a)["Section"]["Mode"] == "Off"
    assert p.win.viewport.history.undo() and a.active and a.hidden


def test_the_fill_cell_moves_with_the_cut(project):
    p, a, b = project
    assert "Pattern" in inspector(p, a)["Section"]
    assert "Pattern" not in inspector(p, b)["Section"]
    edit(p, b, "cut")
    assert "Pattern" not in inspector(p, a)["Section"]
    assert "Pattern" in inspector(p, b)["Section"]


def test_a_plane_takes_its_own_pattern_colour_and_spacing(project):
    p, a, b = project
    edit(p, a, "pattern", "hatch")
    assert a.fill == {"pattern": "hatch"} and b.fill is None
    assert inspector(p, a)["Section"]["Pattern"] == "hatch"
    edit(p, a, "color", "#ff0000")
    assert a.fill == {"pattern": "hatch", "color": (1., 0., 0.)}
    assert p.win.viewport.history.undo() and a.fill == {"pattern": "hatch"}
    edit(p, a, "pattern", "inherit")
    assert a.fill is None and inspector(p, a)["Section"]["Pattern"] == "solid"


def test_the_hatch_spacing_is_typed_as_a_length(project):
    p, a, _ = project
    edit(p, a, "spacing", "8cm")
    assert a.fill["spacing"] == pytest.approx(.08)
    revision = p.win.viewport.history.revision
    edit(p, a, "spacing", "lots")
    assert p.win.viewport.history.revision == revision
    assert a.fill["spacing"] == pytest.approx(.08)


def test_a_plane_is_renamed_in_one_undo_step(project):
    p, a, _ = project
    uid = a.uid
    edit(p, a, "name", "Ground floor plan")
    assert p.rows()[key(a)][0] == "A — Ground floor plan" and a.uid == uid
    assert p.win.viewport.history.undo() and a.name == "Plan cut"
    edit(p, a, "symbol", "P1")
    assert p.rows()[key(a)][0] == "P1 — Plan cut"
    assert p.win.viewport.history.undo() and a.symbol == "A"


@pytest.fixture
def window_project(native_host):
    win = native_host.window
    p = Project(scene=win.viewport.scene)
    p.win = win
    a, b = _two_planes(p.scene)
    p.rows()
    p.action("toggle", "sections:all")
    return p, a, b


def menu_checks(p):
    view_commands.sync(p.view, p.win)
    cluster, menus = p.view.renderer.calls["ui_set_cluster"]
    return {item[0]: item[4] for group in list(cluster) + [rows for _, rows in menus] for item in group}


def folder(p):
    p.action("item", "sections:all")
    wgpu_ui._inspector(p.view, p.win)
    _, sections = p.view.renderer.calls["ui_set_inspector"]
    return {title: dict(entries) for title, entries in sections}


def test_the_folder_row_drives_the_view_menu_switches(window_project):
    p, _, _ = window_project
    assert folder(p)["Display"]["Cuts"] == "On"
    p.action("eye", "sections:all")
    assert not p.scene.show_section_planes and not menu_checks(p)["section_planes"]
    wgpu_ui._tree_action(p.win, p.view, "insp:sectionview:section_cuts")
    assert not p.scene.show_section_cuts and not menu_checks(p)["section_cuts"]
    assert folder(p)["Display"] == {"Frames": "Off", "Cuts": "Off", "Fill": "On"}
    assert view_commands.run(p.win, "section_planes")
    assert view_commands.run(p.win, "section_cuts")
    assert p.rows()["sections:all"][1][5] == 1
    assert folder(p)["Display"]["Cuts"] == "On"


def test_the_section_fill_switch_on_the_folder_and_the_cutting_plane(window_project):
    p, a, b = window_project
    action = "sectionview:section_fill"
    assert action in folder(p)["Actions"]
    wgpu_ui._tree_action(p.win, p.view, "insp:" + action)
    assert not p.scene.display_style.section_fill and not menu_checks(p)["section_fill"]
    assert folder(p)["Display"]["Fill"] == "Off"
    assert action in inspector(p, a)["Actions"]
    wgpu_ui._tree_action(p.win, p.view, "insp:" + action)
    assert p.scene.display_style.section_fill
    assert action not in inspector(p, b)["Actions"]
    assert view_commands.run(p.win, "section_fill")
    assert folder(p)["Display"]["Fill"] == "Off"
