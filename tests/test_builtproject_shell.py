"""A Built project in the native shell, headless (docs/builtproject-format.md
§6): open, a missing member as a row and a problem, the editable file, the
per-user session restored. Synthetic files only (tests/fixtures/shell).
The windowed run is tests/test_builtproject_native.py."""
import json
from pathlib import Path

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401  (autouse)

ROOT = Path(__file__).resolve().parents[1]
SLAB = ROOT / "tests" / "fixtures" / "shell" / "slab-and-cube.igz"


@pytest.fixture
def host(tmp_path):
    from app_shell.host import Host
    h = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
    yield h
    for d in h.window._documents:
        d.saved_content = d.viewport.scene.content_version if d.viewport else None
    h.close()


def three_member_project(folder, legacy=False):
    """a.igz and b.igz on disk, gone.igz listed but missing."""
    import ingetrazo_core as native
    folder.mkdir(parents=True, exist_ok=True)
    for name in ("a.igz", "b.igz"):
        (folder / name).write_bytes(SLAB.read_bytes())
    files = [{"path": n, "role": "host"} for n in ("a.igz", "b.igz", "gone.igz")]
    if legacy:
        from core import bim_registry
        path = folder / "Job.igzproj"
        path.write_text(json.dumps({
            "version": 1, "bim_registry": bim_registry.seed_defaults(),
            "ifc_identity": {"namespace": "ca2c12df23ac48f9a103455660d950e5"},
            "files": files, "links": []}))
        return path
    path = folder / "Job.builtproject"
    native.project_manifest_new(path, "{}", json.dumps({"namespace": "ca2c12df23ac48f9a103455660d950e5"}))
    value = json.loads(path.read_text())
    value["files"] = files
    native.project_manifest_save(path, json.dumps(value))
    return path


def names(win):
    return [d.display_name() for d in win._documents]


def test_open_with_a_missing_member_never_asks(host, tmp_path, monkeypatch):
    from views import chrome_dialogs as cd
    asked = []
    monkeypatch.setattr(cd, "question", lambda *a, **k: asked.append(a) or 0)
    monkeypatch.setattr(cd, "warning", lambda *a, **k: asked.append(a))
    path = three_member_project(tmp_path / "job")
    assert host.open_path(path)
    win = host.window
    assert asked == []
    assert names(win) == ["a.igz", "b.igz"]
    state = win.query_worksession()
    by_name = {Path(f["path"]).name: f for f in state["files"]}
    assert by_name["gone.igz"]["state"] == "missing"
    assert by_name["a.igz"]["state"] == "open" and not by_name["a.igz"]["read_only"]
    assert by_name["b.igz"]["read_only"]
    assert [Path(p["path"]).name for p in state["problems"]] == ["gone.igz"]
    a, b = win._documents
    assert not a.read_only and b.read_only and b.viewport.scene.worksession_read_only
    # Problems and the Project tree carry it.
    from app_shell import project
    assert [r[1] for r in project.problems(win)] == ["project:" + str((tmp_path / "job" / "gone.igz").resolve())]
    rows = project.tree_rows(win, "", set())
    assert {r[0][1]: r[0][4] for r in rows[1:]}["gone.igz"] == "file:missing"


def test_read_only_member_rejects_edits(host, tmp_path):
    from core.history import AddEdgeCommand
    from ingetrazo_core import Vec3
    host.open_path(three_member_project(tmp_path / "job"))
    win = host.window
    b = win._documents[1]
    win.activate_document(b)
    before = b.viewport.scene.content_version
    b.viewport.history.execute(AddEdgeCommand(Vec3(0, 0, 0), Vec3(1, 0, 0)))
    assert b.viewport.scene.content_version == before
    win.set_active_file(b.path)
    assert b.read_only is False and win._documents[0].read_only
    b.viewport.history.execute(AddEdgeCommand(Vec3(0, 0, 0), Vec3(1, 0, 0)))
    assert b.viewport.scene.content_version != before


def test_session_restores_order_editable_and_camera(tmp_path):
    from app_shell import files
    from app_shell.host import Host
    path = three_member_project(tmp_path / "job")
    host = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
    try:
        win = host.window
        host.open_path(path)
        a, b = win._documents
        win.set_active_file(b.path)
        win._documents.reverse()                     # the tabs reordered
        win.activate_document(a)
        a.viewport.camera.distance = 42.5
        win.persist_worksession()
        session_file = Path(json.loads(win._project.query())["session_file"])
        # The shell's profile (started once per process), never Application Support.
        assert session_file.is_relative_to(Path(host.settings_dir))
        saved = json.loads(session_file.read_text())
        assert [t["path"] for t in saved["tabs"]] == ["b.igz", "a.igz", "gone.igz"]
        assert saved["editable"] == "b.igz"
        assert files.request_close(win)               # quitting keeps the tabs
    finally:
        host.close()
    assert [t["path"] for t in json.loads(session_file.read_text())["tabs"]] == ["b.igz", "a.igz", "gone.igz"]
    host = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
    try:
        win = host.window
        host.open_path(path)
        assert names(win) == ["b.igz", "a.igz"]
        assert win._doc.display_name() == "b.igz" and not win._doc.read_only
        a = win._documents[1]
        win.activate_document(a)
        assert a.viewport.camera.distance == pytest.approx(42.5)
        # Closing every tab, then opening again, opens the hosts.
        for doc in list(win._documents):
            win.close_document(doc)
        host.open_path(path)
        assert names(win) == ["a.igz", "b.igz"]
    finally:
        host.close()


def test_opening_an_igzproj_converts_it_in_place(host, tmp_path):
    path = three_member_project(tmp_path / "job", legacy=True)
    model = (tmp_path / "job" / "a.igz").read_bytes()
    assert host.open_path(path)
    assert not path.exists() and path.with_suffix(".builtproject").is_file()
    assert (tmp_path / "job" / "a.igz").read_bytes() == model
    from core import recent
    rows = [r["path"] for r in recent.entries("projects")]
    assert str(path.with_suffix(".builtproject").resolve()) in rows and str(path.resolve()) not in rows


def test_open_path_no_longer_hands_projects_to_classic(host, tmp_path, monkeypatch):
    from app_shell import files
    monkeypatch.setattr(files, "open_in_classic", lambda *a, **k: pytest.fail("handed to Classic"))
    assert files.open_path(host.window, three_member_project(tmp_path / "job"))


def test_locate_a_missing_member_relocates_it_in_the_manifest(host, tmp_path):
    from app_shell import project
    path = three_member_project(tmp_path / "job")
    found = tmp_path / "job" / "moved" / "gone.igz"
    found.parent.mkdir()
    found.write_bytes(SLAB.read_bytes())
    host.open_path(path)
    win = host.window
    project.locate(win, tmp_path / "job" / "gone.igz", found)
    files = [f["path"] for f in json.loads(path.read_text())["files"]]
    assert files == ["a.igz", "b.igz", "moved/gone.igz"]
    assert sorted(names(win)) == ["a.igz", "b.igz", "gone.igz"]   # beside the tab in front
    state = win.query_worksession()
    assert state["problems"] == []
    tabs = {f["path"]: f for f in state["files"] if f["tab"]}
    assert str(found.resolve()) in tabs and tabs[str(found.resolve())]["state"] == "open"
    assert len(tabs) == 3

