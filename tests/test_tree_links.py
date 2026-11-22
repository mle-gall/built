# SPDX-License-Identifier: GPL-3.0-or-later
"""The Project tree with linked files (ui/tree-links, 2026-11-22), on a
synthetic composite (tests/composite_fixture.py): a host linking A and B,
A linking C and D, A with 20 000 placements. Each linked file's structure
nests under its row with its items counted there; one root order; a link
row selects its placement; item lists are paged; eye and lock work on
mounted nodes and undo."""
from __future__ import annotations

import time
from pathlib import Path

import pytest

BIG = 20000


@pytest.fixture(scope="module")
def files(tmp_path_factory):
    from tests.composite_fixture import build
    return build(tmp_path_factory.mktemp("composite"), BIG)


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


class _Win:
    def __init__(self, scene):
        self.viewport = _Viewport(scene)


class _View:
    def __init__(self):
        self.renderer = _Renderer()

    def update(self):
        pass


@pytest.fixture
def ui(files):
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


def test_linked_structures_nest_under_their_link_rows_with_their_counts(ui):
    scene, win, view, rows, act = ui
    tree = view._ui_tree["tree"]
    counts = view._ui_tree["counts"]
    # One root order: the host's own nodes, then Linked files.
    assert [tree.nodes[k]["name"] for k in tree.children[None]] == ["Site works", "Linked files"]
    assert [tree.nodes[k]["name"] for k in tree.children["linked"]] == ["A", "B"]
    # Nested links sit under A's own Linked files, never at the top.
    assert tree.nodes["link:A/link:C"]["parent"] == "link:A/linked"
    assert tree.nodes["link:A/link:D"]["parent"] == "link:A/linked"
    assert not any(k in tree.children[None] for k in ("link:C", "link:D"))
    # A's own structure comes before its Linked files.
    names = [tree.nodes[k]["name"] for k in tree.children["link:A"]]
    assert names == ["Ride", "Linked files"]
    # Items are counted where they belong; the link placements are the rows.
    assert counts["link:A"] == BIG + 600 + 900
    assert counts["link:A/link:C"] == 600 and counts["link:A/link:D"] == 900
    assert counts["link:B"] == 3000 and counts["linked"] == BIG + 600 + 900 + 3000
    assert counts[_key(view, "Rails")] == BIG // 5
    assert counts[_key(view, "Flights")] == 300 and counts[_key(view, "Toilets")] == 300
    assert counts[_key(view, "Site works")] == 50
    assert not view._ui_tree["unfiled"]
    # The rows: link rows read "Reference" and explain it.
    r = {x["key"]: x for x in rows()}
    assert r["n:link:A"]["name"] == "A ↗" and r["n:link:A"]["kind"] == "Reference"
    wu = __import__("views.wgpu_ui", fromlist=["x"])
    raw = {t[0]: t[2] for t, _n in view.renderer.calls["ui_set_tree"][0]}
    assert "linked file" in raw["n:link:A"].split(wu.TIP)[1]
    assert "no IFC class" in raw[f"n:{_key(view, 'Paths')}"].split(wu.TIP)[1]


def test_bootstrap_mounts_only_the_hosts_own_links(ui):
    from core import bim_registry, structure
    scene = ui[0]
    nodes = structure.bootstrap(scene, bim_registry.get(scene))["structure"]["nodes"]
    roots = [n["name"] for n in nodes.values() if n["parent"] is None]
    assert "C" not in roots and "D" not in roots
    assert nodes["link:A/link:C"]["parent"].startswith("link:A/")


def test_a_link_row_selects_its_placement_and_inspects_the_file(ui):
    from core import links
    scene, win, view, rows, act = ui
    import views.wgpu_ui as wu
    t = time.perf_counter()
    act("node", "n:link:A")
    took = time.perf_counter() - t
    (g,) = scene.selection
    assert links.is_linked(g) and g.name == "A"
    assert took < 0.5, took
    wu._inspector(view, win)
    title, sections = view.renderer.calls["ui_set_inspector"]
    assert title == "A"
    info = dict(dict(sections)["Linked file"])
    assert info["What"].startswith("Reference") and info["File"] == "A.igz" and info["Items"] == str(BIG + 1500)
    actions = [label for name, acts in sections if name == "Actions" for _id, label in acts]
    assert {"Open source", "Reload", "Hide", "Isolate", "Select contents"} <= set(actions)
    # A node inside the file selects nothing and is described instead.
    act("node", "n:" + _key(view, "Rails"))
    assert not scene.selection
    wu._inspector(view, win)
    title, sections = view.renderer.calls["ui_set_inspector"]
    assert title == "Rails" and dict(dict(sections)["Node"])["From"].startswith("A.igz")
    # Select contents is the explicit verb.
    act("contents", "n:" + _key(view, "Rails"))
    assert len(scene.selection) == BIG // 5


def test_expanding_is_paged_and_cheap(ui):
    scene, win, view, rows, act = ui
    import views.wgpu_ui as wu
    t = time.perf_counter()
    for name in (None, "Ride", "Track", "Rails"):
        got = act("toggle", "n:link:A" if name is None else "n:" + _key(view, name))
    took = time.perf_counter() - t
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
    view._ui_exp.update({"n:link:A", "n:" + _key(view, "Ride"), "n:" + _key(view, "Track")})
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
