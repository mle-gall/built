"""The native app's daily paths (S1.1 shell parity), headless.

No surface, so no renderer: the sheet cannot be drawn and its questions
are answered here by replacing ``chrome_dialogs.ask`` / ``question`` (the
windowed path, the modal pump answering a real key, is exercised by
``built --script`` with ``BUILT_ANSWERS``).
Every file is synthetic or from tests/fixtures; settings go to tmp_path.
"""
from pathlib import Path

import pytest
from built_core import StandardButton
from built_core import Vec3 as QVector3D
from tests.shell_isolation import restore_shell_patches  # noqa: F401  (autouse)

FIXTURES = Path(__file__).parent / "fixtures" / "shell"
SLAB = FIXTURES / "slab-and-cube.igz"
DATA = Path(__file__).parent / "data"


@pytest.fixture
def host(tmp_path):
    from built_render import Host
    h = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
    yield h
    h.close()


@pytest.fixture
def answers(monkeypatch):
    """Answer the sheet: ``answers.append("discard")`` for the next ask."""
    from built_render import chrome_dialogs as cd

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
        return StandardButton.Yes if got in ("yes", "save") else StandardButton.No

    monkeypatch.setattr(cd, "ask", ask)
    monkeypatch.setattr(cd, "question", question)
    queue.asked = asked
    return queue


def _draw(host):
    from built_core.history import AddFaceCommand
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
    from built_render import shell_files as files
    from formats import built
    assert built.is_built(target)
    assert str(target.resolve()) in [str(Path(p).resolve()) for p in host.window._recent_paths()]
    assert files  # the module is the one the menu runs


def test_save_of_an_igz_asks_where_through_the_native_panel(host, tmp_path, monkeypatch):
    from built_render import shell_services as native
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
    assert out.is_file() and answers.asked == ["Save the rounded copy?"]
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
    from built_core import autosave
    from built_core.scene import Scene
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
    assert answers.asked == ["Recover the auto-saved drawing?"]
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
    assert host.window.tab_title(0).endswith(" •")
    assert not host.window.tab_title(1).endswith(" •")
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
    assert answers.asked == ["What unit is this drawing in?"]
    assert "plan.dxf" in host.state()["status"]


# ---- linked files ----------------------------------------------------------------------
def test_composite_with_nested_links_opens_and_reloads(host, tmp_path):
    from tests.fixtures.shell.make_composite import make
    from built_core import links
    paths = make(tmp_path / "composite")
    assert host.open_path(paths["host"])
    scene = host.viewport.scene
    assert {Path(p).name for p in links.sources(scene)} == {"mid.built", "leaf.built"}
    assert not getattr(scene, "link_errors", None)
    host.menu("reload_links")
    assert host.state()["status"] == "Linked documents reloaded" and host.errors == []


def test_open_dialog_goes_through_the_native_panel(host, tmp_path, monkeypatch):
    from built_render import shell_services as native
    src = _copy(tmp_path)
    monkeypatch.setattr(native, "open_files", lambda title, filters=None, directory=None, multiple=False: [str(src)])
    assert host.menu("open")
    assert host.state()["path"] == str(src)


def test_right_click_builds_the_menu_for_what_was_hit(host, tmp_path, monkeypatch):
    """The native app's right-click menu (docs/context-menu.md): what is
    under the pointer is selected first, and the menu is that kind's. The
    per-kind menus are tests/test_shell_tabs_menu.py's."""
    from built_render import selection_kind
    host.open_path(_copy(tmp_path))
    from tests.test_shell_tabs_menu import StubRenderer
    monkeypatch.setattr(host.view, "renderer", StubRenderer())
    monkeypatch.setattr(host.view, "_ui_attached", True, raising=False)
    monkeypatch.setattr(host.view, "pick_row", lambda x, y: None)
    host.pointer("press", 600, 400, 2, 2, 0)
    host.pointer("release", 600, 400, 2, 0, 0)
    menu = host.view._ctx_menu
    scene = host.viewport.scene
    assert menu.kind == selection_kind(scene, scene.selection)
    assert menu.labels()[0].startswith("Repeat")
    assert host.errors == []


def test_native_timer_wakes_the_loop(host):
    fired = []
    host.timers.later(host.window, 250, lambda: fired.append(True))
    _, delay, *_ = host.idle()
    assert delay is not None and 0 <= delay <= 250
    assert not fired


def test_menu_edit_commands(host, tmp_path):
    host.open_path(_copy(tmp_path))
    host.menu("select_all")
    assert host.state()["selection"] > 0
    host.menu("copy")
    assert host.state()["clipboard"]
    host.menu("select_none")
    assert host.state()["selection"] == 0


def test_selection_commands_have_a_native_owner(host):
    from built_render import ShellWindow
    for verb in ("select_all", "invert_selection", "select_none"):
        host.menu(verb)
    assert host.window._on_select_all.__func__ is ShellWindow._on_select_all
    with pytest.raises(AttributeError):
        host.window.no_such_member_anywhere  # noqa: B018


def test_background_autosave_writes_a_recovery_slot(host, tmp_path):
    """The native clock schedules recovery. The retained writer slices finish on the UI thread."""
    import time
    from built_render import WindowTimer
    from built_core import autosave
    src = _copy(tmp_path)
    host.open_path(src)
    _draw(host)
    host.window._setup_autosave()
    timer = host.window._autosave_timer
    assert isinstance(timer, WindowTimer)
    timer.start(0)
    host.idle()
    timer.stop()
    deadline = time.monotonic() + 20
    while autosave.pending(src) is None and time.monotonic() < deadline:
        host.idle()
        time.sleep(0.02)
    assert autosave.pending(src) is not None


def test_window_timers_follow_their_owner_and_nested_turns(host):
    from built_render import WindowTimers

    events = []

    class Handler:
        def first(self):
            events.append("first")
            assert not getattr(self, "inside", False)
            self.inside = True
            self.first_timer.start(0)
            self.second_timer.start(0)
            other.run_due()
            host.modal_frame()
            self.first_timer.stop()
            self.inside = False

        def second(self):
            events.append("second")

    handler = Handler()
    handler.first_timer = host.timers.timer(handler.first, repeat=True)
    handler.second_timer = host.timers.timer(handler.second)
    other = WindowTimers()
    handler.first_timer.start(0)
    other.adopt(handler.first_timer)
    host.timers.run_due()
    assert events == []
    assert other.next_ms() == 0
    other.run_due()
    assert events == ["first", "second"]
    assert other.next_ms() is None
    assert not handler.second_timer.isActive()

    handler.first_timer.start(0)
    host.timers.adopt(handler.first_timer)
    other.cancel_all()
    host.timers.run_due()
    assert events == ["first", "second", "first", "second"]


def test_native_view_timers_release_callbacks_on_close(host):
    import gc
    import weakref
    from built_render import WindowTimer

    view = host.view
    assert isinstance(view._settle_timer, WindowTimer)
    assert isinstance(view._resize_timer, WindowTimer)
    view._ui_timer = view._view_timer(view.update)
    for timer in view._presentation_timers():
        timer.start(60_000)
    view.closeEvent()
    assert all(not timer.isActive() for timer in view._presentation_timers())

    class Owner:
        def tick(self):
            pytest.fail("cancelled owner received a tick")

    owner = Owner()
    ref = weakref.ref(owner)
    timer = host.timers.timer(owner.tick, repeat=True)
    timer.start(0)
    del owner
    gc.collect()
    assert ref() is None
    host.timers.run_due()
    assert not timer.isActive()


def test_cancelling_named_recovery_keeps_the_slot_and_active_tab(host, tmp_path, answers):
    from built_core import autosave
    from formats import igz
    from built_core.scene import Scene
    source = _copy(tmp_path)
    scene = Scene()
    igz.load_into(scene, source)
    slot = autosave.write(scene, source)
    before = host.window.active_document()
    answers.append("later")
    assert not host.open_path(source)
    assert host.window.active_document() is before
    assert slot.is_file() and before.path is None


def test_a_bad_untitled_slot_preserves_the_open_drawing(host, tmp_path, answers, monkeypatch):
    from built_render import chrome_dialogs
    win = host.window
    _draw(host)
    original = win.active_document()
    revision = win.viewport.history.revision
    bad = tmp_path / "untitled-3.built"
    bad.write_bytes(b"broken archive")
    errors = []
    monkeypatch.setattr(chrome_dialogs, "critical", lambda win, title, msg: errors.append(msg))
    answers.append("yes")
    win._recover_untitled(3, bad)
    assert errors and bad.is_file()
    assert win.documents() == [original] and win.active_document() is original
    assert win.viewport.history.revision == revision


def test_a_read_only_project_rejects_an_import_before_reading_the_file(host, tmp_path, monkeypatch):
    from built_render import chrome_dialogs
    from formats import dae
    win = host.window
    vp = win.viewport
    vp.scene.worksession_read_only = True
    before = vp.history.revision
    errors = []
    monkeypatch.setattr(chrome_dialogs, "critical", lambda win, title, msg: errors.append(msg))
    monkeypatch.setattr(dae, "load_dae", lambda *a, **kw: pytest.fail("read-only import ran"))
    assert not win._import_dae_path(tmp_path / "synthetic.dae")
    assert errors and "read-only" in errors[0]
    assert vp.history.revision == before and not win._is_dirty()
