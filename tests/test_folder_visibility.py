"""Synthetic regressions for independent folder visibility and stable tag rows."""
import built_core
import json

from tests.test_project_drawing_filters import Project
from views.wgpu_ui import SEP

from built_core import layers as L
from built_core.scene import Scene
from built_core.history import History
from built_core.protocol import commands


def document():
    scene = Scene()
    scene.layer_folders = ['parent', 'nested']
    scene.folder_parents = {'nested': 'parent'}
    scene.layers.extend([L.Layer('A', folder='nested'), L.Layer('B', folder='parent')])
    return scene


def test_independent_flags_and_every_undo_redo():
    scene = document()
    history = History(scene)
    states = []
    for names, folders, on in [([], ['parent'], False), (['A'], [], False),
                                ([], ['parent'], True)]:
        states.append(built_core.tag_edits.layer_state(scene))
        commands.set_tag_state(history, tags=[scene.layer(n).key for n in names],
                               folders=folders, visible=on)
        assert history.last_error is None
    assert not scene.layer('A').visible
    assert not built_core.tag_edits.effective_visible(scene, scene.layer('A'))
    assert built_core.tag_edits.effective_visible(scene, scene.layer('B'))
    final = built_core.tag_edits.layer_state(scene)
    for state in reversed(states):
        assert history.undo()
        assert built_core.tag_edits.layer_state(scene) == state
    for _ in states:
        assert history.redo()
    assert built_core.tag_edits.layer_state(scene) == final


def test_expand_collapse_reuses_the_document_projection():
    project = Project(scene=document())
    project.rows()
    projection = project.view._ui_tree_model[1]
    for expanded in (True, False, True):
        rows = project.action("toggle", "preview-folder:parent")
        assert ("preview-folder:nested" in rows) == expanded
        assert project.view._ui_tree_model[1] is projection


def test_visibility_and_rename_keep_row_keys_and_expansion():
    project = Project(scene=document())
    parent, nested = "preview-folder:parent", "preview-folder:nested"
    project.action("toggle", parent)
    rows = project.action("eye", parent)
    assert rows[parent][1][8] and not rows[nested][1][8]
    assert project.scene.layer("A").visible and not rows["preview-tag:B"][1][5]
    project.action("tagstate", json.dumps({"tags": ["A"], "visible": False}))
    project.action("eye", parent)
    rows = project.action("tagfolder", "rename" + SEP + json.dumps({"folder": "parent", "name": "Renamed"}))
    assert rows[parent][0] == "Renamed" and rows[parent][1][8]
    assert not rows[nested][1][8] and not project.scene.layer("A").visible


def test_structural_refresh_keeps_folded_folder():
    project = Project(scene=document())
    for _ in range(2):
        project.action("toggle", "preview-folder:parent")
    project.scene.layers.append(L.Layer("C"))
    project.scene.version += 1
    rows = project.rows()
    assert "preview-tag:C" in rows and "preview-folder:nested" not in rows
    assert not rows["preview-folder:parent"][1][8]


def test_igz_folder_and_child_flags_roundtrip_and_legacy(tmp_path):
    from formats.igz import save_scene, load_into
    import json
    scene = document()
    scene.folder_visibility = {'parent': False}
    scene.layer('A').visible = False
    path = tmp_path / 'synthetic.igz'
    save_scene(scene, path)
    loaded = Scene()
    load_into(loaded, path)
    assert loaded.folder_visibility == {'parent': False}
    assert loaded.layer('B').visible and not loaded.layer('A').visible
    payload = json.loads(path.read_text())
    del payload['scene']['layer_folder_visibility']
    path.write_text(json.dumps(payload))
    load_into(loaded, path)
    assert loaded.folder_visibility == {}
    assert built_core.tag_edits.effective_visible(loaded, loaded.layer('B'))
    assert not built_core.tag_edits.effective_visible(loaded, loaded.layer('A'))


def test_import_preserves_own_flags():
    from formats.skp import apply_payload
    scene = Scene()
    apply_payload(scene, {'layers': [{'name': 'A', 'visible': True}, {'name': 'B', 'visible': False}],
                          'layer_folders': [{'key': 'f', 'name': 'folder', 'hidden': True,
                                             'tags': ['A', 'B']}]})
    assert scene.layer('A').visible and not scene.layer('B').visible
    assert not built_core.tag_edits.effective_visible(scene, scene.layer('A'))
    scene.folder_visibility['folder'] = True
    assert built_core.tag_edits.effective_visible(scene, scene.layer('A'))
    assert not built_core.tag_edits.effective_visible(scene, scene.layer('B'))


def test_metal_visibility_uses_instance_updates_without_repack(monkeypatch):
    from tests.test_wgpu_updates import fixture_scene, Renderer, assert_matches_fresh_pack
    from views.wgpu_view import _paint_key
    scene, updates = fixture_scene()
    scene.layer_folders = ['folder']
    scene.layers.append(L.Layer('A', folder='folder'))
    scene.groups[0].layer = 'A'
    updates.apply(*updates.diff(scene), Renderer())
    old_key, old_bounds = _paint_key(scene), scene._bounds_key()
    monkeypatch.setattr('built_core.pack_scene', lambda *a, **k: (_ for _ in ()).throw(AssertionError('repack')))
    history = History(scene)
    commands.set_tag_state(history, folders=['folder'], visible=False)
    assert scene._bounds_key() != old_bounds
    assert _paint_key(scene) == old_key
    updates.journal_valid = False
    renderer = Renderer()
    delta = updates.diff(scene)
    assert delta is not None
    updates.apply(*delta, renderer)
    assert renderer.calls
    assert_matches_fresh_pack(updates, scene)
    history.undo()
    updates.journal_valid = False
    updates.apply(*updates.diff(scene), renderer)
    assert_matches_fresh_pack(updates, scene)


def test_folder_eye_sequence_is_three_undoable_toggles():
    scene = document()
    project = Project(scene=scene)
    history = project.win.viewport.history
    project.action("eye", "preview-folder:parent")
    assert scene.layer("A").visible
    project.action("tagstate", json.dumps({"tags": ["A"], "visible": False}))
    project.action("eye", "preview-folder:parent")
    assert not scene.layer("A").visible and scene.layer("B").visible
    assert len(history.undo_stack) == 3
    for _ in range(3):
        assert history.undo()
    assert scene.layer("A").visible and built_core.tag_edits.folder_visible(scene, "parent")
    for _ in range(3):
        assert history.redo()
    assert not scene.layer("A").visible and built_core.tag_edits.folder_visible(scene, "parent")


def test_linked_tags_keep_folder_flags_separate(tmp_path):
    scene = Scene()
    tags = {'layers': [L.Layer('A', folder='f').to_dict()],
            'folders': ['f'], 'folder_visibility': {'f': False}}
    built_core.link_sync_tags(scene, str(tmp_path / 'synthetic.igz'), tags)
    tag = next(ly for ly in scene.layers if ly.folder)
    assert tag.visible and not built_core.tag_edits.effective_visible(scene, tag)
    scene.folder_visibility[tag.folder] = True
    assert built_core.tag_edits.effective_visible(scene, tag)
