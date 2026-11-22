"""The native app's daily paths (S1.1 shell parity), headless.

No surface, so no renderer: the sheet cannot be drawn and its questions
are answered here by replacing ``chrome_dialogs.ask`` / ``question`` (the
windowed path, the modal pump answering a real key, is exercised by
``built --script`` with ``BUILT_ANSWERS``; see docs/rust-app-shell.md).
Every file is synthetic or from tests/fixtures; settings go to tmp_path.
"""
from pathlib import Path

import pytest
from PySide6.QtCore import QTimer
from PySide6.QtGui import QVector3D
from PySide6.QtWidgets import QMessageBox

FIXTURES = Path(__file__).parent / "fixtures" / "shell"
SLAB = FIXTURES / "slab-and-cube.igz"
DATA = Path(__file__).parent / "data"


@pytest.fixture
def host(tmp_path):
    from app_shell.host import Host
    h = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
    yield h
    h.close()


@pytest.fixture
def answers(monkeypatch):
    """Answer the sheet: ``answers.append("discard")`` for the next ask."""
    from views import chrome_dialogs as cd

    class Answers(list):
        asked: list

    queue = Answers()
    asked = []

    def ask(parent, title, text, buttons, **kw):
        asked.append(title)
        return queue.pop(0) if queue else kw.get("cancel")

    def question(parent, title, text, buttons=None, defaultButton=None):
        asked.append(title)
        got = queue.pop(0) if queue else "no"
        return QMessageBox.Yes if got in ("yes", "save") else QMessageBox.No

    monkeypatch.setattr(cd, "ask", ask)
    monkeypatch.setattr(cd, "question", question)
    queue.asked = asked
    return queue


def _draw(host):
    from core.history import AddFaceCommand
    loop = [QVector3D(5, 0, 0), QVector3D(6, 0, 0), QVector3D(6, 1, 0)]
    host.viewport.history.execute(AddFaceCommand(loop))


def _copy(tmp_path, name="model.igz"):
    dst = tmp_path / name
    dst.write_bytes(SLAB.read_bytes())
    return dst


# ---- save -------------------------------------------------------------------------
def test_save_as_built_clears_dirty_and_names_the_window(host, tmp_path):
    host.open_path(_copy(tmp_path))
    _draw(host)
    assert host.state()["dirty"]
    target = tmp_path / "saved.built"
    assert host.menu(f"save_to:{target}")
    st = host.state()
    assert target.is_file() and st["path"] == str(target) and not st["dirty"]
    assert st["documents"] == ["saved.built"]
    from app_shell import files
    from formats import built
    assert built.is_built(target)
    assert str(target.resolve()) in [str(Path(p).resolve()) for p in host.window._recent_paths()]
    assert files  # the module is the one the menu runs


def test_save_of_an_igz_asks_where_through_the_native_panel(host, tmp_path, monkeypatch):
    from app_shell import native
    src = _copy(tmp_path)
    host.open_path(src)
    _draw(host)
    chosen = tmp_path / "chosen"            # no suffix: .built is added
    seen = {}

    def save_file(title, name="", filters=None, directory=None):
        seen.update(title=title, name=name, filters=filters, directory=directory)
        return str(chosen)
    monkeypatch.setattr(native, "save_file", save_file)
    host.menu("save")
    assert (tmp_path / "chosen.built").is_file()
    assert seen["name"] == "model.built" and seen["directory"] == str(tmp_path)
    assert not host.state()["dirty"]


def test_save_a_copy_keeps_the_document_and_its_unsaved_state(host, tmp_path):
    src = _copy(tmp_path)
    host.open_path(src)
    _draw(host)
    copy = tmp_path / "copy.built"
    assert host.menu(f"save_copy_to:{copy}")
    st = host.state()
    assert copy.is_file() and st["path"] == str(src) and st["dirty"]


def test_igz_export_asks_about_float32_and_writes_a_copy(host, tmp_path, answers, monkeypatch):
    from formats import igz
    host.open_path(_copy(tmp_path))
    real = igz.export_igz

    def export(scene, path, confirm):
        assert confirm(0.012)               # the question is asked, answered yes
        return real(scene, path, lambda mm: True)
    monkeypatch.setattr(igz, "export_igz", export)
    answers.append("yes")
    out = tmp_path / "out.igz"
    assert host.menu(f"export_igz_to:{out}")
    assert out.is_file() and answers.asked == ["Export precision"]
    assert host.state()["path"].endswith("model.igz")


# ---- close, quit, recover ------------------------------------------------------------
def test_close_asks_about_unsaved_changes_and_cancel_keeps_it(host, tmp_path, answers):
    host.open_path(_copy(tmp_path))
    _draw(host)
    answers.append("cancel")
    assert not host.menu("close_tab")
    assert host.state()["documents"] == ["model.igz"]
    answers.append("discard")
    assert host.menu("close_tab")
    st = host.state()
    assert st["documents"] == ["Untitled"] and st["start_page"]
    assert answers.asked[0].startswith("Close")


def test_quit_with_unsaved_work_asks_once_and_save_all_saves(host, tmp_path, answers):
    a = tmp_path / "a.built"
    host.open_path(_copy(tmp_path))
    host.menu(f"save_to:{a}")
    _draw(host)
    answers.append("cancel")
    assert not host.request_close()
    answers.append("save")
    assert host.request_close()
    assert not host.window._doc_is_dirty(host.window._doc)
    assert host.window._closed


def test_open_offers_the_auto_saved_copy(host, tmp_path, answers):
    from core import autosave
    from core.scene import Scene
    from formats import igz
    src = _copy(tmp_path)
    scene = Scene()
    igz.load_into(scene, src)
    scene.mesh.add_face([QVector3D(9, 0, 0), QVector3D(10, 0, 0), QVector3D(10, 1, 0)])
    autosave.write(scene, src)
    assert autosave.pending(src) is not None
    answers.append("yes")
    assert host.open_path(src)
    st = host.state()
    assert answers.asked == ["Recovered drawing"]
    assert st["faces"] == 7 and st["dirty"]         # recovered ≠ saved


def test_cold_start_shows_the_start_page_and_new_hides_it(host):
    host.cold_start()
    assert host.state()["start_page"]
    host.menu("new")
    st = host.state()
    assert not st["start_page"] and st["documents"] == ["Untitled"] and not st["dirty"]


# ---- several documents behind document ▾ ------------------------------------------------
def test_documents_open_side_by_side_and_switch(host, tmp_path):
    a, b = _copy(tmp_path, "a.igz"), _copy(tmp_path, "b.igz")
    host.open_path(a)
    _draw(host)
    host.open_path(b)
    st = host.state()
    assert st["documents"] == ["a.igz", "b.igz"] and st["active"] == 1
    tabs = host.window._project_tabs
    assert tabs.tabs.tabText(0).endswith(tabs.DIRTY_MARK) and not tabs.tabs.tabText(1).endswith(tabs.DIRTY_MARK)
    host.open_path(a)                                # already open: shown, not read again
    assert host.state()["active"] == 0 and len(host.window._documents) == 2
    assert host.view is host.window._doc.stand_in and host.viewport is host.window._doc.viewport
    host.menu("next_doc")
    assert host.state()["active"] == 1
    from views import wgpu_ui
    wgpu_ui._project_action(host.window, "0")        # the document ▾ row
    assert host.state()["active"] == 0


# ---- imports ---------------------------------------------------------------------------
@pytest.mark.parametrize("name", ["test_export.dae", "skp_instances_2021.skp"])
def test_imports_land_through_the_pipelines(host, tmp_path, name):
    src = DATA / name
    if not src.exists():
        pytest.skip(f"{name} fixture missing")
    assert host.open_path(src)
    st = host.state()
    assert st["groups"] >= 1 and st["documents"] == [name] and st["errors"] == 0


def test_dxf_asks_its_unit_on_the_sheet(host, tmp_path, answers):
    dxf = tmp_path / "plan.dxf"
    ents = []
    for x1, y1, x2, y2 in [(0, 0, 4, 0), (4, 0, 4, 3), (4, 3, 0, 3), (0, 3, 0, 0)]:
        ents += ["0", "LINE", "8", "0", "10", str(x1), "20", str(y1), "30", "0",
                 "11", str(x2), "21", str(y2), "31", "0"]
    dxf.write_text("\n".join(["0", "SECTION", "2", "ENTITIES", *ents, "0", "ENDSEC", "0", "EOF"]) + "\n")
    answers.append("m")
    assert host.open_path(dxf)
    assert answers.asked == ["Import DXF"]
    assert "plan.dxf" in host.state()["status"]


# ---- linked files ----------------------------------------------------------------------
def test_composite_with_nested_links_opens_and_reloads(host, tmp_path):
    from tests.fixtures.shell.make_composite import make
    from core import links
    paths = make(tmp_path / "composite")
    assert host.open_path(paths["host"])
    scene = host.viewport.scene
    assert {Path(p).name for p in links.sources(scene)} == {"mid.built", "leaf.built"}
    assert not getattr(scene, "link_errors", None)
    host.menu("reload_links")
    assert host.state()["status"] == "Linked documents reloaded" and host.errors == []


# ---- the classic app, dialogs, widgets ---------------------------------------------------
def test_classic_app_command_opens_the_same_file(host, tmp_path, monkeypatch):
    from app_shell import files
    src = _copy(tmp_path)
    host.open_path(src)
    started = []
    monkeypatch.setattr(files.subprocess, "Popen", lambda cmd, **kw: started.append(cmd))
    assert host.menu("classic")
    cmd = started[0]
    assert cmd[-1] == str(src) and cmd[-2] == "--new-window" and cmd[-3].endswith("main.py")
    monkeypatch.setenv("BUILT_CLASSIC", "/Applications/Built.app/Contents/MacOS/built --qt")
    assert files.classic_command(src) == ["/Applications/Built.app/Contents/MacOS/built", "--qt", str(src)]


def test_open_dialog_goes_through_the_native_panel(host, tmp_path, monkeypatch):
    from app_shell import native
    src = _copy(tmp_path)
    monkeypatch.setattr(native, "open_files", lambda title, filters=None, directory=None, multiple=False: [str(src)])
    assert host.menu("open")
    assert host.state()["path"] == str(src)


def test_a_qt_dialog_the_borrowed_code_shows_is_refused_not_waited_for(host):
    """Offscreen nobody can answer a Qt modal: it is rejected at once and
    reported (the status names the classic app)."""
    got = QMessageBox.question(None, "Some old dialog", "Really?")
    assert got != QMessageBox.Yes
    st = host.state()
    assert st["blocked_widgets"]
    assert "Classic App" in st["status"]


def test_right_click_says_where_the_verbs_are(host, tmp_path):
    host.open_path(_copy(tmp_path))
    host.pointer("press", 600, 400, 2, 2, 0)
    host.pointer("release", 600, 400, 2, 0, 0)
    assert "Classic App" in host.state()["status"]


def test_qt_timers_started_from_python_wake_the_loop(host):
    from app_shell import qt_host
    fired = []
    QTimer.singleShot(250, lambda: fired.append(1))
    left = qt_host.next_timer_ms()
    assert left is not None and left <= 250
    _, delay, *_ = host.idle()
    assert delay is not None and delay <= 250


def test_menu_edit_commands(host, tmp_path):
    host.open_path(_copy(tmp_path))
    host.menu("select_all")
    assert host.state()["selection"] > 0
    host.menu("copy")
    assert host.state()["clipboard"]
    host.menu("select_none")
    assert host.state()["selection"] == 0


def test_mainwindow_members_are_borrowed_on_demand(host):
    from app_shell.window import BORROWED, ShellWindow
    assert "_on_select_all" not in vars(ShellWindow) or "_on_select_all" in BORROWED
    host.window._on_select_all()
    assert "_on_select_all" in BORROWED
    with pytest.raises(AttributeError):
        host.window.no_such_member_anywhere  # noqa: B018
