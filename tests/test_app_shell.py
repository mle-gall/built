"""The Rust app shell's Python half (app_shell/), headless: no surface, so
no renderer is made; the document, tools and key routing run as in the
window. The windowed path is exercised by ``built --script`` (see
docs/rust-app-shell.md)."""
from pathlib import Path

import pytest
from PySide6.QtCore import Qt
from tests.shell_isolation import restore_shell_patches  # noqa: F401  (autouse)

FIXTURE = Path(__file__).parent / "fixtures" / "shell" / "slab-and-cube.igz"


@pytest.fixture
def host(tmp_path):
    from app_shell.host import Host
    h = Host(0, 1280, 800, 2.0, settings=str(tmp_path))
    yield h
    h.close()


def test_opens_a_document_headless(host):
    assert host.open(FIXTURE)
    state = host.state()
    assert (state["faces"], state["edges"]) == (6, 16)
    assert state["errors"] == 0
    assert host.view.renderer is None          # nothing presents headless


def test_borrowed_classes_are_no_widgets(host):
    from PySide6.QtWidgets import QWidget
    assert not isinstance(host.viewport, QWidget)
    assert not isinstance(host.view, QWidget)
    # Viewport's own state came through its borrowed __init__.
    assert host.viewport.history is not None and host.viewport.scene is not None
    assert host.view.source_viewport is host.viewport


def test_tool_shortcut_goes_through_shortcut_override(host):
    host.open(FIXTURE)
    host.window._activate_tool("select")
    host.key(True, Qt.Key_L.value, 0, "l")
    host.key(False, Qt.Key_L.value, 0, "l")
    assert type(host.viewport.active_tool).__name__ == "LineTool"
    assert host.window._tool_actions["line"].isChecked()
    assert host.errors == []


def test_tool_shortcut_restored_in_new_shell(host, tmp_path):
    from PySide6.QtGui import QKeySequence
    from app_shell.host import Host
    from views.shortcuts import save_shortcut

    action = host.window._tool_actions["line"]
    save_shortcut(action, [QKeySequence("F11")])
    host.close()
    reopened = Host(0, 1280, 800, 2.0, settings=str(tmp_path))
    try:
        action = reopened.window._tool_actions["line"]
        assert action.shortcut() == QKeySequence("F11")
        reopened.key(True, Qt.Key_F11.value, 0, "")
        assert type(reopened.viewport.active_tool).__name__ == "LineTool"
        assert not reopened.errors
    finally:
        reopened.close()


def test_undo_from_the_menu(host):
    host.open(FIXTURE)
    from ingetrazo_core import Vec3 as QVector3D
    from core.history import AddFaceCommand
    vp = host.viewport
    before = len(vp.scene.mesh.faces)
    loop = [QVector3D(5, 0, 0), QVector3D(6, 0, 0), QVector3D(6, 1, 0)]
    vp.history.execute(AddFaceCommand(loop))
    assert len(vp.scene.mesh.faces) == before + 1
    host.menu("undo")
    assert len(vp.scene.mesh.faces) == before
    assert host.errors == []


def test_events_answer_the_qt_calls_the_handlers_make():
    from ingetrazo_render import KeyEvent, MouseEvent, NativeGestureEvent, WheelEvent
    m = MouseEvent("press", 10.5, 20, Qt.LeftButton.value, Qt.LeftButton.value,
                   (Qt.ShiftModifier | Qt.ControlModifier).value)
    assert m.position().x() == 10.5 and m.button() == Qt.LeftButton
    assert m.modifiers() & Qt.ControlModifier and m.buttons() & Qt.LeftButton
    k = KeyEvent(True, Qt.Key_Escape.value)
    assert k.key() == Qt.Key_Escape and not k.isAutoRepeat()
    w = WheelEvent(1, 2, 0, 0, 0, 240)
    assert w.angleDelta().y() == 240 and w.pixelDelta().isNull()
    assert w.device().type().name == "Mouse"
    slide = WheelEvent(1, 2, 0, -30, 0, -60, phase="move", inverted=True)
    from ingetrazo_render import trackpad_nav
    assert trackpad_nav.is_slide(slide)
    g = NativeGestureEvent("zoom", 0.1, 3, 4)
    assert g.gestureType() == Qt.ZoomNativeGesture and g.value() == pytest.approx(0.1)


def test_window_input_answers_from_winit(host):
    from views import input_state
    from PySide6.QtCore import QPoint
    host.pointer("move", 30, 40, 0, 0, Qt.ControlModifier.value)
    assert input_state.modifiers(host.viewport, query=True) & Qt.ControlModifier
    assert input_state.cursor_position(host.viewport) == QPoint(30, 40)
    host.key(True, Qt.Key_Shift.value, Qt.ShiftModifier.value)
    assert input_state.modifiers(host.viewport) == Qt.ShiftModifier


def test_rail_matches_classic_groups_and_dispatches(host):
    from unittest.mock import Mock
    from ingetrazo_render import wgpu_library
    from views import wgpu_ui

    host.open(FIXTURE)
    win, view = host.window, host.view
    wgpu_library.install(win)
    view.renderer = Mock()
    try:
        wgpu_ui._send_tools(view, win)
        groups, = view.renderer.ui_set_tools.call_args.args
        from ingetrazo_render import rail_layout
        expected = [[key for key, _ in row] for row in rail_layout()]
        actions = {key for row in rail_layout() for key, binding in row if binding}
        assert [[entry[0] for entry in group] for group in groups] == expected
        assert set(view._ui_palette_runs) == set(actions)
    finally:
        view.renderer = None
    for group in expected:
        for key in group:
            if key not in actions:
                win._tool_actions[key].trigger()
                assert host.viewport.active_tool is win._tools[key]
    view._ui_palette_runs["eyedropper"]()
    assert win._act_eyedropper.isChecked()
    assert host.viewport.active_tool is win._tools["paint"]
    win.release_eyedropper()
    for key in ("bridge", "make_faces"):
        view._ui_palette_runs[key]()
    assert not host.errors


def test_text3d_place_edit_font_and_undo(host):
    import json
    from unittest.mock import Mock
    from ingetrazo_core import Vec3
    from views import wgpu_ui

    host.open(FIXTURE)
    win, view, vp = host.window, host.view, host.viewport
    view.renderer = Mock()
    view._ui_attached = True

    def apply(family):
        draft = json.loads(view.renderer.ui_font_picker.call_args.args[0])
        draft.update(text="Ab", font=family)
        wgpu_ui._run(win, ["font:" + json.dumps(draft)])

    try:
        win._act_3dtext.trigger()
        apply("Helvetica")
        vp.active_tool.place_at(vp, Vec3(1, 2, 0))
        group = vp.scene.groups[-1]
        assert group.text3d["font"] == "Helvetica"
        assert group.children
        win._on_edit_3d_text(group)
        apply("Georgia")
        assert group.text3d["font"] == "Georgia"
        host.menu("undo")
        assert group.text3d["font"] == "Helvetica"
        host.menu("undo")
        assert not vp.scene.groups
    finally:
        view._ui_attached = False
        view.renderer = None


@pytest.mark.parametrize("tool", ["text", "dimension"])
def test_annotation_font_options(host, tool):
    import json
    from unittest.mock import Mock
    from views import wgpu_ui
    host.open(FIXTURE)
    view, win = host.view, host.window
    win._activate_tool(tool)
    view.renderer = Mock()
    view._ui_attached = True
    try:
        run = next(row[-1] for row in wgpu_ui._options(view) if row[0] == "font")
        run()
        draft = json.loads(view.renderer.ui_font_picker.call_args.args[0])
        assert draft["unit"].strip() == "pt"
        draft.update(font="Georgia", height=12., italic=True)
        wgpu_ui._run(win, ["font:" + json.dumps(draft)])
        style = win.viewport.scene.dimension_style
        assert (style["font_family"], style["font_size"], style["font_italic"]) == ("Georgia", 12, True)
        run()
        wgpu_ui._run(win, ["font:"])
        assert style["font_family"] == "Georgia"
    finally:
        view._ui_attached = False
        view.renderer = None


def test_registered_shortcuts_use_rust_and_plugin_actions_still_trigger(host, monkeypatch):
    from PySide6.QtGui import QAction, QKeySequence
    from ingetrazo_render import ShellAction

    win = host.window
    assert all(isinstance(a, ShellAction) for a in win._registry_actions.values())
    assert not win.findChildren(QAction)
    line = win._registry_actions["tool:line"]
    line.setShortcuts([QKeySequence("F12"), QKeySequence("Ctrl+X, Ctrl+C")])
    seen = []
    line.run = lambda: seen.append("registered")
    assert host._shortcut(Qt.Key_F12, 0)
    assert not host._shortcut(Qt.Key_X, Qt.ControlModifier.value)
    line.setEnabled(False)
    plugin = QAction("Plugin fixture", win)
    plugin.setShortcut(QKeySequence("F12"))
    plugin.triggered.connect(lambda: seen.append("plugin"))
    assert host._shortcut(Qt.Key_F12, 0)
    # Once observed, a raw extension action dispatches without walking the
    # QObject tree. Observe later shortcut edits, disabling and reparenting.
    with monkeypatch.context() as m:
        m.setattr(win, "findChildren", lambda *_: pytest.fail("rescanned stable plugin actions"))
        assert host._shortcut(Qt.Key_F12, 0)
    plugin.setShortcut(QKeySequence("F10"))
    assert not host._shortcut(Qt.Key_F12, 0)
    assert host._shortcut(Qt.Key_F10, 0)
    plugin.setEnabled(False)
    assert not host._shortcut(Qt.Key_F10, 0)
    from PySide6.QtCore import QObject
    container = QObject(win)
    assert not host._shortcut(Qt.Key_F10, 0)
    plugin.setParent(container)
    plugin.setEnabled(True)
    assert host._shortcut(Qt.Key_F10, 0)
    container.setParent(None)
    assert not host._shortcut(Qt.Key_F10, 0)
    assert seen == ["registered"] + ["plugin"] * 4


def test_winit_tool_cursors_use_the_same_art_for_tools_and_navigation(host, monkeypatch):
    from core import general_settings
    from views import icons
    from ingetrazo_render import tool_cursor_art

    monkeypatch.setattr(icons, "tool_cursor", lambda *a: pytest.fail("Requested a Qt tool bitmap"))
    host.window._activate_tool("line")
    host.view._mirror_cursor()
    assert host._cursor == "tool:line"
    art, hotspot = tool_cursor_art("line", 32)
    assert len(art) == 32 * 32 * 4 and tuple(hotspot) == (6, 42)
    host.window._activate_tool("tape")
    assert host._cursor == "tool:tape+"
    # Both modifier states have distinct images and the same working point.
    plain, hot = tool_cursor_art("tape", 32)
    plus, other = tool_cursor_art("tape+", 32)
    assert plain != plus and hot == other
    host.viewport.nav_mode = "orbit"
    host.viewport._apply_nav_cursor()
    assert host._cursor == "tool:orbit"
    general_settings.set_value("cursor_style", "crosshair")
    host.viewport._apply_nav_cursor()
    assert host._cursor == "crosshair"
    general_settings.set_value("cursor_style", "arrow")
    host.window._activate_tool("select")
    assert host._cursor == "default"


def test_model_menu_uses_current_selection_history_and_tool(host):
    assert host.open(FIXTURE)
    win, scene = host.window, host.viewport.scene
    edge = next(iter(scene.mesh.edges))
    scene.selection.add(edge)
    win._registry_actions["menu:hide"].trigger()
    assert edge.hidden
    host.menu("unhide_last")
    assert not edge.hidden
    host.menu("undo")
    assert edge.hidden
    host.menu("unhide_all")
    assert not edge.hidden
    host.menu("tool:freehand")
    assert host.viewport.active_tool is win._tools["freehand"]
    host.menu("tool:select")
    host.menu("repeat_last_command")
    assert host.viewport.active_tool is win._tools["freehand"]
    host.menu("projection_parallel")
    assert host.viewport.camera.projection_mode() == "parallel"
    assert host.errors == []


@pytest.mark.parametrize("suffix", ["obj", "stl"])
def test_mesh_import_menu_waits_for_answer_then_imports_once(host, tmp_path, monkeypatch, suffix):
    from core.settings import settings
    from tests.test_import_failure_reported import _CUBE_OBJ, _GOOD_STL
    from views.filedialogs import file_dialogs
    path = tmp_path / ("triangle." + suffix)
    path.write_bytes(_CUBE_OBJ if suffix == "obj" else _GOOD_STL)
    monkeypatch.setattr(file_dialogs, "getOpenFileName", lambda *a: (str(path), ""))
    forms = []
    monkeypatch.setattr("ingetrazo_render.chrome_forms.show", lambda _, form: forms.append(form))
    win = host.window
    store = settings()
    store.setValue("import/stl_simplify_mode", "all")
    command = "import:" + suffix
    before = win.viewport.scene.content_version
    host.menu(command)
    form = forms.pop()
    form.receive("unit", "3")
    form.press("cancel")
    assert win.viewport.scene.content_version == before
    assert store.value("import/stl_simplify_mode") == "all"
    win._registry_actions["menu:" + command].trigger()
    form = forms.pop()
    assert form.values()["unit"] == ("m" if suffix == "obj" else "mm")
    form.receive("unit", "1")
    if suffix == "stl":
        assert form.values()["simplify"] == "all"
        form.receive("simplify", "0")
    form.press("ok")
    assert win.viewport.history.undo_stack and win._import_name == path.name
    assert "Imported" in win.statusBar().currentMessage()
    if suffix == "stl":
        assert store.value("import/stl_unit") == "cm"
        assert store.value("import/stl_simplify_mode") == "none"
    host.menu("undo")
    assert not win.viewport.scene.groups and not win.viewport.scene.mesh.faces
    assert host.errors == []


@pytest.mark.parametrize("choice", ["cancel", "ok"])
def test_open_unitless_mesh_preserves_original_tab_on_cancel_or_failure(host, tmp_path, monkeypatch, choice):
    assert host.open(FIXTURE)
    win = host.window
    original = win.active_document()
    path = tmp_path / "broken.stl"
    path.write_bytes(b"invalid STL fixture")
    forms, failures = [], []
    monkeypatch.setattr("ingetrazo_render.chrome_forms.show", lambda _, f: forms.append(f) or "chrome")
    monkeypatch.setattr("ingetrazo_render.chrome_dialogs.critical", lambda _, title, text: failures.append(title))
    assert host.open(path)
    assert win.documents() == [original]
    forms.pop().press(choice)
    assert win.documents() == [original] and win.active_document() is original
    assert failures == ([] if choice == "cancel" else ["Import STL failed"])
    assert host.errors == []


def test_scene_menu_acts_on_marked_scene_and_export_uses_chosen_format(host, monkeypatch):
    import ingetrazo_render as wgpu_deliveries
    assert host.open(FIXTURE)
    win = host.window
    scene = host.viewport.scene
    before = len(scene.saved_views)
    host.menu("scene_add")
    original = win._scene_tabs.marked_view()
    host.menu("scene_duplicate")
    copy = win._scene_tabs.marked_view()
    assert copy is not original and len(scene.saved_views) == before + 2
    host.menu("scene_show")
    assert win._scene_tabs.marked_view() is copy
    formats = []
    monkeypatch.setattr(wgpu_deliveries, "delivery_export_form",
                        lambda _w, key, one_off, fmt: formats.append((key, one_off, fmt)))
    for fmt in ("ifc", "glb", "dae", "stl", "obj", "dxf"):
        win._registry_actions["menu:export:" + fmt].trigger()
        assert formats[-1] == (None, True, fmt)
    assert host.errors == []


def test_native_scene_playback_stops_on_input_and_switches_documents_safely(host):
    from core.scene_animation import SceneAnimation

    assert host.open(FIXTURE)
    win, viewport = host.window, host.viewport
    host.menu("scene_add")
    viewport.camera.set_view("top")
    host.menu("scene_add")
    scene = viewport.scene
    scene.animation = SceneAnimation(transition=0.1, pause=0.1)
    clock = [100.]
    win.scene_player._clock = lambda: clock[0]
    host.menu("scene_play")
    assert win.scene_player.is_playing()
    clock[0] += .15
    win.scene_player.tick()
    assert win._scene_tabs.marked_view() is scene.saved_views[0]
    host.key(True, Qt.Key_Escape)
    assert not win.scene_player.is_active()
    host.menu("scene_previous")
    assert win._scene_tabs.marked_view() is scene.saved_views[-1]
    host.menu("scene_transitions")
    assert not scene.animation.enabled
    host.menu("scene_play")
    original = win.active_document()
    win.new_document()
    assert not win.scene_player.is_active()
    assert host.viewport.scene is not original.viewport.scene
    assert host.errors == []


def test_native_edit_context_display_and_preference(host):
    assert host.open(FIXTURE)
    win, vp = host.window, host.viewport
    host.menu("rest_normal")
    assert vp.edit_rest_mode == "normal"
    host.menu("hide_rest")
    assert vp.edit_rest_mode == "hide"
    host.menu("hide_rest")
    assert vp.edit_rest_mode == "normal"
    host.menu("rest_fade")
    host.menu("hide_similar")
    assert vp.edit_rest_mode == "fade" and vp.scene.hide_similar_components
    win._registry_actions["menu:hide_similar"].trigger()
    assert not vp.scene.hide_similar_components and not host.errors
