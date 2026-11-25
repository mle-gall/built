# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The Layers tab and what came with it: active layer, colour and look per
layer, folders, merge, section planes on layers, layer lines on sheets and
in DXF, templates."""
from __future__ import annotations

import ingetrazo_core
from core import layers as L
from core.group import Group
from core.layers import DEFAULT_LAYER, Layer, assign_layer
from core.mesh import Mesh
from ingetrazo_core.saved_views import SavedView
from tests.shell_isolation import native_question_host, restore_shell_patches  # noqa: F401
from tests.test_drawing_records import _square


# ---- The panel -------------------------------------------------------------

def test_add_opens_the_selected_folder_and_preserves_its_parent(native_host):
    from ingetrazo_core import edit_tag_folder
    from tests.native_tree import action
    import json
    scene = native_host.viewport.scene
    action(native_host, "newtag", "")
    assert scene.layers[-1].folder is None
    folder = edit_tag_folder(native_host.viewport.history, "new", json.dumps({"name": "Survey"}))
    inner = edit_tag_folder(native_host.viewport.history, "new", json.dumps({"name": "Roads", "parent": folder}))
    action(native_host, "newtag", "preview-folder:" + inner)
    assert scene.layers[-1].folder == inner and L.folder_parent(scene, inner) == folder
    assert {"preview-folder:" + inner, "preview-folder:" + folder} <= native_host.view._ui_exp


def test_purge_asks_for_the_count_and_accepts_only_a_current_confirmation(native_question_host):
    from tests.native_tree import action
    from ingetrazo_render import chrome_dialogs
    host = native_question_host
    scene = host.viewport.scene
    scene.layers.extend([Layer("Vacia 1"), Layer("Vacia 2")])
    for answer in ("cancel", "stale", "readonly", "delete"):
        action(host, "purgetags", "preview-tag:Vacia 1")
        assert "2" in chrome_dialogs.current_sheet(host.window).text
        if answer == "stale":
            scene.version += 1
        scene.worksession_read_only = answer == "readonly"
        assert chrome_dialogs.answer(host.window, "cancel" if answer == "cancel" else "delete")
        assert len(scene.layers) == (1 if answer == "delete" else 3)
        scene.worksession_read_only = False
    action(host, "purgetags", "")
    assert chrome_dialogs.current_sheet(host.window) is None


def test_the_eye_and_padlock_headers_act_on_every_layer(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B", visible=False)])
    for verb, field, expected in [("headereye", "visible", True), ("headereye", "visible", False),
                                  ("headerlock", "locked", True), ("headerlock", "locked", False)]:
        action(native_host, verb, "tags")
        assert all(getattr(tag, field) == expected for tag in scene.layers)
    assert len(native_host.viewport.history.undo_stack) == 4
    assert native_host.viewport.history.undo() and all(tag.locked for tag in scene.layers)


def test_a_rows_target_isolates_and_a_second_click_brings_back_what_it_hid(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B"), Layer("C"), Layer("Off", visible=False)])
    face = _square(scene.mesh)
    assign_layer(face, "B")
    action(native_host, "tagisolate", "preview-tag:A")
    assert [ly.key for ly in scene.layers if ly.visible] == [DEFAULT_LAYER, "A"]
    assert not scene.entity_visible(face)
    action(native_host, "tagisolate", "preview-tag:B")
    assert scene.entity_visible(face) and not scene.layer("A").visible
    action(native_host, "tagisolate", "preview-tag:B")
    assert all(scene.layer(k).visible for k in ("A", "B", "C"))
    assert not scene.layer("Off").visible


def test_the_target_acts_on_every_selected_row_and_on_folders(native_host):
    import json
    from tests.native_tree import action
    scene = native_host.viewport.scene
    folder = ingetrazo_core.tag_edits.add_folder(scene, "Arq")
    inner = ingetrazo_core.tag_edits.add_folder(scene, "Inner", parent=folder)
    scene.layers.extend([Layer("A"), Layer("B"), Layer("C", folder=folder), Layer("D", folder=inner)])
    def isolate(*rows):
        action(native_host, "isolatetags", json.dumps(rows))
    isolate("preview-tag:A", "preview-tag:B")
    assert {ly.key for ly in scene.layers if ly.visible} == {DEFAULT_LAYER, "A", "B"}
    isolate("preview-folder:" + folder)
    assert {ly.key for ly in scene.layers if ly.visible} == {DEFAULT_LAYER, "C", "D"}
    assert native_host.viewport.history.undo()
    assert scene.layer("A").visible and not scene.layer("D").visible
    assert native_host.viewport.history.redo()
    isolate("preview-folder:" + folder)
    assert all(ly.visible for ly in scene.layers)


def test_the_target_header_ends_the_isolation(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B")])
    action(native_host, "tagisolate", "preview-tag:A")
    assert not scene.layer("B").visible
    action(native_host, "isolatetags", "[]")
    assert scene.layer("B").visible


def test_merge_tag_moves_carriers_in_one_step_and_rejects_a_replaced_target(native_host, monkeypatch):
    from tests.native_tree import action
    from ingetrazo_render import chrome_forms
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B")])
    face = _square(scene.mesh)
    assign_layer(face, "A")
    callbacks = []
    monkeypatch.setattr(chrome_forms, "ask_choice", lambda parent, title, label, items, at, done: callbacks.append(done))
    action(native_host, "mergetag", "preview-tag:A")
    callbacks.pop()("B")
    assert scene.layer("A") is None and L.layer_of(face) == "B"
    assert native_host.viewport.history.undo()
    assert L.layer_of(face) == "A"
    action(native_host, "mergetag", "preview-tag:A")
    scene.layers.remove(scene.layer("B"))
    scene.layers.append(Layer("B"))
    callbacks.pop()("B")
    assert scene.layer("A") is not None and L.layer_of(face) == "A"


def test_expand_and_collapse_all_reach_nested_tag_folders():
    from tests.test_project_drawing_filters import Project
    project = Project()
    outer = ingetrazo_core.tag_edits.add_folder(project.scene, "A")
    inner = ingetrazo_core.tag_edits.add_folder(project.scene, "B", parent=outer)
    project.scene.layers.append(Layer("Walls", folder=inner))
    project.action("lens", "1")
    expanded = project.action("expandall", "")
    assert expanded["preview-folder:" + outer][1][8] == expanded["preview-folder:" + inner][1][8] == 1
    assert "preview-tag:Walls" in expanded
    folded = project.action("collapseall", "")
    assert not folded["preview-folder:" + outer][1][8] and "preview-tag:Walls" not in folded


def test_drawing_quick_filters_and_inversion_follow_document_use_and_selection():
    from tests.test_project_drawing_filters import Project
    p = Project(Layer("Usada"), Layer("Oculta", visible=False),
                Layer("Fija", locked=True), Layer("Vacia", printable=False))
    p.view._ui_lens = 1
    group = Group(Mesh(), name="g")
    group.layer = "Usada"
    p.scene.groups.append(group)
    all_tags = {"Untagged", "Usada", "Oculta", "Fija", "Vacia"}
    for query, expected in [("", all_tags), ("is:visible", all_tags - {"Oculta"}),
                            ("is:hidden", {"Oculta"}), ("is:locked", {"Fija"}),
                            ("is:used", {"Untagged", "Usada"}), ("is:unused", {"Fija", "Oculta", "Vacia"}),
                            ("is:unprinted", {"Vacia"}), ("is:selected", set()),
                            ("! is:hidden", all_tags - {"Oculta"}), ("! is:visible is:locked", all_tags - {"Fija"})]:
        assert set(p.names(query, "preview-tag:")) == expected
    p.scene.select([group])
    assert p.names("is:selected", "preview-tag:") == ["Usada"]
    p.scene.saved_views.append(SavedView("Plan", hidden_layers=["Vacia"]))
    p.scene.version += 1
    assert set(p.names("is:unused", "preview-tag:")) == {"Oculta", "Fija"}
    assert set(p.names("", "preview-tag:")) == all_tags


def test_saved_scene_preference_and_make_active(native_host):
    from tests.native_tree import action
    from core.settings import settings
    scene = native_host.viewport.scene
    scene.saved_views.append(SavedView("Planta"))
    action(native_host, "newtaghidden", "1")
    action(native_host, "newtag", "")
    key = scene.layers[-1].key
    assert settings().value("layers/new_hidden_in_scenes", False, type=bool)
    assert list(scene.saved_views[0].hidden_layers) == [key]
    scene.layers[-1].locked = True
    action(native_host, "tagactive", "preview-tag:" + key)
    assert L.active_layer(scene) == key and not scene.layers[-1].locked
    action(native_host, "newtaghidden", "0")
    action(native_host, "newtag", "")
    assert list(scene.saved_views[0].hidden_layers) == [key]


def test_drawing_model_colours_belong_to_the_style(native_host):
    from core.style import Style
    from tests.native_tree import action
    scene = native_host.viewport.scene
    for on in (True, False):
        action(native_host, "tagcolours", str(int(on)))
        assert Style.from_dict(scene.display_style.to_dict()).color_by_layer == on
    scene.worksession_read_only = True
    action(native_host, "tagcolours", "1")
    assert not scene.display_style.color_by_layer


def test_tag_menu_changes_saved_scene_visibility_and_restores_it(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B")])
    first, second = SavedView("Plan", hidden_layers=["B"]), SavedView("Roof")
    scene.saved_views.extend([first, second])
    action(native_host, "scenetaghide", "preview-tag:A")
    assert list(first.hidden_layers) == ["B", "A"] and list(second.hidden_layers) == ["A"]
    action(native_host, "scenetagshow", "preview-tag:A")
    assert list(first.hidden_layers) == ["B"] and not second.hidden_layers
    assert native_host.viewport.history.undo() and list(second.hidden_layers) == ["A"]
    assert native_host.viewport.history.undo() and list(first.hidden_layers) == ["B"]


def test_template_export_and_import_in_drawing(native_host, tmp_path, monkeypatch):
    from tests.native_tree import action
    from ingetrazo_render.shell_files import ShellFileDialog
    scene = native_host.viewport.scene
    folder = ingetrazo_core.tag_edits.add_folder(scene, "Oficina")
    tag = Layer("Ejes", folder=folder, linetype="dashdot")
    scene.layers.append(tag)
    path = tmp_path / "plantilla.json"
    for picker in ("getSaveFileName", "getOpenFileName"):
        monkeypatch.setattr(ShellFileDialog, picker, lambda *a: (str(path), ""))
    action(native_host, "exporttags", "")
    scene.layers[:] = [Layer(DEFAULT_LAYER)]
    action(native_host, "importtags", "")
    restored = scene.layer("Ejes")
    assert (restored.uid, restored.folder, restored.linetype) == (tag.uid, folder, "dashdot")
    restored.visible = False
    action(native_host, "importtags", "")
    assert not restored.visible and scene.layer("Ejes") is restored

    # Bad input cannot leave a newly imported folder behind.
    path.write_text('{"ingetrazo_layers":1,"folders":["Partial"],"layers":[],"folder_uids":[1]}')
    action(native_host, "importtags", "")
    assert "Partial" not in L.folders_of(scene)
    scene.worksession_read_only = True
    path.write_text('{"ingetrazo_layers":1,"layers":[{"name":"Forbidden"}]}')
    action(native_host, "importtags", "")
    assert scene.layer("Forbidden") is None


def test_right_click_hides_and_isolates_the_clicked_tag(native_host):
    from tests.test_layer_assignment import tag_form
    scene = native_host.viewport.scene
    scene.layers.extend([Layer("A"), Layer("B")])
    face = _square(scene.mesh)
    assign_layer(face, "A")
    scene.select([face])
    for verb in ["Isolate selected tags", "Hide selected tags"]:
        form = tag_form(native_host)
        form.receive("item", str(form.field("item").keys().index(verb)))
        form.press("ok")
        assert not scene.layer("B").visible
    assert not scene.layer("A").visible and not scene.selection


def test_the_default_tag_swatch_distinguishes_no_colour_from_a_picked_or_automatic_colour():
    from tests.test_project_drawing_filters import Project
    project = Project(Layer("Muros"))
    scene = project.scene
    def displayed():
        project.action("lens", "1")
        return {t[0]: t[25] for t, _ in project.view.renderer.calls["ui_set_tree"][0]
                if t[0].startswith("preview-tag:")}
    assert displayed()["preview-tag:Layer 0"] == ""
    colour = ingetrazo_core.tag_styles.layer_color(scene.layer("Muros"))
    assert displayed()["preview-tag:Muros"] == "#" + "".join(f"{int(v * 255 + 0.5):02x}" for v in colour)
    scene.layer(DEFAULT_LAYER).color = (0.2, 0.4, 0.6)
    scene.version += 1
    assert displayed()["preview-tag:Layer 0"] == "#336699"


def test_the_delete_key_deletes_the_current_row(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    scene.layers += [Layer("A"), Layer("B")]
    action(native_host, "deletetag", "preview-tag:A")
    assert scene.layer("A") is None and scene.layer("B") is not None
    assert native_host.viewport.history.undo() and scene.layer("A") is not None
    action(native_host, "deletetag", "preview-tag:" + DEFAULT_LAYER)
    assert scene.layer(DEFAULT_LAYER) is not None and not native_host.viewport.history.undo_stack


def test_drawing_rows_show_the_document_line_type_and_weight():
    from tests.test_project_drawing_filters import Project
    p = Project(Layer("A", linetype="dashed", lineweight=.35), Layer("B"))
    p.action("lens", "1")
    text = {t[0]: t for t, _ in p.view.renderer.calls["ui_set_tree"][0]}
    assert text["preview-tag:A"][6:8] == ["dashed", "0.35"]
    assert text["preview-tag:B"][6] == "continuous"
    assert not p.scene.layer("B").lineweight


def test_selected_tags_take_the_same_line_and_projection_weight():
    from tests.test_project_drawing_filters import Project, drawing_edit
    p = Project(*(Layer(key) for key in "ABC"))
    drawing_edit(p, "drawingpatch", [["A","B"], {"linetype":"dashdot", "weight":"0.50"}])
    assert all((p.scene.layer(k).linetype, p.scene.layer(k).lineweight) == ("dashdot", .5) for k in "AB")
    assert p.scene.layer("C").linetype == "continuous"
    drawing_edit(p, "drawingpatch", [["A","B"], {"weight":"auto"}])
    assert all(p.scene.layer(k).lineweight is None for k in "AB")


def test_new_tag_at_a_tag_uses_its_folder_and_rejects_a_stale_target(native_host):
    from tests.native_tree import action
    scene = native_host.viewport.scene
    arq = ingetrazo_core.tag_edits.add_folder(scene, "Arq")
    scene.layers.extend([Layer("Muros", folder=arq), Layer("Suelta")])
    action(native_host, "newtag", "preview-tag:Muros")
    assert scene.layers[-1].folder == arq
    action(native_host, "newtag", "preview-tag:Suelta")
    assert scene.layers[-1].folder is None
    count = len(scene.layers)
    for key in ["preview-tag:missing", "preview-folder:missing"]:
        action(native_host, "newtag", key)
        assert len(scene.layers) == count


def test_the_layer_and_folder_of_the_model_selection_stand_out():
    from tests.test_project_drawing_filters import Project
    project = Project(Layer("Walls", folder="Building"))
    group = Group(Mesh(), name="Wall")
    group.layer = "Walls"
    project.scene.groups.append(group)
    project.action("lens", "1")
    project.action("expandall", "")
    for selected in ([group], []):
        project.scene.select(selected)
        rows = project.rows()
        for key in ("preview-tag:Walls", "preview-folder:Building"):
            assert rows[key][1][9] == len(selected)


def test_folder_projection_is_deferred_until_a_frame_and_reused_across_unchanged_frames(monkeypatch):
    from tests.test_project_drawing_filters import Project
    from views import wgpu_ui
    project = Project()
    project.action("lens", "1")
    builds = []
    original = wgpu_ui.prepare_tree_model
    def counted(scene):
        builds.append(scene.content_version)
        return original(scene)
    monkeypatch.setattr(wgpu_ui, "prepare_tree_model", counted)
    project.view.renderer.ui_project_hidden = lambda: True
    parent = ingetrazo_core.tag_edits.add_folder(project.scene, "Outer")
    child = ingetrazo_core.tag_edits.add_folder(project.scene, "Inner", parent=parent)
    orphan = ingetrazo_core.tag_edits.add_folder(project.scene, "Orphan")
    project.scene.folder_parents = {child: parent, orphan: "missing"}
    project.scene.version += 1
    wgpu_ui._sync_project_rows(project.view, project.win)
    assert not builds
    project.view.renderer.ui_project_hidden = lambda: False
    wgpu_ui._sync_project_rows(project.view, project.win)
    project.action("expandall", "")
    for _ in range(3):
        rows = project.rows()
        assert rows[f"preview-folder:{orphan}"][1][0] == rows[f"preview-folder:{parent}"][1][0] == 0
        assert rows[f"preview-folder:{child}"][1][0] == 1
    assert builds == [project.scene.content_version]
