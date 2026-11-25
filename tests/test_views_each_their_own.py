"""Camera and display commands act on the selected pane of the native document."""
import pytest

from tests.model_panes import grid  # noqa: F401
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import view_commands


def mode(vp):
    return vp._effective_style().face_mode


def test_style_edge_switch_and_menu_checks_follow_clicks(grid):
    win = grid.win
    top = grid.click("Top")
    main = win._doc.viewport
    assert view_commands.run(win, "style_wireframe")
    assert mode(top) == "wireframe" and mode(main) != "wireframe"
    assert view_commands._snapshot(win)[2] == "Wireframe"
    assert view_commands.run(win, "back_edges")
    assert top.view_style.back_edges != main.scene.display_style.back_edges
    grid.click("Perspective")
    assert view_commands._snapshot(win)[2] != "Wireframe"
    assert view_commands.run(win, "style_xray")
    assert mode(main) == "xray"
    assert mode(top) == "wireframe"
    assert all(mode(p.source_viewport) != "xray" for p in grid.panes().values()
               if p.source_viewport is not main)
    grid.click("Top")
    assert view_commands.run(win, "xray") and mode(top) == "xray"
    assert view_commands.run(win, "xray") and mode(top) == "wireframe"
    assert mode(main) == "xray"


def test_projection_and_standard_shortcuts_change_the_clicked_pane(grid):
    from ingetrazo_core import Input as Qt
    win = grid.win
    top = grid.click("Top")
    main = win._doc.viewport
    original = main.camera.forward().toTuple()
    assert view_commands._snapshot(win)[1] == "parallel"
    grid.host.key(True, Qt.Key_P, Qt.ShiftModifier.value, "P")
    assert top.camera.perspective and view_commands._snapshot(win)[1] == "perspective"
    assert view_commands.run(win, "projection_parallel")
    assert not top.camera.perspective and main.camera.perspective
    front = grid.click("Front")
    grid.host.key(True, Qt.Key_1, Qt.ControlModifier.value, "1")
    assert front.camera.forward().toTuple() == pytest.approx(top.camera.forward().toTuple())
    assert main.camera.forward().toTuple() == original


def test_single_view_edits_the_model_style_and_projection(grid):
    grid.win.set_view_layout("single")
    assert grid.win._active_view() is grid.win._doc.viewport
    assert view_commands.run(grid.win, "style_wireframe")
    assert mode(grid.win.viewport) == grid.win.viewport.scene.display_style.face_mode == "wireframe"
    assert view_commands.run(grid.win, "projection_parallel")
    assert not grid.win.viewport.camera.perspective


def test_switching_projects_restores_each_panes_camera_and_style(grid):
    win = grid.win
    first = win._doc
    grid.click("Top")
    view_commands.run(win, "style_wireframe")
    view_commands.run(win, "projection_perspective")
    win.set_field_of_view(65)
    other = win.new_document()
    assert other.view_layout == "single" and mode(win.viewport) != "wireframe"
    win.activate_document(first)
    grid.frame()
    assert first.view_layout == "four" and mode(win._active_view()) == "wireframe"
    assert win._active_view().camera.fov_deg == pytest.approx(65)


def test_scene_capture_update_and_recall_use_the_panes_own_style(grid):
    win = grid.win
    top = grid.click("Top")
    view_commands.run(win, "style_wireframe")
    win.run_command("scene_add")
    saved = win.viewport.scene.saved_views[-1]
    assert saved.pitch == top.camera.pitch and saved.style["face_mode"] == "wireframe"
    main = win._doc.viewport
    original = main.camera.pitch, mode(main)
    front = grid.click("Front")
    win.tray.scenes.show_view(saved, animate=False)
    assert front.camera.pitch == saved.pitch and mode(front) == "wireframe"
    assert (main.camera.pitch, mode(main)) == original
    grid.click("Perspective")
    win.tray.scenes.update_view(saved)
    assert (saved.pitch, saved.style["face_mode"]) == original


def test_hover_moves_the_preview_but_export_and_style_keep_the_clicked_camera(grid):
    top = grid.click("Top")
    preview = grid.point("Front")
    assert preview is not top and grid.win._active_view() is top
    view_commands.run(grid.win, "style_wireframe")
    assert mode(top) == "wireframe" and mode(preview) != "wireframe"
