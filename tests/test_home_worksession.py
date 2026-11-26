"""Home and project sessions exercise real tabs with isolated test settings."""
from tests.raster_samples import solid_raster
from pathlib import Path
import json
import threading
from tests.shell_isolation import native_host, restore_shell_patches  # noqa: F401
import pytest
from built_core import Input as Qt
from built_core import Vec3 as QVector3D
from built_core import project
from built_core import recent
from built_core.scene import Scene
from built_core.history import AddEdgeCommand
from formats import igz
from formats import project as storage


@pytest.fixture
def win(native_host):
    return native_host.window


def make_project(tmp_path):
    path = tmp_path / 'job.builtproject'
    project.create(Scene(), path)
    value = storage.load(path)
    for name, role in [('a', 'host'), ('b', 'host'), ('source', 'linked')]:
        igz.save_scene(Scene(), tmp_path / (name + '.igz'))
        value['files'].append(dict(path=name + '.igz', role=role))
    storage.save(path, value)
    return path


def test_portable_roundtrip(tmp_path):
    path = make_project(tmp_path)
    a, b = tmp_path / 'a.igz', tmp_path / 'b.igz'
    project.save_worksession(path, [dict(path=b, camera={'distance': 7}), dict(path=a)], a)
    state = project.worksession(path)
    assert [r['path'] for r in state['files']] == [str(b), str(a)]
    assert state['files'][0]['camera'] == {'distance': 7}
    assert state['active_file'] == str(a)
    # This person's session file, never the shared manifest.
    assert 'worksession' not in storage.load(path)
    assert project.worksession(path)['active_file'] == str(a)


def test_recent_bounded_pin_and_relocation(tmp_path):
    from built_core import NativeSettings
    st = NativeSettings(str(tmp_path))
    first = tmp_path / 'first.igz'
    recent.remember(first, pinned=True, settings=st)
    for n in range(30):
        recent.remember(tmp_path / f'{n}.igz', settings=st)
    assert len(recent.entries(settings=st)) == recent.LIMIT
    assert recent.entries(settings=st)[0]['path'] == str(first)
    moved = tmp_path / 'moved.igz'
    recent.remember(moved, replace=str(first), settings=st)
    assert recent.entries(settings=st)[0] == dict(path=str(moved), pinned=True)


def test_home_lists_opens_and_locates(win, tmp_path, monkeypatch):
    from built_render import wgpu_start
    from built_render.shell_files import ShellFileDialog
    a = tmp_path / 'moved.igz'
    igz.save_scene(Scene(), a)
    old = tmp_path / 'old.igz'
    recent.remember(old)
    recent.remember(make_project(tmp_path))
    wgpu_start.show(win)
    rows = wgpu_start.page(win).sheet.items(False)
    assert any(row[2] == str(old) for key in ("missing", "recent") for row in rows[key])
    wgpu_start.page(win).sheet.act(None, "tab", ["projects"])
    rows = wgpu_start.page(win).sheet.items(False)
    assert any(row[2] == str(tmp_path / "job.builtproject") for row in rows["projects"])
    monkeypatch.setattr(ShellFileDialog, 'getOpenFileName', lambda *a: (str(tmp_path / 'moved.igz'), ''))
    assert wgpu_start.open_recent(win, old, locate=True)
    assert win._doc.path == a
    assert str(old) not in [r['path'] for r in recent.entries()]


def test_active_file_tabs_and_reopen(win, tmp_path):
    path = make_project(tmp_path)
    assert win.open_project(path)
    assert len(win.documents()) == 2
    a, b = win.documents()
    assert not a.read_only and b.read_only
    win.activate_document(b)
    before = b.viewport.scene.content_version
    b.viewport.history.execute(AddEdgeCommand(QVector3D(), QVector3D(1, 0, 0)))
    assert b.viewport.scene.content_version == before
    win.set_active_file(tmp_path / 'source.igz')
    assert all(d.read_only for d in (a, b))
    source = win._doc
    source.viewport.history.execute(AddEdgeCommand(QVector3D(), QVector3D(1, 0, 0)))
    assert source.viewport.history.undo_stack
    from built_render import shell_files as files
    assert files.do_save(win, source.path)
    assert next(e for e in storage.load(path)['files'] if e['path'] == 'source.igz')['role'] == 'linked'
    win._documents.remove(source)
    win._documents.insert(0, source)
    win.persist_worksession()
    session_file = Path(win.query_worksession()['session_file'])
    assert json.loads(session_file.read_text())['tabs'][0]['path'] == 'source.igz'
    win.open_project(path)
    assert win.documents()[0] is source
    assert win.query_worksession()["active_file"] == str(source.path)


def test_agent_tools_and_read_only_rejection(win, tmp_path):
    from plugins.ai_bridge import _Bridge
    bridge = _Bridge(win.viewport)
    win.viewport.scene._agent_access = True
    def call(tool, **args):
        job = {'req': {'tool': tool, 'args': args}, 'done': threading.Event()}
        bridge._run_on_main(job)
        return job['reply']
    path = make_project(tmp_path)
    assert call('open_project', path=str(path))['ok']
    # Access is per model: each of the project's files is allowed.
    for vp in win.all_viewports():
        vp.scene._agent_access = True
    assert call('query_worksession')['result']['active_file'].endswith('a.igz')
    win.activate_document(win.documents()[1])
    assert not call('run_python', code='scene.clear()')['ok']
    reply = call('set_active_file', path=str(tmp_path / 'b.igz'))
    assert reply['ok'], reply
    assert call('query_recent')['result']['projects']


def test_thumbnail_is_additive_and_survives_resave(tmp_path):
    import zipfile
    data = solid_raster(32, 20, (255, 255, 255)).png()
    scene = Scene()
    scene._home_thumbnail_png = bytes(data)
    path = tmp_path / 'preview.igz'
    igz.save_scene(scene, path)
    loaded = Scene()
    igz.load_into(loaded, path)
    igz.save_scene(loaded, path)
    with zipfile.ZipFile(path) as archive:
        assert archive.read('thumbnail.png') == bytes(data)


def test_missing_project_member_keeps_the_available_member_open(win, tmp_path):
    path = make_project(tmp_path)
    (tmp_path / 'a.igz').unlink()
    assert win.open_project(path)
    members = {Path(f['path']).name: f['state'] for f in win.query_worksession()['files']}
    assert members['a.igz'] == 'missing'
    assert win._doc.path.name == 'b.igz'


def test_new_document_can_be_saved_into_session(win, tmp_path):
    path = make_project(tmp_path)
    win.open_project(path)
    win._on_new()
    assert not win._doc.read_only
    assert all(d.read_only for d in win.documents() if d is not win._doc)
    new_path = tmp_path / 'new.igz'
    from built_render import shell_files as files
    assert files.do_save(win, new_path)
    assert win.query_worksession()["active_file"] == str(new_path)
    assert new_path.is_file()
    session_file = Path(win.query_worksession()['session_file'])
    assert json.loads(session_file.read_text())['editable'] == new_path.name
