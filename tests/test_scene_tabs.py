"""Saved scenes in the native Scene chooser, per document, through undo and reopen."""
import pytest

from tests.scene_controls import choose, prompt, rows
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views import wgpu_ui


@pytest.fixture
def win(native_host):
    return native_host.window


def labels(win):
    return [row[1] for row in rows(win)]


def checked(win):
    return [row[1] for row in rows(win) if row[4] == 1]


def add(win, name, camera="iso"):
    win.viewport.camera.set_view(camera)
    assert win.run_command("scene_add")
    saved = win.viewport.scene.saved_views[-1]
    assert win.tray.scenes.rename_view(saved, name)
    return saved


def test_first_scene_last_scene_and_undo_follow_the_chooser(win):
    assert labels(win) == []
    wgpu_ui._run(win, ["tab:scene:add"])
    first, = labels(win)
    assert checked(win) == [first]
    win._on_undo()
    assert labels(win) == []
    win._on_redo()
    assert labels(win) == [first]
    choose(win, 0)
    wgpu_ui._run(win, ["tab:scene:add"])
    assert len(labels(win)) == 2
    win._on_undo()
    assert labels(win) == [first]
    choose(win, 0)
    prompt(win, "scene_delete", "yes")
    assert labels(win) == []
    win._on_undo()
    assert labels(win) == [first]


def test_switching_documents_restores_the_shown_scene_without_a_recall(win):
    a = win._doc
    add(win, "Roof", "top")
    add(win, "Lobby", "front")
    choose(win, 0)
    assert checked(win) == ["Roof"]
    assert win.viewport.camera.pitch == win.viewport.scene.saved_views[0].pitch
    win.viewport.camera.distance += 11
    distance = win.viewport.camera.distance
    b = win.new_document()
    assert labels(win) == []
    add(win, "Section")
    win.activate_document(a)
    assert labels(win) == ["Roof", "Lobby"] and checked(win) == ["Roof"]
    assert win.viewport.camera.distance == distance
    win.activate_document(b)
    assert labels(win) == checked(win) == ["Section"]


def test_reorder_keeps_camera_and_mark_and_saves_in_one_undo_step(win, tmp_path):
    from formats import igz
    for name in ("A", "B", "C"):
        add(win, name)
    choose(win, 1)
    camera = win.viewport.camera
    camera.set_view("right")
    pose = camera.yaw, camera.pitch, camera.distance
    count = len(win.viewport.history.undo_stack)
    wgpu_ui._run(win, ["tab:scene:up"])
    assert labels(win) == ["B", "A", "C"] and checked(win) == ["B"]
    assert (camera.yaw, camera.pitch, camera.distance) == pose
    assert len(win.viewport.history.undo_stack) == count + 1
    win._on_undo()
    assert labels(win) == ["A", "B", "C"]
    win._on_redo()
    wgpu_ui._run(win, ["tab:scene:down", "tab:scene:down"])
    assert labels(win) == ["A", "C", "B"] and checked(win) == ["B"]
    path = tmp_path / "ordered.built"
    igz.save_scene(win.viewport.scene, path)
    assert win.open_path(path)
    assert labels(win) == ["A", "C", "B"]


def test_rename_tracks_the_mark_and_sheet_reference_through_undo(win):
    from built_core.composition import Composicion
    add(win, "Site")
    plan = add(win, "Plan")
    page = Composicion(name="Sheet")
    frame = page.default_frame()
    frame.view_key = "scene:Plan"
    page.frames.append(frame)
    win.viewport.scene.compositions.append(page)
    history = win.viewport.history
    for decision, text in (("no", "Other"), ("yes", " Site "), ("yes", "Plan"), ("yes", " ")):
        before = len(history.undo_stack)
        prompt(win, "scene_rename", decision, text)
        assert len(history.undo_stack) == before and plan.name == "Plan"
    payload = prompt(win, "scene_rename", "yes", "  Ground floor ")
    assert "Plan" in payload[1]
    assert checked(win) == ["Ground floor"] and frame.view_key == "scene:Ground floor"
    assert len(history.undo_stack) == before + 1
    win._on_undo()
    assert checked(win) == ["Plan"] and frame.view_key == "scene:Plan"
    win._on_redo()
    assert checked(win) == ["Ground floor"]


def test_chooser_updates_the_marked_scene_and_recall_uses_its_new_camera(win):
    a = add(win, "Planta", "top")
    cam = win.viewport.camera
    cam.set_view("front")
    wgpu_ui._run(win, ["tab:scene:update"])
    front = cam.pitch
    cam.set_view("iso")
    choose(win, 0)
    assert a.pitch == cam.pitch == front and checked(win) == ["Planta"]


def test_an_old_menu_row_can_only_recall_the_document_scene_it_displayed(win):
    a = add(win, "A", "top")
    b = add(win, "B", "front")
    rows(win)
    win.tray.scenes.move_view(a, 1)
    wgpu_ui._run(win, ["tab:scene:0"])
    assert win._scene_tabs.marked_view() is a
    win.tray.scenes.delete_views([a])
    cam = win.viewport.camera
    cam.set_view("right")
    before = (cam.pitch, cam.yaw)
    wgpu_ui._run(win, ["tab:scene:0"])
    assert win._scene_tabs.marked_view() is None and (cam.pitch, cam.yaw) == before
    assert labels(win) == [b.name]
