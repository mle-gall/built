# SPDX-License-Identifier: GPL-3.0-or-later
"""The Project tree with linked files (ui/tree-links, 2026-11-22; files as
roots since tree A + C, 2026-11-23), on a synthetic composite
(tests/composite_fixture.py): a host linking A and B, A linking C and D, A
with 20 000 placements. Each linked file is a root row with its structure
under it (a single top node merged into the row) and its items counted
there; nested files are roots too, "via" their parent; a link row selects
its placement; item lists are paged; eye and lock work on
mounted nodes and undo; Isolate works on host groups, link rows, mounted
nodes and mixed selections, and Show all restores what was there."""
from __future__ import annotations

import time
from pathlib import Path

import pytest

#: A's placements. The tests of what the tree shows use SMALL; the three
#: that bound the cost of a link row, paging and Isolate use BIG, so the
#: work they bound is real (``ui`` takes the size as an indirect parameter).
SMALL = 2000
BIG = 20000
at_scale = pytest.mark.parametrize("ui", [BIG], indirect=True)


@pytest.fixture(scope="module")
def small_files(tmp_path_factory):
    from tests.composite_fixture import build
    return build(tmp_path_factory.mktemp("composite"), SMALL)


@pytest.fixture(scope="module")
def big_files(tmp_path_factory):
    from tests.composite_fixture import build
    return build(tmp_path_factory.mktemp("composite-big"), BIG)


class _Renderer:
    def __init__(self):
        self.calls = {}

    def __getattr__(self, name):
        def record(*args):
            self.calls[name] = args
        return record


class _Viewport:
    def __init__(self, scene):
        from core.history import History
        self.scene, self.history = scene, History(scene)
        self.status = []

    def flash_status(self, text, *_a):
        self.status.append(text)

    def update(self):
        pass

    def notify_scene_changed(self):
        pass

    def refresh_after_edit(self):
        self.notify_scene_changed()


class _Win:
    def __init__(self, scene):
        self.viewport = _Viewport(scene)


class _View:
    def __init__(self):
        self.renderer = _Renderer()

    def update(self):
        pass


@pytest.fixture
def ui(request):
    files = request.getfixturevalue("big_files" if getattr(request, "param", SMALL) == BIG else "small_files")
    from core.scene import Scene
    from formats import igz
    import views.wgpu_ui as wu
    scene = Scene()
    igz.load_into(scene, Path(files["host"]))
    win, view = _Win(scene), _View()

    def rows():
        wu._tree(view, win)
        return [dict(key=t[0], name=t[1], kind=t[2].split(wu.TIP)[0], depth=n[0], count=n[1], row=n[4],
                     eye=n[5], lock=n[6]) for t, n in view.renderer.calls["ui_set_tree"][0]]

    def act(verb, key, extra=None):
        wu._tree_action(win, view, wu.SEP.join([verb, key] + ([extra] if extra is not None else [])))
        return rows()

    rows()
    return scene, win, view, rows, act


def _key(view, name):
    return next(k for k, n in view._ui_tree["nodes"].items() if n["name"] == name)


def test_linked_files_are_roots_with_their_structures_and_counts(ui):
    scene, win, view, rows, act = ui
    tree = view._ui_tree["tree"]
    counts = view._ui_tree["counts"]
    # One root order: the host's own nodes, then every file once, each
    # nested file right after the file placing it.
    assert [tree.nodes[k]["name"] for k in tree.children[None]] == ["Site works", "A", "C", "D", "B"]
    assert [tree.nodes[k]["via"] for k in tree.children[None][1:]] == [None, "A", "A", None]
    assert "linked" not in tree.nodes and not any(n["kind"] == "linked" for n in tree.nodes.values())
    # A's single top node «Ride» is merged into A's row: no "A › Ride".
    names = [tree.nodes[k]["name"] for k in tree.children["link:A"]]
    assert names == ["Track", "Supports", "Fasteners"]
    assert tree.nodes["link:A"]["merged"] == "Ride"
    assert tree.nodes["link:A/link:C"]["merged"] == "Stairs"
    # Items are counted where they belong; the link placements are the rows.
    assert counts["link:A"] == SMALL
    assert counts["link:A/link:C"] == 600 and counts["link:A/link:D"] == 900
    assert counts["link:B"] == 3000
    assert counts[_key(view, "Rails")] == SMALL // 5
    assert counts[_key(view, "Flights")] == 300 and counts[_key(view, "Toilets")] == 300
    assert counts[_key(view, "Site works")] == 50
    assert not view._ui_tree["unfiled"]
    # The rows: a file row is typed (file icon with its state), named by
    # its file, nested ones say which file places them.
    r = {x["key"]: x for x in rows()}
    wu = __import__("views.wgpu_ui", fromlist=["x"])
    raw = {t[0]: t for t, _n in view.renderer.calls["ui_set_tree"][0]}
    assert r["n:link:A"]["name"] == "A.igz" and raw["n:link:A"][4] == "file:resolved"
    assert raw["n:link:A/link:C"][3] == "via A" and raw["n:link:A"][3] == ""
    assert "linked file" in raw["n:link:A"][2].split(wu.TIP)[1]
    assert "no IFC class" in raw[f"n:{_key(view, 'Paths')}"][2].split(wu.TIP)[1]


def test_bootstrap_mounts_only_the_hosts_own_links(ui):
    from core import bim_registry, structure
    scene = ui[0]
    nodes = structure.bootstrap(scene, bim_registry.get(scene))["structure"]["nodes"]
    roots = [n["name"] for n in nodes.values() if n["parent"] is None]
    assert "C" not in roots and "D" not in roots
    assert nodes["link:A/link:C"]["parent"].startswith("link:A/")


@at_scale
def test_a_link_row_selects_its_placement_and_inspects_the_file(ui):
    from core import links
    scene, win, view, rows, act = ui
    import views.wgpu_ui as wu
    # This process's CPU time, not wall time: other work on the machine
    # does not count against it.
    t = time.process_time()
    act("node", "n:link:A")
    took = time.process_time() - t
    (g,) = scene.selection
    assert links.is_linked(g) and g.name == "A"
    assert took < 0.5, took
    wu._inspector(view, win)
    title, sections = view.renderer.calls["ui_set_inspector"]
    assert title == "A.igz"
    info = dict(dict(sections)["Linked file"])
    assert info["What"].startswith("Linked file") and info["File"] == "A.igz" and info["Items"] == str(BIG)
    assert info["Status"] == "Up to date" and info["Top node"] == "Ride"
    actions = [label for name, acts in sections if name == "Actions" for _id, label in acts]
    assert actions[:4] == ["Edit source", "Reload", "Unlink", "Show in Finder"]
    assert "Select contents" in actions
    # A node inside the file selects nothing and is described instead.
    act("node", "n:" + _key(view, "Rails"))
    assert not scene.selection
    wu._inspector(view, win)
    title, sections = view.renderer.calls["ui_set_inspector"]
    assert title == "Rails" and dict(dict(sections)["Node"])["From"].startswith("A.igz")
    # Select contents is the explicit verb.
    act("contents", "n:" + _key(view, "Rails"))
    assert len(scene.selection) == BIG // 5


@at_scale
def test_expanding_is_paged_and_cheap(ui):
    scene, win, view, rows, act = ui
    import views.wgpu_ui as wu
    t = time.process_time()
    for name in (None, "Track", "Rails"):
        got = act("toggle", "n:link:A" if name is None else "n:" + _key(view, name))
    took = time.process_time() - t
    assert took < 1.0, took
    rails = _key(view, "Rails")
    items = [r for r in got if r["row"] == 1]
    assert len(items) == wu.ROW_CAP
    more = [r for r in got if r["key"] == f"m:{rails}"]
    assert len(more) == 1 and f"{BIG // 5 - wu.ROW_CAP} not shown" in more[0]["name"]
    got = act("loose", f"m:{rails}")
    assert len([r for r in got if r["row"] == 1]) == 2 * wu.ROW_CAP
    # Never every item: the whole tree stays small.
    assert len(got) < 3 * wu.ROW_CAP


def test_eye_and_lock_on_mounted_nodes_write_overrides_and_undo(ui):
    from core import layers as L, links
    scene, win, view, rows, act = ui
    rails = _key(view, "Rails")
    view._ui_exp.update({"n:link:A", "n:" + _key(view, "Track")})
    tag = scene.layer("A / Rails")
    entry = links.registry(scene)[next(p for p in links.registry(scene) if p.endswith("A.igz"))]
    assert next(r for r in rows() if r["key"] == "n:" + rails)["eye"] == 1
    act("eye", "n:" + rails)
    assert tag.visible is False and entry["overrides"]["A / Rails"] is False
    assert next(r for r in rows() if r["key"] == "n:" + rails)["eye"] == 0
    win.viewport.history.undo()
    assert tag.visible is True and "A / Rails" not in entry["overrides"]
    act("lock", "n:" + rails)
    assert tag.locked is True
    win.viewport.history.undo()
    assert tag.locked is False
    # The link row hides the whole file through its folder, nested files included.
    act("eye", "n:link:A")
    assert not L.folder_visible(scene, "A") and not L.effective_visible(scene, scene.layer("A / C / Flights"))
    assert next(r for r in rows() if r["key"] == "n:link:A")["eye"] == 0
    win.viewport.history.undo()
    assert L.folder_visible(scene, "A")
    # A nested file's node too.
    flights = _key(view, "Flights")
    act("eye", "n:" + flights)
    assert scene.layer("A / C / Flights").visible is False
    win.viewport.history.undo()
    assert scene.layer("A / C / Flights").visible is True


def test_linked_rows_are_read_only(ui):
    scene, win, view, rows, act = ui
    act("rename", "n:link:A", "Other")
    act("delete", "n:" + _key(view, "Rails"))
    assert view._ui_tree["nodes"]["link:A"]["name"] == "A"
    assert any("read-only" in s for s in win.viewport.status)


def test_set_tag_state_refuses_unknown_keys(ui):
    from core.protocol import commands
    scene, win = ui[0], ui[1]
    with pytest.raises(KeyError):
        commands.run(win.viewport.history, "set_tag_state", tags=["nope"], visible=False)
    with pytest.raises(ValueError):
        commands.run(win.viewport.history, "set_tag_state", folders=["A"], locked=True)
    assert not win.viewport.history.undo_stack


def _visible(scene, g):
    """Drawn by the Metal pack: not filtered, its holders included."""
    from types import SimpleNamespace
    from core.group import iter_placements
    from views import wgpu_scene
    root = g
    while getattr(root, "owner", None) is not None:
        root = root.owner
    root = next((r for r in scene.groups if r is root or any(p is g for p, _m in iter_placements(r))), root)
    for p, _m, _x, f, _c in wgpu_scene._placement_records(scene, [root], SimpleNamespace(scene=scene),
                                                      wgpu_scene._placement_state(scene)):
        if p is g:
            return not f & (wgpu_scene.VISIBILITY_FILTERED | 1)
    return False


@at_scale
def test_isolate_link_row_mounted_node_host_group_and_mixed_then_show_all(ui, monkeypatch):
    from core import facets
    from core.agent_facets import set_display
    import views.wgpu_ui as wu
    scene, win, view, rows, act = ui
    vp = win.viewport
    tree = view._ui_tree["tree"]
    a = tree.links["link:A"][0]
    b = tree.links["link:B"][0]
    host = tree.direct[_key(view, "Paths")][0]
    rail = tree.direct[_key(view, "Rails")][0]
    tie = tree.direct[_key(view, "Ties")][0]
    flight = tree.direct[_key(view, "Flights")][0]
    # A saved view's filter was on before: Show all comes back to it.
    set_display(vp, {"facet": "uid", "in": [a.uid, b.uid, host.uid]}, isolate=True, remember=False)
    before = (dict(vp._view_expression), list(getattr(vp, "_previous_views", [])))
    scene.select([host])
    # Isolate reads the native placement tree (doc/occurrences.rs): it never
    # rebuilds the facet index, which walked every occurrence in Python and
    # was invalidated by each view change. Measured in this process's own CPU
    # time, not wall time, so a loaded machine does not fail it.
    built = []
    monkeypatch.setattr(facets, "build", lambda *a, real=facets.build: built.append(1) or real(*a))
    t = time.process_time()
    wu._tree_action(win, view, wu.SEP.join(["isolate", "n:link:A"]))
    assert time.process_time() - t < 0.5
    assert not built
    rows()
    assert _visible(scene, a) and _visible(scene, rail) and _visible(scene, flight)
    assert not _visible(scene, b) and not _visible(scene, host)
    # A mounted node: its items show through the link placement holding them.
    act("isolate", "n:" + _key(view, "Rails"))
    assert _visible(scene, rail) and not _visible(scene, tie) and not _visible(scene, flight)
    assert a.uid in scene._navigator_through and a.uid not in scene._navigator_allowed
    # A host group, then a mixed selection (host group + linked parts).
    act("isolate", "n:" + _key(view, "Paths"))
    assert _visible(scene, host) and not _visible(scene, rail)
    wu._isolate(win, [host.uid, flight.uid])
    assert _visible(scene, host) and _visible(scene, flight) and not _visible(scene, rail)
    wu._show_all(vp)
    assert (vp._view_expression, vp._previous_views) == before
    assert scene.selection == {host}
    # Back to no filter at all: Show all with nothing isolated clears it.
    set_display(vp, remember=False)
    act("isolate", "n:link:B")
    wu._show_all(vp)
    assert scene._navigator_allowed is None and _visible(scene, rail)


def test_native_isolation_matches_the_facet_index(ui):
    """The native closure (doc/occurrences.rs) shows, and draws through,
    exactly what the Python facet index gives for the same keys."""
    from core import facets
    scene, win, view, rows, act = ui
    index = facets.build(scene)
    tree = view._ui_tree["tree"]
    picks = [[tree.links["link:A"][0].uid], [tree.direct[_key(view, "Rails")][0].uid],
             [tree.direct[_key(view, "Paths")][0].uid, tree.links["link:B"][0].uid],
             list(index)[5:40:7], ["no-such-key"]]
    for wanted in picks:
        for exact in (False, True):
            want = set(facets.evaluate(index, {"facet": "occurrence", "in": wanted}))
            allowed = {u for u, row in index.items()
                       if u in want or (not exact and any(p in want for p in row["path_keys"]))}
            through = {p for u in allowed for p in index[u]["path_keys"]} - allowed
            matched, n_allowed, n_through, qualified = scene.occurrence_closure(wanted, exact)
            assert matched == [u for u in index if u in want]
            assert (n_allowed, n_through) == (allowed, through), (wanted, exact)
            assert qualified == index.qualified


def test_packer_draws_isolated_parts_through_their_link(ui):
    """The Metal pack: an isolated part inside a link is drawn, the link's
    own geometry and the other parts are not."""
    from views import wgpu_scene
    scene, win, view, rows, act = ui
    tree = view._ui_tree["tree"]
    a = tree.links["link:A"][0]
    rail = tree.direct[_key(view, "Rails")][0]
    tie = tree.direct[_key(view, "Ties")][0]
    act("isolate", "n:" + _key(view, "Rails"))
    from types import SimpleNamespace
    flags = {id(g): f for g, _m, _p, f, _c in wgpu_scene._placement_records(
        scene, [a], SimpleNamespace(scene=scene), wgpu_scene._placement_state(scene))}
    filt = wgpu_scene.VISIBILITY_FILTERED
    assert flags[id(a)] & filt and not flags[id(rail)] & filt and flags[id(tie)] & filt
