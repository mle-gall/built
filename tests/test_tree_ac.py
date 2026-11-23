# SPDX-License-Identifier: GPL-3.0-or-later
"""The Project tree, option A + C (docs/ui-decisions.md, 2026-11-23), on a
small copy of the composite fixture named like Maxence's case: Site links
Ride (which links Ride stairs and Ride facilities) and Show building.

A: linked files are typed root rows (file icon, state dot), a file's single
top node is merged into its row, nested files sit at the root once each
"via" their parent, the selection is marked once. C: column sets widen the
dock, and the inspector offers only the verbs that apply to the selection's
kind (views/verbs.py)."""
from __future__ import annotations

import os
import time
from pathlib import Path

import pytest

import views.wgpu_ui as wu
from tests.test_tree_links import _Renderer, _View, _Win

SMALL = 400


@pytest.fixture(scope="module")
def site(tmp_path_factory):
    """Ride stairs.igz, Ride facilities.igz, Show building.igz, Ride.igz and
    Site.igz (as tests/composite_fixture.py builds C, D, B, A and the host),
    plus Site twice.igz, which places Ride stairs itself and through Ride."""
    from tests.composite_fixture import _file
    folder = tmp_path_factory.mktemp("site")
    stairs = _file(folder / "Ride stairs.igz", {"Stairs": None}, {"Flights": "Stairs", "Landings": "Stairs"}, 60)
    facilities = _file(folder / "Ride facilities.igz", {"Facilities": None, "Rooms": "Facilities"},
                       {"Toilets": "Rooms", "Kiosks": "Rooms", "Signs": "Facilities"}, 90)
    show = _file(folder / "Show building.igz", {"Building": None, "Levels": "Building"},
                 {"Slabs": "Levels", "Walls": "Levels", "Site": "Building"}, 300)
    ride = _file(folder / "Ride.igz", {"Ride": None, "Track": "Ride", "Supports": "Ride"},
                 {"Rails": "Track", "Ties": "Track", "Columns": "Supports", "Base plates": "Supports",
                  "Fasteners": "Ride"}, SMALL, links_to=(stairs, facilities))
    host = _file(folder / "Site.igz", {"Site works": None}, {"Paths": "Site works", "Fences": "Site works"},
                 50, links_to=(ride, show))
    twice = _file(folder / "Site twice.igz", {"Site works": None}, {"Paths": "Site works"}, 10,
                  links_to=(ride, stairs))
    return dict(host=host, twice=twice, ride=ride, stairs=stairs, facilities=facilities, show=show)


def _open(path):
    from core.scene import Scene
    from formats import igz
    scene = Scene()
    igz.load_into(scene, Path(path))
    win, view = _Win(scene), _View()

    def rows():
        wu._tree(view, win)
        return [dict(key=t[0], name=t[1], badge=t[3], icon=t[4], depth=n[0], count=n[1], row=n[4],
                     sel=n[9], mark=n[10]) for t, n in view.renderer.calls["ui_set_tree"][0]]

    def act(verb, key, extra=None):
        wu._tree_action(win, view, wu.SEP.join([verb, key] + ([extra] if extra is not None else [])))
        return rows()
    rows()
    return scene, win, view, rows, act


@pytest.fixture
def ui(site):
    return _open(site["host"])


def _key(view, name):
    return next(k for k, n in view._ui_tree["nodes"].items() if n["name"] == name)


# ---- A: files are roots ---------------------------------------------------------

def test_files_are_typed_root_rows_after_the_hosts_own_nodes(ui):
    scene, win, view, rows, act = ui
    roots = [r for r in rows() if r["depth"] == 0]
    assert [r["name"] for r in roots] == ["Site works", "Ride.igz", "Ride stairs.igz", "Ride facilities.igz",
                                          "Show building.igz"]
    files = {r["name"]: r for r in roots[1:]}
    assert all(r["icon"] == "file:resolved" and r["row"] == 0 for r in files.values())
    # Only the host's own nodes start open: each file is one folded row.
    assert not any(r["depth"] > 0 and r["key"].startswith("n:link:") for r in rows())


def test_file_row_states_found_changed_missing(site, tmp_path):
    import shutil
    for k in ("host", "ride", "stairs", "facilities", "show"):
        shutil.copy2(site[k], tmp_path / Path(site[k]).name)
    # The links point at the copies only if their records are relative; else
    # move the originals out of the way is not ours to do: work on copies of
    # the whole folder and touch / remove there.
    scene, win, view, rows, act = _open(tmp_path / "Site.igz")
    paths = {Path(n["path"]).name: n["path"] for n in view._ui_tree["nodes"].values() if n["kind"] == "link"}
    if not all(str(p).startswith(str(tmp_path)) for p in paths.values()):
        pytest.skip("links resolve to the originals, not the copies")
    st = os.stat(paths["Ride.igz"])
    os.utime(paths["Ride.igz"], ns=(st.st_atime_ns, st.st_mtime_ns + 5_000_000_000))
    os.remove(paths["Show building.igz"])
    view._ui_tree_sig = None
    icons = {r["name"]: r["icon"] for r in rows() if r["depth"] == 0}
    assert icons["Ride.igz"] == "file:modified"
    assert icons["Show building.igz"] == "file:missing"
    assert icons["Ride stairs.igz"] == "file:resolved"


def test_file_state_reads_without_writing_the_link_registry(ui):
    from core import links
    from core.link_structure import file_state
    scene = ui[0]
    before = {k: dict(v) for k, v in links.registry(scene).items()}
    nested = next(n["path"] for n in ui[2]._ui_tree["nodes"].values() if n.get("via"))
    assert file_state(scene, nested, nested=True) == "resolved"
    assert file_state(scene, "/nowhere/Gone.igz") == "missing"
    assert {k: dict(v) for k, v in links.registry(scene).items()} == before


def test_a_single_top_node_is_merged_into_its_file_row(ui):
    scene, win, view, rows, act = ui
    tree = view._ui_tree["tree"]
    # Ride.igz's own top node is «Ride»: never "Ride.igz › Ride".
    assert tree.nodes["link:Ride"]["merged"] == "Ride"
    assert [tree.nodes[k]["name"] for k in tree.children["link:Ride"]] == ["Track", "Supports", "Fasteners"]
    got = act("toggle", "n:link:Ride")
    under = [r["name"] for r in got if r["depth"] == 1 and r["key"].startswith("n:link:Ride/")]
    assert under == ["Track", "Supports", "Fasteners"] and "Ride" not in under
    # Every part of the file is still counted on its row.
    assert view._ui_tree["counts"]["link:Ride"] == SMALL
    # A file with several top nodes keeps them: none here; the host's own
    # single top node is not merged (it is not a file).
    assert _key(view, "Site works") in tree.children[None]


def test_a_file_without_structure_merges_its_single_top_tag_folder(ui):
    from core import bim_registry, link_structure, links
    scene, win, view, rows, act = ui
    reg = links.registry(scene)
    path = next(p for p in reg if p.endswith("Show building.igz"))
    saved = reg[path].pop("structure", None)
    try:
        tree = link_structure.Tree(scene, bim_registry.get(scene))
        key = "link:Show building"
        assert tree.nodes[key]["merged"] == "Building"
        names = [tree.nodes[k]["name"] for k in tree.children[key]]
        assert {"Levels", "Site"} <= set(names) and "Building" not in names
        assert sum(len(tree.direct.get(k, ())) for k in tree.subtree(key)) == 300
    finally:
        if saved is not None:
            reg[path]["structure"] = saved


def test_nested_links_are_roots_marked_via_their_parent(ui):
    scene, win, view, rows, act = ui
    tree = view._ui_tree["tree"]
    stairs = tree.nodes["link:Ride/link:Ride stairs"]
    assert stairs["parent"] is None and stairs["via"] == "Ride"
    r = {x["key"]: x for x in rows()}
    assert r["n:link:Ride/link:Ride stairs"]["badge"] == "via Ride"
    assert r["n:link:Ride"]["badge"] == "" and r["n:link:Show building"]["badge"] == ""
    # Nested rows count only their own file's parts; Ride does not include them.
    counts = view._ui_tree["counts"]
    assert counts["link:Ride/link:Ride stairs"] == 60 and counts["link:Ride"] == SMALL
    # File verbs on a nested row: open and show, never reload or unlink there.
    from views import verbs
    assert verbs.file_verbs(True) == ("linkopen", "linkfinder")
    act("linkunlink", "n:link:Ride/link:Ride stairs")
    assert any("comes inside Ride" in s for s in win.viewport.status)


def test_a_file_placed_twice_is_one_row(site):
    scene, win, view, rows, act = _open(site["twice"])
    tree = view._ui_tree["tree"]
    files = [n for n in tree.nodes.values() if n["kind"] == "link"]
    names = [Path(n["path"]).name for n in files]
    assert sorted(names) == sorted(set(names))
    row = next(k for k, n in tree.nodes.items() if n["kind"] == "link" and n["path"].endswith("Ride stairs.igz"))
    # The host's own placement wins: no "via"; both chains' parts and
    # placements count on that one row, and its eye covers both folders.
    assert tree.nodes[row]["via"] is None
    assert view._ui_tree["counts"][row] == 120 and len(tree.links[row]) == 2
    assert len(tree.nodes[row]["folders"]) == 2


# ---- A: the selection is marked once ----------------------------------------------

def test_the_selection_is_marked_once_on_the_deepest_row(ui):
    scene, win, view, rows, act = ui
    tree = view._ui_tree["tree"]
    rails = _key(view, "Rails")
    rail = tree.direct[rails][0]
    view._ui_exp.update({"n:link:Ride", "n:" + _key(view, "Track")})
    scene.select([rail])
    got = {r["key"]: r for r in rows()}
    assert got["n:" + rails]["mark"] == 1
    assert got["n:" + _key(view, "Track")]["mark"] == 2 and got["n:link:Ride"]["mark"] == 2
    assert got["n:" + _key(view, "Site works")]["mark"] == 0
    assert sum(1 for r in got.values() if r["mark"] == 1) == 1
    # Open Rails: the item row holds it now, Rails gets the quiet mark.
    got = {r["key"]: r for r in act("toggle", "n:" + rails)}
    assert got[f"i:{rail.uid}"]["mark"] == 1 and got["n:" + rails]["mark"] == 2
    assert sum(1 for r in got.values() if r["mark"] == 1) == 1


def test_mark_selection_counts_paged_out_items_on_the_node():
    rows = [([k], [d, 0, 0, 0, row, -1, -1, 0, 0, sel, 0]) for k, d, row, sel in (
        ("n:a", 0, 0, 3),      # node: 3 selected below it
        ("i:1", 1, 1, 1),      # one shown and selected
        ("m:a", 1, 4, 0),      # "Show more": two more past the page
        ("n:b", 0, 0, 0),
    )]
    wu.mark_selection(rows)
    assert [r[1][10] for r in rows] == [1, 1, 0, 0]


# ---- C: column sets ---------------------------------------------------------------

@pytest.fixture
def settings(tmp_path):
    from PySide6.QtCore import QSettings
    st = QSettings(str(tmp_path / "dock.ini"), QSettings.IniFormat)
    yield st
    st.sync()


def test_drawing_widens_the_dock_and_filing_and_deliver_keep_the_users_width(ui, settings):
    scene, win, view, rows, act = ui
    settings.setValue("ui/dock_width", 320.0)
    seen = []
    view.renderer.ui_set_dock_width = seen.append
    for colset in (1, 2, 0):
        view._ui_colset = colset
        wu.apply_dock_width(view, settings)
    assert seen == [wu.WIDE_DOCK, 320.0, 320.0]
    # A width dragged in Drawing is kept for Drawing only.
    view._ui_colset = 1
    wu.keep_dock_width(view, "430", settings)
    assert float(settings.value("ui/dock_width")) == 320.0
    assert wu.dock_width(1, settings) == 430.0 and wu.dock_width(0, settings) == 320.0
    # Never narrower than the user's own width.
    settings.setValue("ui/dock_width", 480.0)
    assert wu.dock_width(1, settings) == 480.0


def test_colset_action_sends_the_width(ui, monkeypatch):
    scene, win, view, rows, act = ui
    seen = []
    view.renderer.ui_set_dock_width = seen.append
    monkeypatch.setattr(wu, "dock_width", lambda colset, settings=None: 390.0 if colset == 1 else 300.0)
    act("colset", "1")
    act("colset", "0")
    assert seen == [390.0, 300.0]
    assert view.renderer.calls["ui_set_colset"] == (0,)


def test_drawing_cells_carry_colour_line_weight_print(ui):
    scene, win, view, rows, act = ui
    view._ui_colset = 1
    view._ui_tree_sig = None
    wu._tree(view, win)
    row = next(t for t, _n in view.renderer.calls["ui_set_tree"][0] if t[0] == "n:" + _key(view, "Paths"))
    assert len(row) == 10  # key, name, kind, badge, icon + colour, line, weight, print, hatch
    file_row = next(t for t, _n in view.renderer.calls["ui_set_tree"][0] if t[0] == "n:link:Ride")
    assert file_row[4] == "file:resolved"


# ---- C: the inspector's verbs by kind -----------------------------------------------

class _Action:
    def __init__(self, label):
        self.label = label

    def shortcut(self):
        class _K:
            def toString(self, *_a):
                return ""
        return _K()

    def trigger(self):
        pass

    def isEnabled(self):
        return True


def _verbs(win, sel):
    labels = ["Hide", "Explode Group", "Make Group", "Make Component…"] + wu.MORE
    win._ui_edit_actions = {lb: _Action(lb) for lb in labels}
    return {label for _id, label, _k, _run, _on, _more in wu._commands(win, sel)}


def test_inspector_verbs_follow_the_selection_kind(ui):
    from core import links
    from core.group import Group
    from core.mesh import Mesh
    from ingetrazo_core import Vec3 as QVector3D
    from views import verbs
    scene, win, view, rows, act = ui
    tree = view._ui_tree["tree"]
    part = tree.direct[_key(view, "Rails")][0]
    link = tree.links["link:Ride"][0]
    own = tree.direct[_key(view, "Paths")][0]
    assert verbs.kind_of(scene, [part]) == "linked_part" and verbs.kind_of(scene, [link]) == "link"
    editing = {"Edit", "Explode", "Component", "File to…", "Group"}
    # A part inside a linked file, and the link placement: nothing that edits.
    for sel in ([part], [link]):
        got = _verbs(win, sel)
        assert {"Isolate", "Hide"} <= got and not got & editing, got
        assert not got & {"Reverse Faces", "Soften / Smooth Edges"}, got
    # This document's own group: the full set.
    assert {"File to…", "Isolate", "Hide", "Edit", "Explode", "Component"} <= _verbs(win, [own])
    # Raw geometry: a group can be made, never a component, never filed.
    m = Mesh()
    m.add_face([QVector3D(0, 0, 0), QVector3D(1, 0, 0), QVector3D(1, 1, 0), QVector3D(0, 1, 0)])
    faces = list(m.faces)
    assert verbs.kind_of(scene, faces) == "faces"
    got = _verbs(win, faces)
    assert "Group" in got and "Component" not in got and "File to…" not in got
    assert isinstance(own, Group) and links.linked_ancestor(own) is None


def test_read_only_inspector_has_no_editable_name_and_no_edits(ui):
    scene, win, view, rows, act = ui
    tree = view._ui_tree["tree"]
    part = tree.direct[_key(view, "Rails")][0]
    part.ifc = {"class": "IfcMember"}
    try:
        scene.select([part])
        view._ui_insp_sig = None
        wu._inspector(view, win)
        title, sections = view.renderer.calls["ui_set_inspector"]
        item = dict(dict(sections)["Item"])
        assert not item["Name"].startswith(wu.ED), "a linked part's name must not be editable"
        assert item["Filed in"].startswith("Ride.igz")
        acts = [i for name, a in sections if name == "Actions" for i, _l in a]
        assert not any(a.startswith(("status:", "solidrepair")) for a in acts)
        assert "linkfile" in acts and "linkopen" in acts
        # Select its linked file: the placement holding the part.
        wu._tree_action(win, view, "insp:linkfile")
        assert list(scene.selection) == [tree.links["link:Ride"][0]]
    finally:
        part.ifc = None
    # This document's own group keeps its editable name.
    own = tree.direct[_key(view, "Paths")][0]
    scene.select([own])
    view._ui_insp_sig = None
    wu._inspector(view, win)
    item = dict(dict(view.renderer.calls["ui_set_inspector"][1])["Item"])
    assert item["Name"].startswith(wu.ED)


def test_rows_stay_fast_on_the_bigger_fixture(ui):
    scene, win, view, rows, act = ui
    t = time.perf_counter()
    for _ in range(5):
        view._ui_tree_sig = None
        rows()
    assert (time.perf_counter() - t) / 5 < 0.25


def test_drawing_cell_edits_run_without_a_layer_panel_and_undo(ui):
    from core.protocol import commands
    scene, win, view, _, act = ui
    view._ui_colset = 1
    node = "n:" + _key(view, "Paths")
    tag = scene.layer("Paths")
    for verb, value, attr, expected in [
            ("color", "#ff8000", "color", (1.0, 128 / 255, 0.0)),
            ("linetype", "dashed", "linetype", "dashed"),
            ("weight", "0.35", "lineweight", 0.35),
            ("print", "0", "printable", False),
            ("hatch", "brick", "hatch", "brick")]:
        before = getattr(tag, attr)
        act(verb, node, value)
        assert getattr(tag, attr) == expected
        assert win.viewport.history.undo()
        assert getattr(tag, attr) == before
        assert win.viewport.history.redo()
        assert getattr(tag, attr) == expected
    act("color", node, "")
    act("weight", node, "auto")
    assert tag.color is None and tag.lineweight is None
    rev = win.viewport.history.revision
    with pytest.raises(KeyError):
        commands.run(win.viewport.history, "edit_tag", tags=[tag.key, "gone"],
                     field="weight", value="0.5")
    assert tag.lineweight is None and win.viewport.history.revision == rev


def test_rename_and_move_keep_node_and_membership_and_undo(ui):
    from core import bim_registry, structure
    scene, win, view, rows, act = ui
    key, parent, root = (_key(view, n) for n in ("Paths", "Fences", "Site works"))

    def current():
        return structure._nodes(bim_registry.get(scene))[key]

    original = current()
    act("rename", "n:" + key, "Walkways")
    assert current()["name"] == "Walkways"
    assert _key(view, "Walkways") == key
    assert win.viewport.history.undo()
    assert current() == original
    members = structure.members(scene, key)
    for target in (parent, ""):
        act("parent", "n:" + key, target)
        assert current()["parent"] == (target or None)
        assert structure.members(scene, key) == members
        assert win.viewport.history.undo()
        assert current()["parent"] == root
    version = win.viewport.history.revision
    for source, target in ((root, key), (key, key), (key, "link:Ride")):
        act("parent", "n:" + source, target)
    assert win.viewport.history.revision == version
    assert current() == original


def test_unstructured_host_shows_read_only_drawing_rows_without_writing():
    from core import bim_registry
    from core.layers import Layer
    from core.scene import Scene
    scene = Scene()
    scene.layer_folders = ["site", "empty"]
    scene.layer_folder_names = {"site": "Site works"}
    scene.layers.append(Layer("Paths", folder="site"))
    scene.layers.append(Layer("Grid"))
    view, win = _View(), _Win(scene)
    before = scene.version, bim_registry.get(scene), dict(scene.layer_folder_uids)

    def preview():
        wu._tree(view, win)
        return [(t[0], t[1], t[2], n[0]) for t, n in view.renderer.calls["ui_set_tree"][0] if n[4] == 6]

    assert ("preview-folder:site", "Site works", "", 0) in preview()
    assert ("preview-folder:empty", "empty", "", 0) in preview()
    assert ("preview-tag:Grid", "Grid", "Drawing only", 0) in preview()
    assert not any(row[1] == "Paths" for row in preview())
    wu._tree_action(win, view, wu.SEP.join(("filter", "paths")))
    assert preview() == [("preview-folder:site", "Site works", "", 0),
                         ("preview-tag:Paths", "Paths", "Drawing only", 1)]
    assert (scene.version, bim_registry.get(scene), dict(scene.layer_folder_uids)) == before
    assert not win.viewport.history.undo_stack


def test_drawing_row_changes_visibility_and_lock_in_undo_order(ui):
    scene, win, view, rows, act = ui
    key = "n:" + _key(view, "Paths")
    tag = scene.layer("Paths")
    act("eye", key)
    act("lock", key)
    assert not tag.visible and tag.locked
    assert win.viewport.history.undo()
    assert not tag.visible and not tag.locked
    assert win.viewport.history.undo()
    assert tag.visible and not tag.locked


def test_filing_selected_items_to_active_node_is_one_undo_step(ui):
    from core import bim_registry, structure
    scene, win, view, rows, act = ui
    src, dest = (_key(view, n) for n in ("Paths", "Fences"))
    act("active", "n:" + dest)
    group = view._ui_tree["direct"][src][0]
    other = view._ui_tree["direct"][dest][0]
    scene.select([group, other])
    act("filehere", "n:" + dest)
    assert group.uid in structure.members(scene, dest)
    assert other.uid in structure.members(scene, dest)
    assert group.layer == scene.layer("Fences").key
    assert bim_registry.get(scene)["structure"]["active"] == dest
    assert win.viewport.history.undo()
    assert group.uid in structure.members(scene, src)
    assert other.uid in structure.members(scene, dest)
    assert group.layer == scene.layer("Paths").key


def test_isolating_another_node_then_show_all_restores_the_view(ui):
    scene, win, view, rows, act = ui
    a, b = (_key(view, n) for n in ("Paths", "Fences"))
    for key in (a, b):
        act("isolate", "n:" + key)
        assert set(win.viewport._view_expression["in"]) == set(g.uid for g in view._ui_tree["direct"][key])
    act("showall", "")
    assert not win.viewport._view_expression


def test_switching_column_sets_updates_the_focused_node_inspector(ui):
    scene, win, view, rows, act = ui
    key = "n:" + _key(view, "Paths")
    act("node", key)
    wu._inspector(view, win)
    assert "Drawing" not in dict(view.renderer.calls["ui_set_inspector"][1])
    act("colset", "1")
    wu._inspector(view, win)
    catalog = dict(view.renderer.calls["ui_set_inspector"][1])["Drawing"]
    assert catalog[0] == ("node", key) and ("hatch", "brick_flemish") in catalog


@pytest.mark.parametrize("operation", ["group", "insert", "paste", "link", "group_selection"])
def test_created_placement_inherits_the_active_node_in_its_creation_step(ui, site, operation):
    from core import bim_registry, links, structure
    from core.group import Group
    from core.history import InsertGroupCommand, MakeGroupCommand, MakeNestedGroupCommand
    from ingetrazo_core import Vec3
    scene, win, view, _, act = ui
    dest = _key(view, "Fences")
    act("active", "n:" + dest)
    assert bim_registry.get(scene)["structure"]["active"] == dest
    if operation == "group":
        f = scene.mesh.add_face([Vec3(0, 0, 0), Vec3(1, 0, 0), Vec3(0, 1, 0)])
        cmd = MakeGroupCommand([f], list(scene.mesh.edges))
    elif operation == "group_selection":
        cmd = MakeNestedGroupCommand([], [], view._ui_tree["direct"][_key(view, "Paths")][:2])
    elif operation == "link":
        group = links.make_link(scene, site["stairs"], host_path=site["host"])
        cmd = InsertGroupCommand(group)
    else:
        group = Group(name=operation)
        if operation == "paste":
            group.layer = scene.layer("Paths").key
            group.ext = {"structure": {"node": _key(view, "Paths")}}
        cmd = InsertGroupCommand(group)
    h = win.viewport.history
    before = len(h.undo_stack)
    h.execute(cmd)
    assert h.last_error is None and len(h.undo_stack) == before + 1
    group = cmd.container if operation == "group_selection" else cmd.group
    assert structure.node_of(group) == dest and group.layer == scene.layer("Fences").key
    assert h.undo() and group not in scene.groups
    assert h.redo() and group in scene.groups
    assert structure.node_of(group) == dest and group.layer == scene.layer("Fences").key
