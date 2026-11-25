"""Device-free regression checks for resident edits and shared tool semantics."""
from types import SimpleNamespace
import numpy as np
import pytest
from core.input_codes import Input as Qt
from ingetrazo_core import ScreenPoint as QPointF
from ingetrazo_core import Vec3 as QVector3D, Matrix4 as QMatrix4x4
from core.group import Group
from core.history import MoveGroupCommand, CompoundCommand
from core.mesh import Mesh
from core.scene import Scene
from tools.base import ToolContext
from tools.tape import TapeMeasureTool
from ingetrazo_core import pack_scene
from ingetrazo_core import InstanceUpdates
from ingetrazo_render import TOOL_DISPATCH, move_command


def fixture_scene():
    mesh = Mesh()
    mesh.add_face([QVector3D(0, 0, 0), QVector3D(2, 0, 0), QVector3D(0, 3, 0)])
    groups = [Group(mesh), Group(mesh)]
    for g in groups:
        g.xform = QMatrix4x4()
    scene = Scene(groups=groups)
    return scene, InstanceUpdates(scene, pack_scene(scene))


def _prototype(packed, code):
    """What a prototype draws: its four ranges' vertices and indices."""
    parts = []
    for commands in (packed.commands, packed.edge_commands,
                     packed.hidden_commands, packed.hidden_edge_commands):
        c = commands[code]
        first, count = int(c['first']), int(c['count'])
        parts.append((packed.vertices[int(c['base']) + packed.indices[first:first+count].astype(np.int64)].tobytes(),
                      count))
    return parts, packed.bounds[code].tobytes()


def assert_matches_fresh_pack(updates, scene, **kw):
    """The resident pack after incremental edits draws what a fresh pack
    of the scene draws: same rows (by uid), same flags and paint, same
    prototype geometry, colours and hidden ranges."""
    fresh = pack_scene(scene, **kw)
    packed = updates.packed
    live = [r for r in range(len(packed.instances)) if r not in set(updates.removed)
            and updates.resident.is_active(packed.uids[r])]
    assert sorted(packed.uids[r] for r in live) == sorted(fresh.uids)
    for uid in fresh.uids:
        f = fresh.uids.index(uid)
        r = updates.resident.row(uid)
        np.testing.assert_array_equal(packed.instances[r]['world'], fresh.instances[f]['world'])
        np.testing.assert_array_equal(packed.instances[r]['info'][1:], fresh.instances[f]['info'][1:])
        assert (_prototype(packed, int(packed.instances[r]['info'][0]))
                == _prototype(fresh, int(fresh.instances[f]['info'][0])))


class Renderer:
    def __init__(self):
        self.calls = []

    def __getattr__(self, name):
        return lambda *args: self.calls.append((name, args))


def test_record_edits_submit_nothing():
    scene, updates = fixture_scene()
    scene.groups[0].ifc = {'class': 'IfcWall'}
    scene.version += 1
    renderer = Renderer()
    updates.apply(*updates.diff(scene), renderer)
    assert renderer.calls == []


def test_equal_leaf_uids_keep_distinct_metal_rows_on_hide():
    from core.history import History, HideCommand

    scene = Scene()
    mesh = Mesh()
    mesh.add_face([QVector3D(0, 0, 0), QVector3D(2, 0, 0), QVector3D(0, 3, 0)])
    for uid in ('left', 'right'):
        child = Group(mesh, uid='repeated')
        child.xform = QMatrix4x4()
        parent = Group(uid=uid)
        parent.adopt([child])
        scene.groups.append(parent)
    packed = pack_scene(scene)
    left, right = (parent.children[0] for parent in scene.groups)
    tracker = InstanceUpdates(scene, packed)
    history = History(scene)
    tracker.observe(history)
    ri, li = (tracker.resident.row(packed.row_key(g)) for g in (right, left))
    assert ri != li and packed.instances[ri]['info'][0] == packed.instances[li]['info'][0]
    history.execute(HideCommand([right]))
    tracker.apply(*tracker.diff(scene))
    assert packed.instances[ri]['info'][2] & 1 and not packed.instances[li]['info'][2] & 1
    assert_matches_fresh_pack(tracker, scene)
    assert history.undo()
    from core.agent_facets import set_display
    from ingetrazo_core import VISIBILITY_FILTERED
    vp = SimpleNamespace(scene=scene, update=lambda: None)
    set_display(vp, {'facet': 'occurrence', 'eq': packed.row_key(right)})
    filtered = pack_scene(scene)
    assert not filtered.instances[ri]['info'][2] & (VISIBILITY_FILTERED | 1)
    assert filtered.instances[li]['info'][2] & VISIBILITY_FILTERED



def test_move_one_row_without_bake(monkeypatch):
    scene, updates = fixture_scene()
    monkeypatch.setattr('ingetrazo_core.pack_scene', lambda *a: (_ for _ in ()).throw(AssertionError('bake')))
    scene.groups[1].xform.translate(4, 5, 6)
    ids, rows = updates.diff(scene)
    assert ids.tolist() == [1]
    renderer = Renderer()
    updates.apply(ids, rows, renderer)
    assert [name for name, _ in renderer.calls] == ['update_instances']
    np.testing.assert_array_equal(updates.packed.instances['world'][1, 3, :3], [4, 5, 6])


def test_add_shared_instance_and_remove_preserve_ids():
    scene, updates = fixture_scene()
    added = Group(scene.groups[0].mesh)
    added.xform = QMatrix4x4()
    scene.groups.append(added)
    ids, rows = updates.diff(scene)
    assert ids.tolist() == [2]
    updates.apply(ids, rows)
    scene.groups.pop(0)
    ids, rows = updates.diff(scene)
    updates.apply(ids, rows)
    assert updates.resident.row(added.uid) == 2
    assert updates.packed.instances['info'][0, 2] == 1


def test_shared_mutation_bakes_once(monkeypatch):
    scene, updates = fixture_scene()
    calls = []
    original = pack_scene
    monkeypatch.setattr('ingetrazo_core.pack_scene', lambda scene, **kw: (calls.append(scene), original(scene, **kw))[1])
    mesh = scene.groups[0].mesh
    mesh.move_vertex(mesh.vertices[0], QVector3D(0, 0, 2))
    renderer = Renderer()
    updates.apply(*updates.diff(scene), renderer)
    assert updates.changed_code_rows[0].tolist() == [0, 1]
    assert len(calls) == 1
    assert len(calls[0].groups) == 1
    assert [name for name, _ in renderer.calls] == ['replace_prototype', 'update_prototype_materials']


def test_move_command_matches_gl_and_undo():
    scene, _ = fixture_scene()
    delta = QVector3D(3, 4, 5)
    command = move_command(scene.groups[:1], delta)
    assert isinstance(command, MoveGroupCommand)
    before = scene.groups[0].xform.data()
    command.do(scene)
    assert scene.groups[0].xform.column(3).toVector3D() == delta
    command.undo(scene)
    assert scene.groups[0].xform.data() == before
    assert isinstance(move_command(scene.groups, delta), CompoundCommand)


def test_tape_matches_gl_measurement():
    from ingetrazo_render import NativeTools
    tool = TapeMeasureTool()
    source = SimpleNamespace(active_tool=tool, flash_status=lambda *a: None,
                             measurementChanged=SimpleNamespace(emit=lambda *a: None),
                             _measurement_text=lambda: '')
    view = SimpleNamespace(source_viewport=source, overlay_primitives=[], update=lambda: None)
    native = NativeTools(view)
    native.sync()
    tool._mode = 'measure'
    event = SimpleNamespace(position=lambda: QPointF(), modifiers=lambda: Qt.NoModifier)
    for point in (QVector3D(1, 2, 3), QVector3D(4, 6, 3)):
        view.last_snap = SimpleNamespace(point=point, kind='endpoint')
        native.click(event)
    gl = TapeMeasureTool()
    gl._mode = 'measure'
    viewport = SimpleNamespace(pick_edge_any=lambda *a: None, update=lambda: None,
                               flash_status=lambda *a: None)
    for point in (QVector3D(1, 2, 3), QVector3D(4, 6, 3)):
        gl.on_click(ToolContext(viewport, point, QPointF(), Qt.NoModifier, None))
    assert tool._measured == gl._measured == 5.


def test_new_prototype_preserves_resident_ids():
    scene, updates = fixture_scene()
    mesh = Mesh()
    mesh.add_edge(QVector3D(), QVector3D(1, 1, 1))
    group = Group(mesh)
    scene.groups.append(group)
    renderer = Renderer()
    updates.apply(*updates.diff(scene), renderer)
    assert updates.resident.row(group.uid) == 2
    assert updates.packed.instances['info'][2, 0] == 1
    assert [name for name, _ in renderer.calls] == ['replace_prototype', 'update_prototype_materials', 'update_instances']


def test_dispatch_is_explicit():
    assert {k: TOOL_DISPATCH[k] for k in ('SelectTool', 'TapeMeasureTool', 'MoveTool', 'LineTool', 'RectangleTool', 'PushPullTool', 'EraserTool')} == {'SelectTool': 'select', 'TapeMeasureTool': 'tape', 'MoveTool': 'move', 'LineTool': 'mesh', 'RectangleTool': 'mesh', 'PushPullTool': 'mesh', 'EraserTool': 'mesh'}
    assert TOOL_DISPATCH.get('TexturePositionTool', 'inert') == 'mesh'


def test_history_journal_limits_move_discovery(monkeypatch):
    from core.history import History, TagGroupsCommand
    scene, updates = fixture_scene()
    history = History(scene)
    updates.observe(history)
    history.execute(MoveGroupCommand(scene.groups[0], QVector3D(1, 0, 0)))
    ids, rows = updates.diff(scene)
    assert ids.tolist() == [0]
    updates.apply(ids, rows)
    monkeypatch.setattr('core.purge.iter_groups', lambda *a: (_ for _ in ()).throw(AssertionError('scan')))
    history.execute(TagGroupsCommand(scene.groups, 'IfcWall'))
    renderer = Renderer()
    updates.apply(*updates.diff(scene), renderer)
    assert renderer.calls == []


def test_updated_bounds_match_fresh_pack():
    scene, updates = fixture_scene()
    scene.groups[0].xform.translate(20, -5, 8)
    updates.apply(*updates.diff(scene))
    np.testing.assert_allclose(updates.packed.world_bounds, pack_scene(scene).world_bounds)
    scene.groups.pop(0)
    updates.apply(*updates.diff(scene))
    np.testing.assert_allclose(updates.packed.world_bounds, pack_scene(scene).world_bounds)


def test_prototype_reassignment_preserves_row():
    scene, updates = fixture_scene()
    mesh = Mesh()
    mesh.add_edge(QVector3D(), QVector3D(2, 2, 2))
    scene.groups[0].mesh = mesh
    updates.apply(*updates.diff(scene))
    assert updates.resident.row(scene.groups[0].uid) == 0
    assert updates.packed.instances['info'][:, 0].tolist() == [1, 0]
    assert updates.packed.commands['start'].tolist() == [0, 1]


@pytest.mark.parametrize("completion", ["click", "escape", "enter"])
def test_native_move_preview_then_history_commit(completion):
    from core.history import History
    from tools.move import MoveTool
    from ingetrazo_render import NativeTools
    scene, updates = fixture_scene()
    scene.select([scene.groups[0]])
    history = History(scene)
    source = SimpleNamespace(active_tool=MoveTool(), history=history,
                             flash_status=lambda *a: None, notify_scene_changed=lambda: None,
                             measurementChanged=SimpleNamespace(emit=lambda *a: None))
    # Like the real view, re-apply resident rows only when the document
    # changed: MoveTool's own update() calls set_scene during the preview,
    # and re-applying the unchanged document would wipe the previewed rows.
    seen = {'content': scene.content_version}

    def set_scene(scene):
        if scene.content_version != seen['content']:
            seen['content'] = scene.content_version
            updates.apply(*updates.diff(scene))
    view = SimpleNamespace(scene=scene, packed=updates.packed, renderer=Renderer(),
                           source_viewport=source, overlay_primitives=[],
                           uid_rows=updates.uid_rows,
                           _instance_updates=updates, update=lambda: None,
                           _world_to_pixel=lambda point: (point.x(), point.y()),
                           set_scene=set_scene)
    native = NativeTools(view)
    event = SimpleNamespace(position=lambda: QPointF(), modifiers=lambda: Qt.NoModifier)
    view.last_snap = SimpleNamespace(point=QVector3D(), kind='endpoint')
    native.click(event)
    view.last_snap = SimpleNamespace(point=QVector3D(2, 0, 0), kind='endpoint')
    native.hover(event)
    assert scene.groups[0].xform.column(3).x() == 0
    assert view.packed.instances['world'][0, 3, 0] == 2
    from tests.headless_gpu import HeadlessView
    view.tools = native
    if completion == 'escape':
        HeadlessView.keyPressEvent(view, SimpleNamespace(key=lambda: Qt.Key_Escape))
        assert not history.undo_stack
        assert view.packed.instances['world'][0, 3, 0] == 0
        return
    if completion == 'enter':
        HeadlessView.keyPressEvent(view, SimpleNamespace(key=lambda: Qt.Key_4, text=lambda: '4', modifiers=lambda: Qt.NoModifier))
        HeadlessView.keyPressEvent(view, SimpleNamespace(key=lambda: Qt.Key_Return, text=lambda: '', modifiers=lambda: Qt.NoModifier))
    else:
        native.click(event)
    assert isinstance(history.undo_stack[-1], MoveGroupCommand)
    assert scene.groups[0].xform.column(3).x() == (4 if completion == 'enter' else 2)
    history.undo()
    assert scene.groups[0].xform.column(3).x() == 0


def test_inference_move_reuses_prototype_and_hits_updated_row():
    from views.wgpu_inference import prepare, surface
    scene, updates = fixture_scene()
    index = prepare(updates.packed)
    base, prototype = index.base, index.protos[0]
    scene.groups[0].xform.translate(10, 0, 0)
    ids, rows = updates.diff(scene)
    updates.apply(ids, rows)
    index.update(ids, updates.packed)
    assert index.base is base
    assert index.protos[0] is prototype
    hit = surface(index, 0, QVector3D(10.25, .25, 1), QVector3D(0, 0, -1))
    assert hit == QVector3D(10.25, .25, 0)
    assert surface(index, 1, QVector3D(.25, .25, 1), QVector3D(0, 0, -1)) == QVector3D(.25, .25, 0)


def test_inference_reads_committed_rows_from_the_native_index():
    """No copy of the instance table: a row not updated since prepare
    reads from the native placements, an updated one from the update."""
    from views.wgpu_inference import prepare
    scene, updates = fixture_scene()
    index = prepare(updates.packed)
    assert not hasattr(index, 'initial')
    code, world = index.committed(1)
    assert code == int(updates.packed.instances['info'][1, 0])
    np.testing.assert_array_equal(world, updates.packed.instances['world'][1].astype(float))
    scene.groups[1].xform.translate(5, 0, 0)
    ids, rows = updates.diff(scene)
    updates.apply(ids, rows)
    index.update(ids, updates.packed)
    assert index.committed(1)[1][3, 0] == 5


def test_moving_geometry_leaves_the_snap_candidates_as_columns():
    from ingetrazo_core import ResidentEdges
    a = np.array([[0., 0, 0], [1, 0, 0], [5, 5, 5]])
    b = np.array([[1., 0, 0], [1, 1, 0], [6, 5, 5]])
    context = object()
    groups = [context, object()]
    edges = ResidentEdges((np.array([0, 0, 1]), np.array([0, 1, 2]), a, b, np.zeros(3)), groups, context)
    assert edges.in_group().tolist() == [False, False, True]
    kept = edges.keep([True, False, True])
    assert isinstance(kept, ResidentEdges) and kept.ids.tolist() == [0, 2]
    assert kept.snap_columns()[0].shape == (2, 3)


def test_inference_shared_mesh_change_invalidates_only_its_prototype():
    from views.wgpu_inference import prepare, surface
    scene, updates = fixture_scene()
    index = prepare(updates.packed)
    mesh = scene.groups[0].mesh
    for vertex in list(mesh.vertices):
        mesh.move_vertex(vertex, QVector3D(0, 0, 2))
    ids, rows = updates.diff(scene)
    updates.apply(ids, rows)
    index.update(ids, updates.packed, [code for _, _, code in updates.changed])
    assert surface(index, 1, QVector3D(.25, .25, 3), QVector3D(0, 0, -1)) == QVector3D(.25, .25, 2)


def test_uid_keeps_row_when_document_replaces_group_object():
    scene, updates = fixture_scene()
    replacement = Group(scene.groups[0].mesh)
    replacement.uid = scene.groups[0].uid
    replacement.xform = QMatrix4x4()
    scene.groups[0] = replacement
    renderer = Renderer()
    updates.apply(*updates.diff(scene), renderer)
    assert updates.packed.groups[0] is replacement
    assert renderer.calls == []


def test_history_insert_undo_redo_keeps_one_row(monkeypatch):
    from core.history import History, InsertGroupCommand
    from core.doc_model import NATIVE
    doc_tree, render_rows = NATIVE.doc_tree, NATIVE.doc_render_rows
    scene, updates = fixture_scene()
    history = History(scene)
    updates.observe(history)
    added = Group(scene.groups[0].mesh)
    added.xform = QMatrix4x4()
    visited = []
    def walk(roots):
        visited.extend(roots)
        return doc_tree(roots)

    def rows(roots, *rest):
        visited.extend(roots)
        return render_rows(roots, *rest)
    monkeypatch.setattr(NATIVE, "doc_tree", walk)
    monkeypatch.setattr(NATIVE, "doc_render_rows", rows)
    history.execute(InsertGroupCommand(added))
    ids, rows = updates.diff(scene)
    assert visited == [added]
    assert ids.tolist() == [2]
    updates.apply(ids, rows)
    history.undo()
    updates.apply(*updates.diff(scene))
    assert updates.packed.instances['info'][2, 2] == 1
    history.redo()
    ids, rows = updates.diff(scene)
    assert ids.tolist() == [2]
    updates.apply(ids, rows)
    assert updates.packed.instances['info'][2, 2] == 0


def test_tool_change_signal_reports_unsupported_tool_without_mouse_input():
    from tests.headless_gpu import HeadlessView
    messages = []

    class Signal:
        def __init__(self):
            self.slots = []

        def connect(self, slot):
            self.slots.append(slot)

        def emit(self, *args):
            for slot in self.slots:
                slot(*args)

    class Source:
        active_tool = None

        def __init__(self):
            self.measurementChanged = Signal()

        def flash_status(self, message):
            messages.append(message)

    source = Source()
    view = HeadlessView()
    view.source_viewport = source
    view._observe_tool_source()
    view.isVisible = lambda: True            # shown: the sync is only for a visible view
    source.active_tool = SimpleNamespace()
    source.measurementChanged.emit('')
    view.run_timers()
    assert view.tools.mode == 'inert'
    assert messages == ['Unsupported document tool']
    view.close()


def test_prototype_bake_does_not_consume_document_group_names(monkeypatch):
    import itertools
    import core.group as groups
    monkeypatch.setattr(groups, '_counter', itertools.count(100))
    scene, updates = fixture_scene()
    before = next(groups._counter)
    mesh = scene.groups[0].mesh
    mesh.move_vertex(mesh.vertices[0], QVector3D(0, 0, 1))
    updates.apply(*updates.diff(scene))
    assert next(groups._counter) == before + 1


def test_replacement_preserves_full_material_rows_and_resident_texture_ids(tmp_path):
    from ingetrazo_render import SurveyRaster
    meshes = []
    for name, rgba in (('red', (255, 0, 0, 255)), ('blue', (0, 0, 255, 255))):
        image = SurveyRaster(2, 2, bytes(rgba) * 4)
        path = tmp_path / (name + '.png')
        assert image.save(str(path))
        mesh = Mesh()
        mesh.add_face([QVector3D(), QVector3D(1, 0, 0), QVector3D(0, 1, 0)])
        mesh.faces[0].attrs['texture'] = {'path': str(path)}
        meshes.append(mesh)
    scene = Scene(groups=[Group(mesh) for mesh in meshes])
    updates = InstanceUpdates(scene, pack_scene(scene))
    meshes[1].move_vertex(meshes[1].vertices[0], QVector3D(0, 0, .1))
    renderer = Renderer()
    updates.apply(*updates.diff(scene), renderer)
    materials = next(args[1] for name, args in renderer.calls if name == 'update_prototype_materials')
    assert materials.nbytes == 3 * 32
    assert materials.view(np.uint32).reshape(-1, 8)[:, 5].tolist() == [2, 2, 2]
    assert not any(name == 'set_textures' for name, _ in renderer.calls)


def test_drag_preview_batches_rows_without_rendering_ids():
    from ingetrazo_render import NativeTools
    from ingetrazo_core import INSTANCE
    renderer = Renderer()
    packed = SimpleNamespace(instances=np.zeros(2000, dtype=INSTANCE))
    view = SimpleNamespace(renderer=renderer, packed=packed)
    native = NativeTools(view)
    native.ids = np.arange(0, 2000, 2, dtype=np.int64)
    rows = packed.instances[native.ids].copy()
    rows['world'][:, 3, 0] = 7
    native.write_rows(rows)
    assert [name for name, _ in renderer.calls] == ['update_instances']
    assert renderer.calls[0][1][0] == native.ids.tolist()
    assert len(renderer.calls[0][1][1]) == 1000 * 80
    assert np.all(packed.instances['world'][native.ids, 3, 0] == 7)
    assert np.all(packed.instances['world'][1::2, 3, 0] == 0)


def test_mesh_history_journal_does_not_walk_placements(monkeypatch):
    from core.history import History, SnapshotMutation
    scene, updates = fixture_scene()
    scene.begin_group_edit(scene.groups[0])
    updates.apply(*updates.diff(scene))
    history = History(scene)
    updates.observe(history)
    mesh = scene.mesh
    history.execute(SnapshotMutation(lambda s: s.mesh.move_vertex(
        s.mesh.vertices[0], QVector3D(0, 0, 1))))
    monkeypatch.setattr('core.purge.iter_groups',
                        lambda *a: (_ for _ in ()).throw(AssertionError('placement scan')))
    updates.apply(*updates.diff(scene))
    assert len(updates.changed) == 1
    assert updates.changed[0][0] is mesh.__dict__.get("_instance_definition", mesh)
    history.undo()
    updates.apply(*updates.diff(scene))
    assert len(updates.changed) == 1
    history.redo()
    updates.apply(*updates.diff(scene))
    assert len(updates.changed) == 1


def test_nested_row_edits_preserve_painted_variants_and_inherited_flags():
    scene, _ = fixture_scene()
    a, b = scene.groups
    root = Group()
    root.adopt([a])
    root.material = {'opacity': .5, 'color': [.2, .3, .4]}
    scene.groups[:] = [root, b]
    updates = InstanceUpdates(scene, pack_scene(scene))
    root.xform.translate(1, 2, 3)
    root.hidden = True
    ids, rows = updates.diff(scene)
    assert ids.tolist() == [updates.resident.row(a.uid)]
    updates.apply(ids, rows)
    np.testing.assert_array_equal(updates.packed.instances, pack_scene(scene).instances)
    a.mesh.move_vertex(a.mesh.vertices[0], QVector3D(0, 0, 2))
    # The mesh is drawn twice, plain (b) and under the root's opacity (a):
    # both variants rebake, neither one repacks the scene.
    updates.apply(*updates.diff(scene))
    assert_matches_fresh_pack(updates, scene)


def test_hide_command_on_nested_container_updates_only_instance_flags():
    from core.history import History, HideCommand
    scene, _ = fixture_scene()
    root = Group()
    root.adopt(scene.groups[:1])
    scene.groups[0] = root
    updates = InstanceUpdates(scene, pack_scene(scene))
    history = History(scene)
    updates.observe(history)
    history.execute(HideCommand([root]))
    updates.apply(*updates.diff(scene))
    np.testing.assert_array_equal(updates.packed.instances, pack_scene(scene).instances)
    history.undo()
    updates.apply(*updates.diff(scene))
    np.testing.assert_array_equal(updates.packed.instances, pack_scene(scene).instances)


def test_new_paint_variant_bakes_without_repack(monkeypatch):
    """Painting a container with opacity gives its nested mesh a baked
    variant it never had: one prototype bake, not a scene repack."""
    scene, _ = fixture_scene()
    a, b = scene.groups
    root = Group()
    root.adopt([a])
    scene.groups[:] = [root, b]
    updates = InstanceUpdates(scene, pack_scene(scene))
    root.material = {'opacity': .5, 'color': [.2, .3, .4]}
    ids, rows = updates.diff(scene)
    assert [m for m, _, _ in updates.changed] == [a.mesh]
    updates.apply(ids, rows)
    assert_matches_fresh_pack(updates, scene)
    # Both variants follow a later geometry edit.
    a.mesh.move_vertex(a.mesh.vertices[1], QVector3D(5, 0, 0))
    updates.apply(*updates.diff(scene))
    assert len(updates.changed) == 2
    assert_matches_fresh_pack(updates, scene)


def test_attribute_only_edit_without_journal_rebakes_that_mesh():
    scene, updates = fixture_scene()
    mesh = scene.groups[0].mesh
    face = mesh.faces[0]
    face.attrs['color'] = (1.0, 0.0, 0.0)
    mesh.bump_revision()                     # what the paint commands do
    updates.apply(*updates.diff(scene))
    assert [m for m, _, _ in updates.changed] == [mesh]
    assert_matches_fresh_pack(updates, scene)


def test_mesh_history_edit_inside_nested_container_does_not_repack(monkeypatch):
    from core.history import History, SnapshotMutation
    scene, _ = fixture_scene()
    a, b = scene.groups
    root = Group()
    root.adopt([a])
    scene.groups[:] = [root, b]
    updates = InstanceUpdates(scene, pack_scene(scene))
    history = History(scene)
    updates.observe(history)
    scene.begin_group_edit(root)
    scene.begin_group_edit(a)
    history.execute(SnapshotMutation(lambda s: s.mesh.move_vertex(
        s.mesh.vertices[0], QVector3D(0, 0, 4))))
    scene.end_group_edit()
    scene.end_group_edit()
    calls = []
    original = pack_scene
    monkeypatch.setattr('ingetrazo_core.pack_scene',
                        lambda s, **kw: (calls.append(len(s.groups)), original(s, **kw))[1])
    updates.apply(*updates.diff(scene))
    assert calls == [1]                       # one prototype, not the scene
    assert_matches_fresh_pack(updates, scene)


def test_a_rebake_the_size_of_the_scene_falls_back_to_a_full_pack():
    """Revisions also move without an edit (a lazy mesh built, commands that
    flag every group mesh). Rebaking most of the scene in place is slower
    than one pack and left a blank GPU scene on the facilities model."""
    meshes = []
    for i in range(300):
        m = Mesh()
        m.add_face([QVector3D(i, 0, 0), QVector3D(i + 1, 0, 0), QVector3D(i, 1, 0)])
        meshes.append(m)
    scene = Scene(groups=[Group(m) for m in meshes])
    updates = InstanceUpdates(scene, pack_scene(scene))
    for m in meshes[:10]:
        m.bump_revision()
    assert updates.diff(scene) is not None and len(updates.changed) == 10
    for m in meshes:
        m.bump_revision()
    assert updates.diff(scene) is None


def test_native_incremental_matches_full_rebuild():
    from views.wgpu_inference import prepare
    scene, updates = fixture_scene()
    index = prepare(updates.packed, scene)
    base = index.base
    for vertex in list(scene.groups[0].mesh.vertices):
        scene.groups[0].mesh.move_vertex(vertex, QVector3D(0, 0, 2))
    ids, rows = updates.diff(scene)
    updates.apply(ids, rows)
    index.update(ids, updates.packed, [code for _, _, code in updates.changed])
    fresh = prepare(updates.packed, scene)
    assert index.base is base
    rows = np.arange(len(updates.packed.instances), dtype=np.int64)
    for a, b in zip(index.ray_cands(rows, (.25, .25, 4), (0., 0., -1.), 10.),
                    fresh.ray_cands(rows, (.25, .25, 4), (0., 0., -1.), 10.)):
        np.testing.assert_allclose(a, b)


def _nested_component_scene():
    """root ▸ inner ▸ leaf, the leaf a component instance (an ``xform`` and
    a definition a top-level sibling shares), every level transformed: the
    probe's ride edit (a support column five levels deep) in miniature."""
    def moved(x, y, z):
        m = QMatrix4x4()
        m.translate(x, y, z)
        return m
    mesh = Mesh()
    mesh.add_face([QVector3D(0, 0, 0), QVector3D(1, 0, 0), QVector3D(1, 1, 0), QVector3D(0, 1, 0)])
    leaf, sibling = Group(mesh, name='leaf'), Group(mesh, name='sibling')
    leaf.xform, sibling.xform = moved(1, 0, 0), moved(5, 0, 0)
    inner, root = Group(name='inner'), Group(name='root')
    inner.adopt([leaf])
    inner.xform = moved(0, 2, 0)
    root.adopt([inner])
    root.xform = moved(0, 0, 3)
    scene = Scene(groups=[root, sibling])
    return scene, leaf, inner, root, mesh


def test_component_edit_deep_in_nested_groups_rewrites_one_row(monkeypatch):
    """Entering a component instance swaps its mesh for a world copy the
    tracker never saw; the first edit used to walk every placement of the
    model (2 s on the ride) and the last append copied the resident arrays
    (0.5-1.4 s). The swap is one row from the edit path, a rebake that keeps
    its size goes back in place, and leaving shares back without a walk."""
    from core.history import History, MoveVerticesCommand
    from core.reference_edit import ReshareInstanceCommand
    scene, leaf, inner, root, proto = _nested_component_scene()
    updates = InstanceUpdates(scene, pack_scene(scene))
    history = History(scene)
    updates.observe(history)
    for group in (root, inner, leaf):
        scene.begin_group_edit(group)
    assert scene.mesh is not proto                       # the world copy
    import ingetrazo_core

    walk = ingetrazo_core.placement_records

    def records(scene, roots, *rest):
        assert not roots, 'placement walk'
        return walk(scene, roots, *rest)
    monkeypatch.setattr('ingetrazo_core.placement_records', records)
    bakes = []
    original = pack_scene
    monkeypatch.setattr('ingetrazo_core.pack_scene',
                        lambda s, **kw: (bakes.append(len(s.groups)), original(s, **kw))[1])

    def move():
        face = scene.mesh.faces[0]
        history.execute(MoveVerticesCommand(list(face.vertices), face.normal() * .01,
                                            carry_textures=True))
        delta = updates.diff(scene)
        assert delta is not None
        updates.apply(*delta)
        assert updates.last_local

    move()
    assert bakes == [1]
    assert [int(r) for r in updates.diff(scene)[0]] == []          # settled
    assert_matches_fresh_pack(updates, scene)
    size = len(updates.packed.vertices)
    move()                                                         # same counts
    assert len(updates.packed.vertices) == size
    assert_matches_fresh_pack(updates, scene)

    share = scene._edit_stack[-1].pop("share", None)
    scene.end_group_edit()
    if share is not None:
        group, definition, xform, _ = share
        history.execute(ReshareInstanceCommand(group, definition, xform, group.mesh))
    else:
        assert leaf.mesh is proto
    updates.apply(*updates.diff(scene))
    assert updates.last_local and leaf.mesh is proto
    assert_matches_fresh_pack(updates, scene)                      # the sibling too
    history.undo()
    updates.apply(*updates.diff(scene))
    assert updates.last_local
    assert_matches_fresh_pack(updates, scene)


def test_reserved_room_takes_the_first_append_without_a_copy():
    from concurrent.futures import ThreadPoolExecutor
    scene, updates = fixture_scene()
    size = len(updates.packed.vertices)
    with ThreadPoolExecutor(1) as pool:
        updates.reserve(pool)
        old = updates.adopt_reserved(wait=True)
    assert old is not None and updates.packed.room is not None
    room = updates.packed.room
    mesh = scene.groups[0].mesh
    mesh.add_face([QVector3D(0, 0, 1), QVector3D(1, 0, 1), QVector3D(0, 1, 1)])
    updates.apply(*updates.diff(scene))
    # Appended into the reserved room: the same allocation, not a copy.
    assert updates.packed.room is room and len(updates.packed.vertices) > size
    assert len(updates.packed.vertices) <= room.capacity
    assert_matches_fresh_pack(updates, scene)


def _soft_quads():
    def quad(x):
        m = Mesh()
        m.add_face([QVector3D(x, 0, 0), QVector3D(x+1, 0, 0), QVector3D(x+1, 1, 0), QVector3D(x, 1, 0)])
        m.add_face([QVector3D(x+1, 0, 0), QVector3D(x+2, 0, 1), QVector3D(x+2, 1, 1), QVector3D(x+1, 1, 0)])
        for edge in m.edges:
            if len(edge.faces) == 2:
                edge.soft = True
        return m
    scene = Scene(groups=[Group(quad(0)), Group(quad(5))])
    return scene, InstanceUpdates(scene, pack_scene(scene))


def test_edit_sends_only_its_own_profile_exclusions():
    """A native module that keeps exclusions per prototype gets only the
    rebaked ones: sending the whole list cost 0.45 s per edit on the ride."""
    scene, updates = _soft_quads()
    assert len(updates.packed.profile_exclusions) == 2
    renderer = Renderer()
    mesh = scene.groups[0].mesh
    mesh.move_vertex(mesh.vertices[0], QVector3D(0, 0, .5))
    updates.apply(*updates.diff(scene), renderer)
    sent = [args[0] for name, args in renderer.calls if name == 'set_profile_exclusions']
    code = updates.changed[0][2]
    assert len(sent) == 1 and [c for c, _ in sent[0]] == [code]


def test_older_module_gets_the_whole_list_when_an_exclusion_moved():
    class Older(Renderer):
        incremental_profile_exclusions = False
    scene, updates = _soft_quads()
    renderer = Older()
    mesh = scene.groups[0].mesh
    soft = next(e for e in mesh.edges if e.soft)
    vertex = next(v for v in mesh.vertices if v.position == soft.a)
    mesh.move_vertex(vertex, QVector3D(0, 0, .5))
    updates.apply(*updates.diff(scene), renderer)
    sent = [args[0] for name, args in renderer.calls if name == 'set_profile_exclusions']
    assert len(sent) == 1 and len(sent[0]) == 2


def test_an_edit_keeps_the_container_boxes_it_did_not_touch():
    """Every set_scene used to drop the container boxes measured after the
    open (0.9 s each on the ride, measured again at the next double-click)."""
    from tests.headless_gpu import HeadlessView
    scene, updates = fixture_scene()
    view = SimpleNamespace(packed=updates.packed, _container_boxes={}, _container_box_rows={})
    far, near = object(), object()
    boxes = {far: 'far box', near: 'near box'}
    rows = {far: np.array([1]), near: np.array([0])}
    tracker = SimpleNamespace(last_local=True, added=0, removed=[], changed=[])
    HeadlessView._keep_container_boxes(view, tracker, np.array([0]), boxes, rows)
    assert view._container_boxes == {far: 'far box'}
    view._container_boxes = {}
    tracker.added = 1                       # a new row may sit in any container
    HeadlessView._keep_container_boxes(view, tracker, np.array([0]), boxes, rows)
    assert view._container_boxes == {}


def test_draw_snapshot_and_guides_do_not_scan_unrelated_placements(monkeypatch):
    from core.history import History, SnapshotCompound, AddEdgeCommand, AddGuideCommand
    from core.guide import Guide
    scene, updates = fixture_scene()
    history = History(scene)
    updates.observe(history)
    scene.begin_group_edit(scene.groups[0])
    updates.remember_edit_path()
    updates.apply(*updates.diff(scene))
    original = __import__('ingetrazo_core').placement_records

    def bounded(scene, roots, *args):
        roots = list(roots)
        assert not roots, 'small edit traversed unrelated placements'
        return original(scene, roots, *args)

    monkeypatch.setattr('ingetrazo_core.placement_records', bounded)
    for command in (SnapshotCompound([AddEdgeCommand(QVector3D(8, 0, 0), QVector3D(9, 0, 0))]),
                    AddGuideCommand(Guide(QVector3D(), QVector3D(1, 0, 0)))):
        history.execute(command)
        assert history.last_error is None
        for action in (None, history.undo, history.redo):
            if action:
                action()
            updates.apply(*updates.diff(scene))
            assert updates.last_local
            assert_matches_fresh_pack(updates, scene)


def test_new_default_layer_does_not_change_resident_paint_key():
    from core.layers import Layer
    from views.wgpu_view import _paint_key
    scene = Scene()
    before = _paint_key(scene)
    scene.layers.append(Layer('Guides'))
    assert _paint_key(scene) == before
    scene.layers[-1].opacity = .5
    assert _paint_key(scene) != before


def test_bridge_edits_preserve_nested_face_and_edge_draw_ranges():
    from plugins.ai_bridge import _Bridge
    from core.history import History
    scene, leaf, inner, root, mesh = _nested_component_scene()
    second = Mesh()
    second.add_face([QVector3D(), QVector3D(2, 0, 0), QVector3D(0, 2, 0)])
    other = Group(second)
    scene.groups.append(other)
    root.xform.rotate(31, 1, 0, 0)
    updates = InstanceUpdates(scene, pack_scene(scene))
    history = History(scene)
    updates.observe(history)
    bridge = SimpleNamespace(_scope={'leaf': leaf, 'other': other},
                             _viewport=SimpleNamespace(scene=scene, history=history,
                                                       notify_scene_changed=lambda: None))
    for code in (
        'leaf.mesh.move_vertex(leaf.mesh.vertices[0], QVector3D(.25, 0, 1))',
        'leaf.mesh = other.mesh',
        'leaf.mesh = Mesh()',
        'leaf.mesh = Mesh(); leaf.mesh.add_face([QVector3D(), QVector3D(1,0,0), QVector3D(1,1,0)])',
        'for i in range(20): leaf.mesh.add_face([QVector3D(i,2,0), QVector3D(i+1,2,0), QVector3D(i,3,0)])',
    ):
        reply = _Bridge._tool_run_python(bridge, code)
        assert reply['changed'] and not reply['error']
        delta = updates.diff(scene)
        assert delta is not None
        updates.apply(*delta)
        assert_matches_fresh_pack(updates, scene)
        # First instance is the offset into survivors for this prototype.
        packed = updates.packed
        counts = np.bincount(packed.instances['info'][:, 0], minlength=len(packed.bounds))
        for commands in (packed.commands, packed.edge_commands):
            np.testing.assert_array_equal(commands['start'], counts.cumsum() - counts)


def test_rebaked_billboard_keeps_hidden_edges_at_their_own_offsets():
    scene, updates = fixture_scene()
    group = scene.groups[0]
    group.billboard = 'mesh'
    scene.show_hidden_geometry = True
    mesh = group.mesh
    mesh.edges[0].hidden = True
    updates = InstanceUpdates(scene, pack_scene(scene))
    mesh.move_vertex(mesh.vertices[0], QVector3D(.25, 0, 1))
    updates.apply(*updates.diff(scene))
    assert_matches_fresh_pack(updates, scene)


def test_callable_agent_edits_leave_other_deferred_meshes_resident():
    from core.ai import run_callable_transactional
    from core.history import History
    from core.mesh import DeferredMesh

    def build(m):
        m.add_face([QVector3D(), QVector3D(1, 0, 0), QVector3D(0, 1, 0)])

    meshes = [Mesh(), Mesh()]
    for m in meshes:
        DeferredMesh.defer(m, build, 1)
    scene = Scene(groups=[Group(m) for m in meshes])
    history = History(scene)
    viewport = SimpleNamespace(scene=scene, history=history, notify_scene_changed=lambda: None)
    def edit():
        m = meshes[0]
        m.move_vertex(m.vertices[0], QVector3D(0, 0, 2))
        return True
    assert run_callable_transactional(viewport, edit)
    assert type(meshes[1]) is DeferredMesh
    vertex = meshes[0].vertices[0]
    assert vertex.position.z() == 2
    history.undo()
    assert type(meshes[1]) is DeferredMesh
    assert vertex == meshes[0].vertices[0] and vertex.position.z() == 0
    history.redo()
    assert type(meshes[1]) is DeferredMesh
    assert vertex == meshes[0].vertices[0] and vertex.position.z() == 2


def test_agent_adding_group_on_active_layer_rebakes_only_new_mesh():
    from core.ai import run_transactional
    from core.history import History
    from core.layers import Layer
    scene, updates = fixture_scene()
    scene.layers.append(Layer("Active"))
    scene.active_layer = "Active"
    history = History(scene)
    updates.observe(history)
    viewport = SimpleNamespace(scene=scene, history=history, notify_scene_changed=lambda: None)
    reply = run_transactional(viewport, 'g=Group(); g.mesh.add_edge(QVector3D(), QVector3D(1,0,0)); groups.append(g)', {})
    assert reply['changed']
    delta = updates.diff(scene)
    assert [mesh for mesh, _, _ in updates.changed] == [scene.groups[-1].mesh]
    updates.apply(*delta)
    assert_matches_fresh_pack(updates, scene)


def test_reupload_after_offscreen_bridge_edit_keeps_material_headers():
    scene, updates = fixture_scene()
    replacement = Mesh()
    face = replacement.add_face([QVector3D(0,0,0), QVector3D(2,0,0), QVector3D(0,1,0)])
    face.attrs['back'] = {'color': [.2, .4, .8]}
    scene.groups[0].mesh = replacement
    updates.apply(*updates.diff(scene))  # edit while the GPU view has no renderer
    uploaded = Renderer()
    updates.packed.upload(uploaded)
    stream = next(args[0] for name, args in uploaded.calls if name == 'set_materials')
    code = int(updates.packed.instances[0]['info'][0])
    base, first, count, flags = map(int, stream.view('<u4').reshape(-1, 4)[code])
    assert (base, count, flags) == (updates.packed.commands[code]['base'], 3, 0)
    fresh = pack_scene(scene)
    fresh_first = int(fresh.materials.view('<u4').reshape(-1, 4)[0, 1])
    np.testing.assert_array_equal(stream[first*16:first*16+96],
                                  fresh.materials[fresh_first*16:fresh_first*16+96])


def test_adding_annotations_preserves_local_journal():
    from core.dimension import Dimension
    from core.history import AddDimensionCommand, AddTextLabelCommand, History
    from core.textlabel import TextLabel
    scene, updates = fixture_scene()
    history = History(scene)
    updates.observe(history)
    for command in (AddDimensionCommand(Dimension(QVector3D(), QVector3D(2, 0, 0),
                                                  QVector3D(0, 1, 0))),
                    AddTextLabelCommand(TextLabel(QVector3D(), QVector3D(1, 1, 0), "axis"))):
        history.execute(command)
        for step in (lambda: None, history.undo, history.redo):
            step()
            ids, rows = updates.diff(scene)
            assert updates.last_local and not len(ids) and not updates.changed
            updates.apply(ids, rows)


def test_first_loose_edge_only_adds_its_own_row():
    from core.history import AddEdgeCommand, History
    scene, updates = fixture_scene()
    history = History(scene)
    updates.observe(history)
    history.execute(AddEdgeCommand(QVector3D(), QVector3D(1, 0, 0)))
    updates.apply(*updates.diff(scene))
    assert updates.last_local
    assert_matches_fresh_pack(updates, scene)


def test_retyped_line_updates_resident_face_split():
    from core.history import History
    from tools.rust_tools import LineTool

    scene, _ = fixture_scene()
    V = QVector3D
    scene.mesh.add_face([V(), V(4, 0, 0), V(4, 4, 0), V(0, 4, 0)])
    updates = InstanceUpdates(scene, pack_scene(scene))
    history = History(scene)
    updates.observe(history)
    vp = SimpleNamespace(scene=scene, history=history, update=lambda: None)
    line = LineTool()

    def check(faces):
        assert len(scene.mesh.faces) == faces
        updates.apply(*updates.diff(scene))
        assert updates.last_local
        assert_matches_fresh_pack(updates, scene)

    for point in (V(), V(4, 4, 0)):
        line.on_click(ToolContext(vp, point, QPointF(), Qt.NoModifier, None))
    check(2)
    assert line.on_value(vp, 1.0)
    assert len(history.undo_stack) == 1
    check(1)
    assert history.undo()
    check(1)


def test_incremental_edge_distance_ties_use_row_then_edge():
    from views.wgpu_inference import prepare
    scene, updates = fixture_scene()
    scene.groups.append(Group(scene.groups[0].mesh))
    packed = pack_scene(scene)
    index = prepare(packed)
    # An edit to a third placement still puts a two-row query in incremental
    # mode. The untouched BVH orders ties by the supplied candidate order.
    index.update([2], packed)
    for edited in ([], [0]):
        index.update(edited, packed)
        rows, edges, _, _, distances = index.edges_near(
            [1, 0], QMatrix4x4().data(), 100., 100., 50., 50., 300., 32)
        ranked = list(zip(distances.tolist(), rows.tolist(), edges.tolist()))
        assert set(rows) == {0, 1}
        assert ranked == sorted(ranked)
