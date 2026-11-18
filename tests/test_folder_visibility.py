"""Synthetic regressions for independent folder visibility and stable tag rows."""
from types import SimpleNamespace

from PySide6.QtCore import Qt
from PySide6.QtWidgets import QApplication

from core import layers as L
from core.scene import Scene
from core.history import History, SetLayerVisibilityCommand


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
        states.append(L.layer_state(scene))
        history.execute(SetLayerVisibilityCommand(names, on, folders))
        assert history.last_error is None
    assert not scene.layer('A').visible
    assert not L.effective_visible(scene, scene.layer('A'))
    assert L.effective_visible(scene, scene.layer('B'))
    final = L.layer_state(scene)
    for state in reversed(states):
        assert history.undo()
        assert L.layer_state(scene) == state
    for _ in states:
        assert history.redo()
    assert L.layer_state(scene) == final


def panel_for(scene):
    from views.layers_panel import LayersPanel
    viewport = SimpleNamespace(scene=scene, history=History(scene), notify_scene_changed=lambda: None,
                               isAncestorOf=lambda widget: False)
    window = SimpleNamespace(viewport=viewport, isVisible=lambda: False,
                             statusBar=lambda: SimpleNamespace(showMessage=lambda *a: None))
    return LayersPanel(window)


def test_expand_collapse_never_refreshes(monkeypatch):
    from views.layers_panel import KIND_FOLDER, COL_NAME
    panel = panel_for(document())
    calls = []
    original = panel.refresh
    monkeypatch.setattr(panel, 'refresh', lambda: (calls.append(True), original()))
    item = panel.item_for('parent', KIND_FOLDER)
    for expanded in [False, True, False]:
        item.setExpanded(expanded)
        # A decoration/name-column notification is not a rename. Qt's
        # itemChanged carries a column, not the role that changed.
        panel.tree.itemChanged.emit(item, COL_NAME)
        QApplication.processEvents()
    assert calls == []
    assert panel.item_for('parent', KIND_FOLDER) is item
    panel.close()


def test_visibility_and_rename_keep_rows_and_expansion(monkeypatch):
    from views.layers_panel import KIND_FOLDER, COL_NAME, COL_EYE
    scene = document()
    panel = panel_for(scene)
    parent = panel.item_for('parent', KIND_FOLDER)
    nested = panel.item_for('nested', KIND_FOLDER)
    child = panel.item_for('A')
    nested.setExpanded(False)
    monkeypatch.setattr(panel, 'refresh', lambda: (_ for _ in ()).throw(AssertionError('rebuild')))
    panel.click_cell((KIND_FOLDER, 'parent'), COL_EYE)
    assert parent.isExpanded() and not nested.isExpanded()
    assert panel.item_for('A') is child
    assert child.data(COL_EYE, Qt.UserRole + 2)
    panel.set_visible(['A'], False)
    panel.click_cell((KIND_FOLDER, 'parent'), COL_EYE)
    assert not scene.layer('A').visible
    assert not child.data(COL_EYE, Qt.UserRole + 2)
    parent.setText(COL_NAME, 'Renamed')
    QApplication.processEvents()
    assert panel.item_for('parent', KIND_FOLDER) is parent
    assert L.folder_label(scene, 'parent') == 'Renamed'
    panel.close()


def test_structural_refresh_keeps_folded_folder():
    from views.layers_panel import KIND_FOLDER
    scene = document()
    panel = panel_for(scene)
    panel.item_for('parent', KIND_FOLDER).setExpanded(False)
    scene.layers.append(L.Layer('C'))
    panel.refresh()
    assert not panel.item_for('parent', KIND_FOLDER).isExpanded()
    panel.close()


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
    assert L.effective_visible(loaded, loaded.layer('B'))
    assert not L.effective_visible(loaded, loaded.layer('A'))


def test_import_preserves_own_flags():
    from formats.skp import apply_payload
    scene = Scene()
    apply_payload(scene, {'layers': [{'name': 'A', 'visible': True}, {'name': 'B', 'visible': False}],
                          'layer_folders': [{'key': 'f', 'name': 'folder', 'hidden': True,
                                             'tags': ['A', 'B']}]})
    assert scene.layer('A').visible and not scene.layer('B').visible
    assert not L.effective_visible(scene, scene.layer('A'))
    scene.folder_visibility['folder'] = True
    assert L.effective_visible(scene, scene.layer('A'))
    assert not L.effective_visible(scene, scene.layer('B'))


def test_metal_visibility_uses_instance_updates_without_repack(monkeypatch):
    from tests.test_wgpu_updates import fixture_scene, Renderer, assert_matches_fresh_pack
    from views.wgpu_view import _paint_key
    scene, updates = fixture_scene()
    scene.layer_folders = ['folder']
    scene.layers.append(L.Layer('A', folder='folder'))
    scene.groups[0].layer = 'A'
    updates.apply(*updates.diff(scene), Renderer())
    old_key, old_bounds = _paint_key(scene), scene._bounds_key()
    monkeypatch.setattr('views.wgpu_updates.pack_scene', lambda *a, **k: (_ for _ in ()).throw(AssertionError('repack')))
    history = History(scene)
    history.execute(SetLayerVisibilityCommand([], False, ['folder']))
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
    from views.layers_panel import KIND_FOLDER, COL_EYE
    scene = document()
    panel = panel_for(scene)
    history = panel._window.viewport.history
    panel.click_cell((KIND_FOLDER, 'parent'), COL_EYE)
    assert scene.layer('A').visible  # ancestor hiding never changes this
    panel.set_visible(['A'], False)
    panel.click_cell((KIND_FOLDER, 'parent'), COL_EYE)
    assert not scene.layer('A').visible and scene.layer('B').visible
    assert len(history.undo_stack) == 3
    for _ in range(3):
        assert history.undo()
        panel._refresh_after_change()
    assert scene.layer('A').visible and L.folder_visible(scene, 'parent')
    for _ in range(3):
        assert history.redo()
        panel._refresh_after_change()
    assert not scene.layer('A').visible and L.folder_visible(scene, 'parent')
    panel.close()


def test_linked_tags_keep_folder_flags_separate(tmp_path):
    from core.links import _apply_tags
    scene = Scene()
    tags = {'layers': [L.Layer('A', folder='f').to_dict()],
            'folders': ['f'], 'folder_visibility': {'f': False}}
    _apply_tags(scene, str(tmp_path / 'synthetic.igz'), tags)
    tag = next(ly for ly in scene.layers if ly.folder)
    assert tag.visible and not L.effective_visible(scene, tag)
    scene.folder_visibility[tag.folder] = True
    assert L.effective_visible(scene, tag)


def test_full_refresh_preserves_selection_and_scroll():
    scene = document()
    scene.layers.extend(L.Layer(f'Z{i:03}') for i in range(100))
    panel = panel_for(scene)
    panel.resize(500, 350)
    panel.show()
    QApplication.processEvents()
    item = panel.item_for('Z050')
    item.setSelected(True)
    panel.tree.setCurrentItem(item)
    panel.tree.scrollToItem(item)
    bar = panel.tree.verticalScrollBar()
    scroll = bar.value()
    assert scroll > 0
    scene.layers.append(L.Layer('ZZZ'))
    panel.refresh()
    assert panel.item_for('Z050').isSelected()
    assert panel.tree.currentItem() is panel.item_for('Z050')
    assert bar.value() == scroll
    panel.close()
