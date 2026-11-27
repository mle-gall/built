"""Project tabs, windows and the viewport right-click menu (shell design:
tabs option B, the right-click menu with no More ›), headless.

The windowed parts (the strip and the menu drawn by egui, windows made by
winit) run in ``built --script``. Here the same Python the chrome calls is
driven directly, on synthetic files under tmp_path (tests/fixtures/shell)."""
import re
from pathlib import Path

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401  (autouse)

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests" / "fixtures" / "shell"
SLAB = FIXTURES / "slab-and-cube.igz"


@pytest.fixture
def host(tmp_path):
    from built_render import Host
    h = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
    yield h
    h.close()


@pytest.fixture
def second(tmp_path):
    """A second window's host in the same process (Rust makes one per
    winit window)."""
    made = []

    def make():
        from built_render import Host
        h = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
        made.append(h)
        return h
    yield make
    for h in made:
        h.close()


def _copies(tmp_path, n):
    out = []
    for i in range(n):
        p = tmp_path / f"model-{i}.igz"
        p.write_bytes(SLAB.read_bytes())
        out.append(p)
    return out


def _settle(host):
    """The work the loop does after the first picture (links, heap)."""
    host._picture_seen = True
    for _ in range(8):
        host.idle()


class StubRenderer:
    """What the chrome is sent, without a GPU."""

    def __init__(self):
        self.tabs = None
        self.menu = None
        self.icons = []

    def ui_set_project_tabs(self, tabs, elsewhere=(), can_move=False, pictures=True):
        self.tabs = (list(tabs), list(elsewhere), can_move, pictures)

    def ui_set_icons(self, icons):
        self.icons.extend(icons)

    def ui_set_context_menu(self, x, y, title, subtitle, rows):
        self.menu = (x, y, title, subtitle, rows)

    def ui_over_chrome(self, x, y):
        return False

    def render(self):
        return 0.0, 0.0

    def ui_take_actions(self):
        return []

    def ui_repaint_after_ms(self):
        return None

    def __getattr__(self, name):
        # Everything else the view asks of a renderer: nothing to draw.
        if name.startswith("__"):
            raise AttributeError(name)
        return lambda *a, **k: None


# ---- the tab strip ---------------------------------------------------------------------
def test_tab_strip_lists_the_projects_with_front_and_unsaved(host, tmp_path):
    from views import wgpu_tabs
    paths = _copies(tmp_path, 4)
    for p in paths:
        assert host.open_path(p)
    win = host.window
    from built_core.history import AddFaceCommand
    from built_core import Vec3 as V
    host.viewport.history.execute(AddFaceCommand([V(5, 0, 0), V(6, 0, 0), V(6, 1, 0)]))
    tabs = wgpu_tabs.tabs(win)
    assert [t[1] for t in tabs] == [p.name for p in paths]
    assert [t[2] for t in tabs] == [False, False, False, True]          # front
    assert [t[3] for t in tabs] == [False, False, False, True]          # unsaved
    assert not any(t[4] for t in tabs)                                  # no link source
    assert tabs[0][5].endswith(f"{paths[0].name} · " + tabs[0][5].split(" · ")[-1])
    # What the chrome is sent: the tabs, nothing elsewhere, Move offered.
    view = host.view
    view.renderer = StubRenderer()
    try:
        wgpu_tabs.document_tabs(view).sync(view.renderer, win, 86.0)
        sent, elsewhere, can_move, pictures = view.renderer.tabs
        assert [t[1] for t in sent] == [p.name for p in paths]
        assert elsewhere == [] and can_move and pictures
    finally:
        view.renderer = None


class OwningRenderer(StubRenderer):
    """A stand-in that keeps its own DocumentTabs, as the renderer does."""

    def __init__(self):
        super().__init__()
        from built_render import DocumentTabs
        self._document_tabs = DocumentTabs()
        self.sent = []
        self.projects = []

    def document_tabs(self):
        return self._document_tabs

    def ui_set_project_tabs(self, tabs, elsewhere=(), can_move=False, pictures=True):
        super().ui_set_project_tabs(tabs, elsewhere, can_move, pictures)
        self.sent.append([t[2] for t in tabs])

    def ui_set_projects(self, left, docs):
        self.projects.append([d[1] for d in docs])


def test_a_view_handed_the_renderer_back_resends_what_it_shows(host, tmp_path):
    """Review finding 5: A, then B, then A again with one renderer. The
    record of what was sent is the renderer's, so A's tabs go out again
    although A's view sent the same ones before."""
    from types import SimpleNamespace
    from views import wgpu_tabs
    paths = _copies(tmp_path, 2)
    for p in paths:
        assert host.open_path(p)
    win = host.window
    renderer = OwningRenderer()
    assert len(win._documents) == 2
    views = [SimpleNamespace(renderer=renderer) for _ in win._documents]
    a, b = win._documents
    for doc, view in ((a, views[0]), (b, views[1]), (a, views[0])):
        win.activate_document(doc)
        wgpu_tabs.document_tabs(view).sync(view.renderer, win, 86.0)
    assert renderer.sent == [[True, False], [False, True], [True, False]]
    assert renderer.projects == [[True, False], [False, True], [True, False]]
    # Unchanged, nothing more is sent until the tips' next refresh.
    wgpu_tabs.document_tabs(views[0]).sync(renderer, win, 86.0)
    assert len(renderer.sent) == 3 and len(renderer.projects) == 3


def test_tab_clicks_activate_close_and_the_tab_menu(host, tmp_path):
    from views import wgpu_tabs
    paths = _copies(tmp_path, 4)
    for p in paths:
        host.open_path(p)
    win, view = host.window, host.view
    wgpu_tabs.action(win, view, "activate:1")
    assert host.state()["active"] == 1
    wgpu_tabs.action(win, view, "close:3")
    assert host.state()["documents"] == [p.name for p in paths[:3]]
    wgpu_tabs.action(win, view, "menu:close_right:0")
    assert host.state()["documents"] == [paths[0].name]
    wgpu_tabs.action(win, view, "new")
    assert len(host.state()["documents"]) == 2


def test_ctrl_tab_walks_the_strip_both_ways(host, tmp_path):
    paths = _copies(tmp_path, 3)
    for p in paths:
        host.open_path(p)
    assert host.state()["active"] == 2
    host.menu("next_doc")                 # ⌃Tab (Window menu)
    assert host.state()["active"] == 0
    host.menu("next_doc")
    assert host.state()["active"] == 1
    host.menu("prev_doc")                 # ⌃⇧Tab
    host.menu("prev_doc")
    assert host.state()["active"] == 2


def test_a_file_opens_only_once(host, tmp_path):
    a, b = _copies(tmp_path, 2)
    host.open_path(a)
    host.open_path(b)
    assert host.open_path(a)
    st = host.state()
    assert st["documents"] == [a.name, b.name] and st["active"] == 0


def test_a_file_open_in_another_window_brings_that_window(host, second, tmp_path):
    a, b = _copies(tmp_path, 2)
    host.open_path(a)
    host.open_path(b)
    other = second()
    host.extras()                          # nothing pending yet
    assert other.open_path(a)
    assert other.state()["documents"] == ["Untitled"]
    assert host.state()["active"] == 0     # its tab in front…
    assert host.extras()["focus"]          # …and its window asked to come forward


# ---- windows: one process --------------------------------------------------------------
def test_new_window_is_made_in_this_process(host, second):
    assert host.menu("new_window")
    token = host.extras()["new_window"]
    assert token                           # Rust makes the window, then:
    other = second()
    assert other.adopt(token)
    assert host.state()["windows"] == 2
    assert other.state()["start_page"]


def test_move_to_new_window_takes_the_project_and_its_history(host, second, tmp_path):
    from views import wgpu_tabs
    from built_core.history import AddFaceCommand
    from built_core import Vec3 as V
    a, b = _copies(tmp_path, 2)
    host.open_path(a)
    host.open_path(b)
    faces = host.state()["faces"]
    host.viewport.history.execute(AddFaceCommand([V(5, 0, 0), V(6, 0, 0), V(6, 1, 0)]))
    doc = host.window._doc
    history = doc.viewport.history
    assert wgpu_tabs._can_move(host.window)
    wgpu_tabs.action(host.window, host.view, "menu:move:1")
    token = host.extras()["new_window"]
    other = second()
    assert other.adopt(token)
    assert host.state()["documents"] == [a.name]
    st = other.state()
    assert st["documents"] == [b.name] and st["dirty"]
    assert other.window._doc is doc and doc.viewport.history is history
    assert doc.viewport._host is other and doc.stand_in._host is other
    assert other.state()["faces"] == faces + 1
    other.menu("undo")                     # its undo came along
    assert other.state()["faces"] == faces
    # Its window now lists the other one's project under ›› n.
    assert [e[1] for e in wgpu_tabs._elsewhere(other.window)] == [a.name]


def test_the_last_project_cannot_move_out(host, tmp_path):
    from built_render import shell_windows as windows
    (a,) = _copies(tmp_path, 1)
    host.open_path(a)
    assert not windows.can_move(host.window)
    assert windows.move_to_new_window(host.window, host.window._doc) is None


# ---- link sources ------------------------------------------------------------------------
def test_open_source_makes_a_link_tab_right_of_its_host(host, tmp_path):
    from tests.fixtures.shell.make_composite import make
    from views import wgpu_tabs
    paths = make(tmp_path / "composite")
    (other,) = _copies(tmp_path, 1)
    host.open_path(other)
    host.open_path(paths["host"])
    _settle(host)
    host.window.activate_document(host.window._documents[1])
    assert wgpu_tabs.open_link_source(host.window, paths["mid"])
    tabs = wgpu_tabs.tabs(host.window)
    assert [t[1] for t in tabs] == [other.name, "host.built", "mid.built"]
    assert [t[4] for t in tabs] == [False, False, True]
    assert "linked in host.built · 2 placements" in tabs[2][5]
    assert host.state()["link_tabs"] == ["mid.built"]
    from built_core.layers import Layer
    from views import wgpu_ui
    parent, mount = wgpu_tabs.source_context(host.window)
    assert parent.path == paths["host"] and mount in parent.viewport.scene.groups
    guide = Layer("Survey guides")
    parent.viewport.scene.layers.append(guide)
    source = host.viewport
    history_before = len(source.history.undo_stack)
    placement = source.scene.groups[0]
    # Host changes stay on that document's undo stack and keep the source tab active.
    wgpu_ui._tree_action(host.window, host.view, wgpu_ui.SEP.join(("hosteye", guide.key, "0")))
    assert not guide.visible and parent.viewport.history.undo()
    assert guide.visible and host.viewport is source and source.scene.groups[0] is placement
    assert len(source.history.undo_stack) == history_before


# ---- the right-click menu ----------------------------------------------------------------
@pytest.fixture
def kinds(host, tmp_path):
    """The model with one of each kind, open, its links loaded; what to
    select for each kind."""
    from built_core import links
    from tests.fixtures.shell.make_kinds import make
    paths = make(tmp_path / "kinds")
    assert host.open_path(paths["kinds"])
    _settle(host)
    scene = host.viewport.scene
    groups = {g.name: g for g in scene.groups}
    link = next(g for g in scene.groups if links.is_linked(g))
    face = next(iter(scene.mesh.faces))
    edge = next(iter(scene.mesh.edges))
    return {
        "faces": [face], "edges": [edge], "geometry": [face, edge],
        "group": [groups["Box A"]], "component": [groups["Door"]],
        "groups": [groups["Box A"], groups["Box B"]], "mixed": [groups["Box A"], face],
        "link": [link], "linked_part": [link.children[0]],
        "annotation": [scene.text_labels[0]], "dimension": [scene.dimensions[0]],
        "guide": [scene.guides[0]], "empty": [],
    }


#: Each kind's column in docs/context-menu.md: (table heading, column).
DOC_COLUMNS = {
    "faces": ("Raw geometry", 1), "edges": ("Raw geometry", 2), "geometry": ("Raw geometry", 3),
    "group": ("Groups and components", 1), "component": ("Groups and components", 2),
    "groups": ("Groups and components", 3),
    "mixed": ("Mixed, linked, annotations, guides, empty space", 1),
    "link": ("Mixed, linked, annotations, guides, empty space", 2),
    "linked_part": ("Mixed, linked, annotations, guides, empty space", 3),
    "annotation": ("Mixed, linked, annotations, guides, empty space", 4),
    "guide": ("Mixed, linked, annotations, guides, empty space", 5),
    "empty": ("Mixed, linked, annotations, guides, empty space", 6),
}


def _doc_columns():
    """docs/context-menu.md's tables: heading → column → [(family, {verbs})]."""
    text = (ROOT / "docs" / "context-menu.md").read_text(encoding="utf-8")
    out = {}
    for section in re.split(r"^### ", text, flags=re.M)[1:]:
        title, _, body = section.partition("\n")
        cols = {}
        for line in body.splitlines():
            if not line.startswith("| ") or line.startswith("| #") or line.startswith("|---"):
                continue
            cells = [c.strip() for c in re.split(r"(?<!\\)\|", line.strip().strip("|"))]
            if not cells or not cells[0].isdigit():
                continue
            for i, cell in enumerate(cells[1:], 1):
                cell = re.sub(r"\*\([^)]*\)\*", "", cell).replace("\\|", "|").strip()
                if not cell:
                    continue
                family, _, rest = cell.rpartition(": ") if ": " in cell else ("", "", cell)
                verbs = {v.strip() for v in rest.split("|")}
                cols.setdefault(i, []).append((family, verbs))
        out[title.strip()] = cols
    return out


def _matches(row, documented):
    """A built row is documented when its family matches and its verbs are
    the documented ones or some of them (the rest are conditional)."""
    label = row.label
    verbs = {v.label for v in row.verbs}
    return any(family == label and verbs <= allowed for family, allowed in documented)


@pytest.mark.parametrize("kind", list(DOC_COLUMNS) + ["dimension"])
def test_each_kind_gets_the_rows_docs_context_menu_lists(host, kinds, kind):
    from built_render import context_menu_build, context_menu_action, verbs_constants
    scene = host.viewport.scene
    scene.select(kinds[kind])
    menu = context_menu_build(host.window, host.view)
    expected = "annotation" if kind == "dimension" else kind
    assert menu.kind == expected
    assert menu.rows[0].verbs[0].id == "repeat"
    assert len(menu.rows) <= verbs_constants()[3]
    heading, col = DOC_COLUMNS[expected]
    documented = _doc_columns()[heading][col]
    missing = [r.text() for r in menu.rows[1:] if not _matches(r, documented)]
    assert not missing, f"{kind}: rows not in docs/context-menu.md: {missing}"
    assert not any(r.label == "More" or r.text().startswith("More") for r in menu.rows)


def test_the_menus_carry_the_expected_verbs(host, kinds, monkeypatch):
    from built_render import context_menu_build, context_menu_action, verbs_constants
    from built_render import shell_services as native
    scene = host.viewport.scene
    # A model copied by an earlier test would add "Paste here" to empty space.
    monkeypatch.setattr(native, "pasteboard_types", lambda: ([], 0))
    host.viewport.clipboard = None

    def rows(kind):
        scene.select(kinds[kind])
        return context_menu_build(host.window, host.view).labels()[1:]
    assert rows("faces")[:6] == ["Hide | Erase", "Zoom to | Reveal selection | Tag…", "Make group | Make component…",
                                 "Reverse faces | Orient faces", "Intersect: Model | Selection | Context",
                                 "Soften / Smooth edges…"]
    assert "Divide…" in rows("edges") and "Reverse faces | Orient faces" not in rows("edges")
    assert rows("group")[:6] == ["Open for editing", "Rename…", "Hide | Isolate", "Lock | Erase", "Zoom to | Reveal selection | Tag…", "File to…"]
    assert "Explode | Make component…" in rows("group") and "Split into pieces" in rows("group")
    assert "Select instances | Make unique" in rows("component")
    assert "Merge groups | Explode" in rows("groups")
    assert rows("link")[:2] == ["Select contents", "Hide"] and "Edit source" in rows("link")
    assert rows("linked_part")[0] == "Select its linked file"
    assert rows("annotation") == ["Edit text…", "Zoom to | Reveal selection | Tag…", "Erase"]
    assert rows("guide") == ["Delete guide", "Delete all guides", "Zoom to | Reveal selection | Tag…"]
    assert rows("empty")[:3] == ["Select all", "Unhide all", "Zoom extents"]


@pytest.mark.parametrize("kind", ["link", "linked_part"])
def test_read_only_kinds_offer_no_editing_verbs(host, kinds, kind):
    from built_render import context_menu_build, context_menu_action, verbs_constants
    host.viewport.scene.select(kinds[kind])
    menu = context_menu_build(host.window, host.view)
    assert menu.kind == kind and "read-only" in menu.subtitle
    ids = {i.split(":")[0] for i in menu.ids()}
    assert not ids & set(verbs_constants()[2]), ids & set(verbs_constants()[2])
    assert not menu.rows[0].verbs[0].enabled            # Repeat, greyed


def test_the_header_says_what_and_where(host, kinds):
    from built_render import context_menu_build, context_menu_action, verbs_constants
    scene = host.viewport.scene
    scene.select(kinds["group"])
    menu = context_menu_build(host.window, host.view)
    assert (menu.title, menu.subtitle) == ("Box A", "Group · Not filed")
    scene.select(kinds["linked_part"])
    assert context_menu_build(host.window, host.view).subtitle == "in parts.built ↗ · read-only"
    scene.select(kinds["groups"])
    assert context_menu_build(host.window, host.view).title == "2 items"


def test_repeat_names_the_last_command_and_its_value(host, kinds):
    from tools.pushpull import PushPullTool
    from built_render import context_menu_build, context_menu_action, verbs_constants
    host.window._activate_tool("pushpull")
    host.window._activate_tool("select")
    old = PushPullTool.last_distance
    PushPullTool.last_distance = 0.45
    try:
        repeat = context_menu_build(host.window, host.view).rows[0].verbs[0]
    finally:
        PushPullTool.last_distance = old
    name = host.window._tools["pushpull"].name
    assert repeat.label.startswith(f"Repeat {name}") and "0.45" in repeat.label.replace(",", ".")
    assert repeat.enabled and repeat.key == "⇧R"


def test_a_verb_runs_the_same_command_as_the_tree(host, kinds):
    """Hide on one group goes through the tree's row verb (core ``hide``)."""
    from built_render import context_menu_build, context_menu_action, verbs_constants
    scene = host.viewport.scene
    box = kinds["group"][0]
    scene.select([box])
    menu = context_menu_build(host.window, host.view)
    host.view._ctx_menu = menu
    assert context_menu_action(host.window, host.view, "hide")
    assert box.hidden
    host.menu("undo")
    assert not box.hidden


def test_right_click_selects_what_it_hit_first(host, kinds, monkeypatch):
    """Right-clicking an unselected group selects it, then the menu is the
    group’s. Picking needs the GPU, so the hit is given."""
    box = kinds["group"][0]
    face = kinds["faces"][0]
    scene = host.viewport.scene
    monkeypatch.setattr(host.view, "renderer", StubRenderer())
    monkeypatch.setattr(host.view, "_ui_attached", True, raising=False)
    tools = host.view.tools
    monkeypatch.setattr(tools, "pick_locked_group", lambda x, y: None)
    monkeypatch.setattr(tools, "pick_group", lambda x, y: box)
    monkeypatch.setattr(tools, "pick_edge", lambda x, y: None)
    monkeypatch.setattr(tools, "pick_face", lambda x, y: None)
    scene.select([face])
    host.pointer("press", 500.0, 400.0, 2, 2, 0)
    host.pointer("release", 500.0, 400.0, 2, 0, 0)
    assert list(scene.selection) == [box]
    assert host.view._ctx_menu.kind == "group"


def test_right_click_inside_the_selection_keeps_it(host, kinds, monkeypatch):
    a, b = kinds["groups"]
    scene = host.viewport.scene
    monkeypatch.setattr(host.view, "renderer", StubRenderer())
    monkeypatch.setattr(host.view, "_ui_attached", True, raising=False)
    tools = host.view.tools
    monkeypatch.setattr(tools, "pick_locked_group", lambda x, y: None)
    monkeypatch.setattr(tools, "pick_group", lambda x, y: a)
    monkeypatch.setattr(tools, "pick_edge", lambda x, y: None)
    monkeypatch.setattr(tools, "pick_face", lambda x, y: None)
    scene.select([a, b])
    host.pointer("press", 500.0, 400.0, 2, 2, 0)
    assert set(scene.selection) == {a, b}
    assert host.view._ctx_menu.kind == "groups"


def test_with_the_chrome_the_menu_goes_to_it(host, kinds, monkeypatch):
    """The chrome's path (native and Qt hosts): WgpuView's right-click
    hands the rows to the renderer, and ctx:<id> runs the verb."""
    from views import wgpu_ui
    box = kinds["group"][0]
    view = host.view
    tools = view.tools
    monkeypatch.setattr(tools, "pick_group", lambda x, y: box)
    monkeypatch.setattr(tools, "pick_edge", lambda x, y: None)
    monkeypatch.setattr(tools, "pick_face", lambda x, y: None)
    stub = StubRenderer()
    view.renderer = stub
    view._ui_attached = True
    view._ui_tools_sent = True
    try:
        from built_render import ContextMenuEvent
        view.contextMenuEvent(ContextMenuEvent(520, 410, 0))
        x, y, title, subtitle, rows = stub.menu
        assert (x, y, title) == (520.0, 410.0, "Box A")
        assert rows[0][1][0][0] == "repeat"
        assert ("Intersect", [("intersect:model", "Model", "", True, -1),
                              ("intersect:selection", "Selection", "", True, -1),
                              ("intersect:context", "Context", "", True, -1)]) in rows
        wgpu_ui._run(host.window, ["ctx:hide"])
        assert box.hidden
    finally:
        view.renderer = None
        view._ui_attached = False


def test_feed_changes_follow_the_document_across_windows(host, second):
    from built_render import shell_windows as windows
    from built_core.history import SetPluginDataCommand
    from views.extension_api import ExtensionApp

    src = host.window
    doc = src.new_document()
    dst = second()
    left, right = [], []
    ExtensionApp(src, "probe").on_document_changed(lambda: left.append(src._doc))
    ExtensionApp(dst.window, "probe").on_document_changed(lambda: right.append(dst.window._doc))
    # Each window owns its plugin registrations. A moved viewport must switch
    # all three callback lists, including those registered before it arrived.
    callback_lists = ("_ext_overlays", "_ext_snap_providers", "_ext_pickables")
    destination = [getattr(dst.viewport, key) for key in callback_lists]
    for key, callbacks in zip(callback_lists, destination):
        assert getattr(doc.viewport, key) is not callbacks
    history = doc.viewport.history
    history.execute(SetPluginDataCommand("probe", 1))
    assert left == [doc] and not right
    assert windows.transfer(src, doc, dst)
    for key, callbacks in zip(callback_lists, destination):
        assert getattr(doc.viewport, key) is callbacks
    left.clear()
    right.clear()
    history.undo()
    history.redo()
    doc.viewport.notify_scene_changed()  # the same revision also reached the view
    assert not left and right == [doc, doc]
    dst.close()
    history.undo()
    assert right == [doc, doc]


def test_replacing_the_viewport_history_detaches_the_old_feed(host):
    from built_core.history import History, SetPluginDataCommand
    from built_core.scene import Scene
    from views.extension_api import ExtensionApp

    vp = host.viewport
    original = vp.history
    scene = Scene()
    replacement = History(scene)
    notices = []
    ExtensionApp(host.window, "probe").on_document_changed(lambda: notices.append(vp.scene))
    vp.set_document(scene, replacement)
    assert notices == [scene]
    original.execute(SetPluginDataCommand("old", 1))
    assert notices == [scene]
    replacement.execute(SetPluginDataCommand("new", 2))
    assert notices == [scene, scene]
    # Bypass history as a legacy plugin may do. An idle turn notices it once.
    scene.version += 1
    host.idle()
    host.idle()
    assert notices == [scene, scene, scene]


def test_input_and_modal_buttons_stay_in_the_owning_window(host, second):
    from built_render import shell_modal as modal
    from built_core import window_input
    from built_core import Input as Qt, PixelPoint as QPoint

    other = second()
    host.pointer("move", 17, 23, held=1, modifiers=Qt.ShiftModifier.value)
    other.pointer("move", 5, 3, modifiers=Qt.ControlModifier.value)
    assert window_input.mouse_buttons(host.window) == Qt.LeftButton
    assert window_input.cursor_position(host.view) == QPoint(17, 23)
    assert window_input.modifiers(host.viewport) == Qt.ShiftModifier
    assert window_input.mouse_buttons(other.viewport) == Qt.NoButton
    assert window_input.modifiers(other.viewport) == Qt.ControlModifier
    other.view._ui_attached = True
    modal.dispatch(other, ("press", 1, 2, 2, 0, 0, ""))
    assert window_input.mouse_buttons(other.window) == Qt.RightButton
    modal.dispatch(other, ("release", 1, 2, 2, 0, 0, ""))
    assert window_input.mouse_buttons(other.window) == Qt.NoButton
    host.focus(False)
    assert window_input.mouse_buttons(host.window) == Qt.NoButton
    assert window_input.modifiers(host.viewport) == Qt.NoModifier


def test_closing_the_move_origin_releases_its_request_and_restores_the_other_host(host, tmp_path):
    from built_render import Host, shell_modal, shell_windows

    origin = Host(0, 640, 480, 1.0, settings=str(tmp_path / "origin"))
    try:
        assert shell_modal.host() is origin
        token = shell_windows.request(origin, "move", (origin.window, origin.window._doc))
    finally:
        origin.close()
    assert shell_windows.take(token) is None
    assert origin not in shell_windows.hosts()
    assert shell_modal.host() is host
