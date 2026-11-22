"""Link identities belong to source placements, independent of labels and reloads."""
import pytest
from core import links
from core.group import Group, iter_placements
from core.scene import Scene
from formats import igz
from PySide6.QtGui import QMatrix4x4


def source(tmp_path):
    scene = Scene()
    scene.groups = [Group(name='part ' + str(i)) for i in range(5)]
    path = tmp_path / 'parts.igz'
    igz.save_scene(scene, path)
    return scene, path


def test_stable_hosts_subset_and_rename(tmp_path):
    src, path = source(tmp_path)
    host = Scene()
    selected = [g.uid for g in src.groups[:2]]
    group = links.make_link(host, path, select={'uids': selected})
    host.groups.append(group)
    expected = [c.uid for c in group.children]
    assert expected == [links.stable_uid(group.uid, uid) for uid in selected]
    for name in ('one', 'two'):
        file = tmp_path / (name + '.igz')
        igz.save_scene(host, file)
        for _ in range(2):
            loaded = Scene()
            igz.load_into(loaded, file)
            assert [c.uid for c in loaded.groups[0].children] == expected
    src.groups[0].name = 'renamed'
    igz.save_scene(src, path)
    links.reload(host, path)
    assert [c.uid for c in group.children] == expected
    assert group.children[0].name == 'renamed'
    other = links.make_link(host, path, select={'uids': selected})
    assert set(expected).isdisjoint(c.uid for c in other.children)


def test_filter_and_ancestor_transform(tmp_path):
    src, path = source(tmp_path)
    child = src.groups[0]
    parent = Group(name='parent')
    parent.adopt([child])
    parent.xform.translate(7, 0, 0)
    src.groups = [parent]
    igz.save_scene(src, path)
    host = Scene()
    group = links.make_link(host, path, select={'filter': {'facet': 'name', 'eq': child.name}})
    assert len(group.children) == 1
    assert group.children[0].xform.column(3).x() == 7


def test_ride_read_only():
    import os
    from pathlib import Path
    import pytest
    if not os.environ.get('INGETRAZO_TEST_RIDE'):
        pytest.skip('opt-in real read-only fixture')
    path = Path('~/Desktop/models/asbuilt_ride.igz')
    before = path.stat().st_mtime_ns
    rows = []
    for _ in range(2):
        scene = Scene()
        igz.load_into(scene, path)
        assert not scene.link_errors
        rows.append([row['uid'] for row in __import__('core.facets', fromlist=['build']).build(scene).values() if row['linked']])
    assert rows[0] and rows[0] == rows[1]
    assert path.stat().st_mtime_ns == before


def test_portable_synthetic_fixture(tmp_path):
    import shutil
    from pathlib import Path
    fixture = Path(__file__).parent / 'fixtures' / 'links'
    for name in ('five-parts.igz', 'two-parts.igz'):
        shutil.copyfile(fixture / name, tmp_path / name)
    scene = Scene()
    igz.load_into(scene, tmp_path / 'two-parts.igz')
    assert not scene.link_errors
    assert [g.name for g in scene.groups[0].children] == ['Part 0', 'Part 1']


def test_nested_link_policy_per_placement(tmp_path):
    leaf, leaf_path = source(tmp_path)
    middle = Scene()
    parent = Group(name='local assembly')
    parent.adopt([Group(name='local part'), links.make_link(middle, leaf_path)])
    middle.groups = [parent]
    links.set_packed(middle, leaf_path, True)
    middle_path = tmp_path / 'middle.igz'
    igz.save_scene(middle, middle_path)

    host = Scene()
    host.groups = [links.make_link(host, middle_path, include_nested=False),
                   links.make_link(host, middle_path)]
    file = tmp_path / 'host.igz'
    igz.save_scene(host, file)
    igz.load_into(host, file)
    links.reload(host, middle_path)
    local, full = host.groups
    assert [g.name for g in local.children[0].children] == ['local part']
    nested = full.children[0].children[1]
    assert len(nested.children) == len(leaf.groups)
    assert links.link_of(local)['include_nested'] is False
    links.set_include_nested(host, full, False)
    assert [g.name for g in full.children[0].children] == ['local part']
    links.set_include_nested(host, local, True)
    igz.save_scene(host, file)
    igz.load_into(host, file)
    local, full = host.groups
    assert len(local.children[0].children[1].children) == len(leaf.groups)
    assert [g.name for g in full.children[0].children] == ['local part']


def test_source_only_skips_nested_file_before_selecting(tmp_path, monkeypatch):
    _, leaf_path = source(tmp_path)
    middle = Scene()
    link = links.make_link(middle, leaf_path)
    local = Group(name='local')
    middle.groups = [local, link]
    middle_path = tmp_path / 'middle.igz'
    igz.save_scene(middle, middle_path)
    opened = []
    read = igz._load_into_inner

    def record_read(scene, path, progress, link_meshes=None):
        opened.append(path)
        return read(scene, path, progress, link_meshes)

    monkeypatch.setattr(igz, '_load_into_inner', record_read)
    host = Scene()
    placed = links.make_link(host, middle_path, include_nested=False,
                             select={'uids': [link.uid, local.uid]})
    assert opened == [middle_path]
    assert [child.name for child in placed.children] == ['local']


def test_bridge_source_save_updates_one_binary_mesh_in_resident_scene(tmp_path, monkeypatch):
    from types import SimpleNamespace
    from PySide6.QtGui import QVector3D as V
    from core.history import History
    from plugins.ai_bridge import _Bridge
    from tests.test_wgpu_updates import assert_matches_fresh_pack
    from views.wgpu_scene import pack_scene
    from views.wgpu_updates import InstanceUpdates

    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    src = Scene()
    for i in range(300):
        g = Group(name=str(i))
        g.xform = QMatrix4x4()
        g.xform.translate(i, 2, 0)
        g.mesh.add_face([V(), V(1, 0, 0), V(0, 1, 0)])
        src.groups.append(g)
    path = tmp_path / "assembly.igz"
    igz.save_scene(src, path)
    host = Scene()
    placed = links.make_link(host, path)
    host.groups.append(placed)
    old_groups = list(placed.children)
    old_meshes = [g.mesh for g in old_groups]
    host.selection.add(old_groups[1])
    updates = InstanceUpdates(host, pack_scene(host))
    updates.observe(History(host))

    vp = SimpleNamespace(scene=src, history=History(src), notify_scene_changed=lambda: None)
    reply = _Bridge._tool_run_python(SimpleNamespace(_scope={}, _viewport=vp),
                                     'm=groups[17].mesh; m.move_vertex(m.vertices[0], QVector3D(0,0,1))')
    assert reply["changed"] and not reply["error"]
    igz.save_scene(src, path)
    inflated = []
    read = igz._read_geometry_members

    def track(archive, path, payload, skip=()):
        got = read(archive, path, payload, skip)
        inflated.extend(got)
        return got

    monkeypatch.setattr(igz, "_read_geometry_members", track)
    monkeypatch.setattr(links, "_apply_tags", lambda *a: pytest.fail("unchanged tags"))
    links.reload(host, path)
    assert len(inflated) == 1
    assert placed.children == old_groups
    assert [i for i, g in enumerate(placed.children) if g.mesh is not old_meshes[i]] == [17]
    assert old_groups[1] in host.selection
    ids, rows = updates.diff(host)
    assert updates.last_local
    assert ids.tolist() == [17]
    assert len(updates.changed) == 1
    updates.apply(ids, rows)
    assert_matches_fresh_pack(updates, host)
