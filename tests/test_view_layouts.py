"""Pane layouts, divider drag previews and document ownership in the native host."""
from types import SimpleNamespace

import pytest
from built_core import Vec3
from built_render import ModelViews, view_layouts

from tests.model_panes import grid  # noqa: F401
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def arrange(layout="four", **state):
    model = ModelViews()
    model.restore({"layout": layout, **state})
    cells = model.arrange(800, 600, [-1., -1., 1., 1.])
    return model, cells


def test_picker_uses_the_document_layout_and_cancel_leaves_it_alone(native_host, monkeypatch):
    from built_render import chrome_forms
    win = native_host.window
    assert win._doc.view_layout == "single" and win.view.peers() == []
    choices = {}
    def intercept(parent, title, field, options, current, done):
        choices.update(options=options, current=current, done=done)
    monkeypatch.setattr(chrome_forms, "ask_choice", intercept)
    win.set_view_layout("three")
    win._pick_view_layout()
    keys = list(choices["options"].values())
    assert keys == ["single", "four", "two", "three", "two_stacked", "three_mirror", "two_wide"]
    assert keys[choices["current"]] == "three"
    choices["done"](None)
    assert win._doc.view_layout == "three"
    choices["done"](next(k for k, v in choices["options"].items() if v == "two_wide"))
    assert win._doc.view_layout == "two_wide" and not win._is_dirty()


@pytest.mark.parametrize("layout", [row[0] for row in view_layouts()])
def test_model_area_shapes_and_camera_suggestions(layout):
    model, cells = arrange(layout)
    sizes = {idx: (x, y, w, h) for idx, x, y, w, h in cells}
    assert sum(w*h for x, y, w, h in sizes.values()) == 800*600
    if layout == "two_stacked":
        assert sizes[1] == (0, 0, 800, 300) and sizes[3] == (0, 300, 800, 300)
        assert model.initial_view(3) == "top"
    elif layout == "two_wide":
        assert sizes[1][0] == 0 and sizes[1][2] > 1.5 * sizes[0][2]
    elif layout in ("three", "three_mirror"):
        assert sizes[1][3] == 2 * sizes[0][3]
        assert (sizes[1][0] > sizes[0][0]) == (layout == "three_mirror")


@pytest.mark.parametrize(("start","end","cursor","expected"), [
    ((100,300),(100,205),"row-resize",[.5,205/600,.5]),
    ((400,100),(493,100),"col-resize",[493/800,.5,.5]),
    ((400,300),(520,220),"move",[.65,220/600,220/600]),
    ((400,300),(1,1),"move",[.1,.1,.1]),
    ((400,300),(798,599),"move",[.9,.9,.9]),
    ((400,300),(412,308),"move",[.5,.5,.5]),
])
def test_only_release_changes_shares_and_escape_discards_the_guide(start, end, cursor, expected):
    m, _ = arrange()
    assert m.divider("move", *start, False, 16) == cursor
    for cancel in (True, False):
        m.divider("press", *start, True, 16)
        m.divider("move", *end, False, 16)
        assert m.guides() and m.shares == [.5]*3
        if cancel:
            assert m.cancel_divider()
            assert m.shares == [.5]*3 and not m.guides()
        else:
            m.divider("release", *end, True, 16)
            assert m.shares == pytest.approx(expected) and not m.guides()


def test_crossing_guide_shows_full_proposed_split_and_snaps_separated_rows():
    m, _ = arrange(rows=[.3,.7])
    m.divider("press",400,180,True,16)
    m.divider("move",500,210,False,16)
    assert m.guides() == [[500,0,500,600],[0,210,500,210],[500,210,800,210]]
    m.divider("release",500,210,True,16)
    assert m.shares == pytest.approx([.625,.35,.35])


@pytest.mark.parametrize(("layout","tolerance","expected"), [
    ("four",16,.3), ("four",0,185/600), ("three_mirror",16,185/600)])
def test_row_snap_respects_the_other_visible_half_and_tolerance(layout,tolerance,expected):
    model, _ = arrange(layout, rows=[.6,.3])
    model.divider("press",50,360,True,tolerance)
    model.divider("release",50,185,True,tolerance)
    assert model.shares[1] == pytest.approx(expected)


def test_maximize_uses_hover_and_restores_the_original_dividers():
    m, cells = arrange(rows=[.3,.7], columns=.45)
    m.pointer("move",25,55,False)
    home = m.archive()
    m.toggle_maximized()
    assert m.arrange(800,600,[-1.,-1.,1.,1.]) == [(0,0,0,800,600)]
    assert m.divider("move",360,180,False,16) is None and m.archive() == home
    m.toggle_maximized()
    assert m.arrange(800,600,[-1.,-1.,1.,1.]) == cells
    for layout, cursor in [("two","col-resize"),("three","move"),("two_stacked","row-resize")]:
        m, _ = arrange(layout)
        assert m.divider("move",400,300,False,16) == cursor


@pytest.mark.parametrize("home",[None, {}, {"layout": 7}, {"layout":"gone"}, {"layout":"single"}])
def test_replacing_a_split_document_with_no_valid_layout_resets_it(home):
    model, _ = arrange()
    model.restore(home)
    assert model.layout == "single" and model.archive() is None


def test_archive_round_trip_preserves_both_row_fractions(native_host, tmp_path):
    from built_core.scene import Scene
    from formats import igz
    scene = Scene()
    scene.views_home = dict(layout="four", columns=.625, rows=[.25,.75])
    source = tmp_path/"split.igz"
    igz.save_scene(scene, source)
    assert native_host.open(source)
    win = native_host.window
    assert win._views_dict(win._doc) == scene.views_home and not win._is_dirty()
    win._maximize_model_view()
    target = tmp_path/"split.built"
    assert win.run_command("save_to:" + str(target))
    read = Scene()
    igz.load_into(read, target)
    assert read.views_home == scene.views_home
    read.clear()
    assert read.views_home is None


def test_layout_changes_preserve_peer_cameras_but_fit_only_visible_views(grid):
    main = grid.win._doc.viewport
    scene = main.scene
    far = scene.mesh.add_face([Vec3(100,100,0), Vec3(110,100,0), Vec3(110,110,0)])
    scene.version += 1
    peers = grid.panes()
    front = peers[grid.targets["Front"][0]].source_viewport
    top = peers[grid.targets["Top"][0]].source_viewport
    front.camera.set_view("back")
    original = front.camera.target.toTuple()
    grid.choose("two")
    scene.selection.add(far)
    grid.win._on_zoom_selection()
    assert main.camera.target.x() > 90 and top.camera.target.x() > 90
    assert front.camera.target.toTuple() == original
    grid.choose("four")
    assert "Back" in grid.targets and front is grid.panes()[grid.targets["Back"][0]].source_viewport
    grid.win._on_zoom_extents()
    assert front.camera.target.x() > 30


def test_extension_workspace_and_edit_state_stay_with_the_document(grid):
    from built_core.history import History
    from built_core.scene import Scene
    win = grid.win
    doc = win._doc
    main = doc.viewport
    main.scene.hide_similar_components = True
    before = main.scene
    for p in grid.panes().values():
        for key in ("_ext_snap_providers","_ext_pickables","_ext_overlays"):
            assert getattr(p.source_viewport,key) is getattr(main,key)
    scene = Scene()
    workspace = SimpleNamespace(scene=scene, history=History(scene), title=lambda:"test",
                                confirm_leave=lambda:True, is_dirty=lambda:False)
    assert win.enter_workspace(workspace)
    assert doc.view_layout == "single"
    grid.choose("four")
    assert all(p.source_viewport.scene is scene and p.source_viewport.history is workspace.history
               for p in grid.panes().values())
    assert win.leave_workspace()
    grid.frame()
    assert doc.view_layout == "four" and main.scene is before and before.hide_similar_components
    assert all(p.source_viewport.scene is before for p in grid.panes().values())


def test_a_drawing_tool_owns_a_crossing_while_a_divider_still_drags(grid):
    from built_render import MouseEvent
    host = grid.host
    doc = grid.win._doc
    grid.win._activate_tool("line")
    x, y = host.width // 2, host.height // 2
    for point, expect_divider in [((x,y),False), ((x,y//2),True)]:
        for kind, method in [("press","mousePressEvent"), ("release","mouseReleaseEvent")]:
            held = int(kind == "press")
            event = MouseEvent(kind, *point, 1, held, 0)
            target = doc.model_views.route(doc, kind, method, event, held)
            assert (target is None) == expect_divider


@pytest.mark.parametrize(("tolerance", "snaps"), [(16, False), (45, True)])
def test_native_divider_uses_the_saved_snap_distance(grid, tolerance, snaps):
    from built_render import MouseEvent
    from built_core.settings import settings

    settings().setValue("views/divider_snap_px", tolerance)
    grid.choose("single")
    grid.choose("four")
    doc = grid.win._doc
    x, middle = grid.host.width // 4, grid.host.height // 2
    for kind, method, y, held in [
        ("press", "mousePressEvent", middle, 1),
        ("move", "mouseMoveEvent", middle + 35, 1),
        ("release", "mouseReleaseEvent", middle + 35, 0),
    ]:
        button = 0 if kind == "move" else 1
        event = MouseEvent(kind, x, y, button, held, 0)
        assert doc.model_views.route(doc, kind, method, event, held) is None
    expected = middle if snaps else middle + 35
    assert doc.model_views.archive()["rows"][0] == pytest.approx(expected / grid.host.height)
