"""Native GPU tiles share geometry while keeping independent cameras and styles."""
from ingetrazo_core import Vec3
from tests.model_panes import grid  # noqa: F401
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_visible_tiles_share_one_resident_scene_and_gpu(grid):
    primary = grid.win.view
    assert len(grid.targets) == 4
    peers = primary.peers()
    assert len(peers) == 4
    uploads = len(grid.renderer.named("set_scene"))
    grid.frame()
    assert len(grid.renderer.named("set_scene")) == uploads
    for pane in peers:
        assert pane.packed is primary.packed and pane.uid_rows is primary.uid_rows
        assert pane.renderer.raw is grid.renderer
        assert pane.source_viewport.history is grid.win.viewport.history


def test_each_tile_draws_its_own_camera_and_style(grid):
    from ingetrazo_core.style import style_by_name
    vp = grid.click("Top")
    vp.view_style = style_by_name("X-ray").copy()
    grid.renderer.calls.clear()
    for pane in grid.panes().values():
        pane._last_camera = None
        pane._frame_style = None
    grid.frame()
    cameras = {target: tuple(args[0]) for target, _, args in grid.renderer.named("set_camera")}
    assert len(set(cameras.values())) == 4
    modes = {target: args[0]["face_mode"] for target, _, args in grid.renderer.named("set_style")}
    assert {k for k, mode in modes.items() if mode == "xray"} == {vp.keys_stand_in._target}
    assert {r[0] for r in grid.renderer.named("render")} == set(cameras)


def test_moving_a_tool_between_panes_moves_its_preview(grid):
    grid.win._activate_tool("line")
    tool = grid.win.viewport.active_tool
    for name in ("Top", "Right"):
        vp = grid.point(name)
        grid.frame()
        assert grid.win.viewport is vp and vp.active_tool is tool
        panes = list(grid.panes().values())
        assert [p for p in panes if p.source_viewport.tool_preview] == [vp.keys_stand_in]
        assert vp.keys_stand_in.tools.tool is tool
        assert all(p._preview_given_away for p in panes if p is not vp.keys_stand_in)


def test_maximized_pane_draws_model_changes_and_releases_hidden_targets(grid):
    vp = grid.click("Top")
    grid.win._maximize_model_view()
    grid.frame()
    assert list(grid.targets) == ["Top"] and len(grid.renderer.alive) == 2
    vp.scene.mesh.add_face([Vec3(0, 0, 0), Vec3(2, 0, 0), Vec3(2, 2, 0)])
    vp.scene.version += 1
    vp.notify_scene_changed()
    grid.frame()
    assert not grid.win.view._scene_dirty
    grid.win._maximize_model_view()
    grid.frame()
    assert len(grid.renderer.alive) == 5
    grid.win.new_document()
    assert grid.renderer.alive == {0}
