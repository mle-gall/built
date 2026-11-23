"""Slice 2 of the Rust document (docs/rust-document-model.md §11): custom
attributes and scene tables as document values (tracked containers), the
materials registry and kept components as document lists, tag styles, the
tree invariant, bulk writes and reads, the viewport's detached proxies and
the renderer's dirtiness from the step's transaction."""
import gc

import numpy as np
import pytest
from PySide6.QtGui import QMatrix4x4, QVector3D

from core import doc_model
from core.group import Group, PlacementProxy, model_placements
from core.history import (AssignLayerCommand, HideCommand, History,
                          LockCommand, SetPluginDataCommand, SetRecordCommand,
                          SetStructureNodeCommand)
from core.layers import STYLE_FIELDS, Layer, layer_values
from core.materials import Material
from core.mesh import Mesh
from core.scene import Scene
from formats import built

pytestmark = pytest.mark.skipif(not doc_model.ENABLED,
                                reason="the Rust document is not built")
N = doc_model.NATIVE


def txn(fn):
    doc_model.begin()
    try:
        fn()
    finally:
        t = doc_model.commit()
    return t


# ------------------------------------------------- tracked attributes

def test_an_attribute_edited_in_place_is_journalled():
    g = Group(Mesh(), name="g")
    g.ext = {"link": {"path": "a.igz"}}
    view = g.ext
    assert isinstance(view, dict) and isinstance(view["link"], dict)
    t = txn(lambda: g.ext["link"].update(rel="a.igz"))
    assert [op[0] for op in t.ops()] == ["set"]
    assert g.ext == {"link": {"path": "a.igz", "rel": "a.igz"}}
    t.undo()
    assert g.ext == {"link": {"path": "a.igz"}}
    t.redo()
    assert g.ext["link"]["rel"] == "a.igz"


def test_a_replaced_container_stops_writing():
    g = Group(Mesh(), name="g")
    g.ifc = {"class": "IfcWall"}
    old = g.ifc
    g.ifc = {"class": "IfcSlab"}
    old["name"] = "stale"
    doc_model.flush()
    assert g.ifc == {"class": "IfcSlab"}
    assert old._doc_bind is None


def test_a_plain_container_stored_inside_is_followed():
    scene = Scene()
    reg = scene.plugin_data["links"] = {}      # the very dict, as in Python
    reg["a.igz"] = {"overrides": {}}
    entry = reg["a.igz"]

    def edit():
        entry["overrides"]["Wood"] = False
    t = txn(edit)
    assert scene.plugin_data["links"]["a.igz"]["overrides"] == {"Wood": False}
    t.undo()
    assert scene.plugin_data["links"]["a.igz"]["overrides"] == {}


def test_attributes_are_plain_for_json_copy_and_pickle():
    import copy
    import json
    import pickle
    g = Group(Mesh(), name="g")
    g.ifc = {"class": "IfcBeam", "classifications": [{"system": "x"}]}
    assert json.loads(json.dumps(g.ifc)) == g.ifc
    for c in (copy.deepcopy(g.ifc), pickle.loads(pickle.dumps(g.ifc)),
              doc_model.plain(g.ifc)):
        assert type(c) is dict and type(c["classifications"]) is list
        assert c == g.ifc


def test_values_that_are_not_json_are_kept_as_they_are():
    scene = Scene()
    s = {1, 2}
    scene.plugin_data["bad"] = s
    doc_model.flush()
    assert scene.plugin_data["bad"] is s


def test_records_and_plugin_data_undo_by_transaction():
    scene = Scene()
    g = Group(Mesh(), name="g")
    scene.groups.append(g)
    h = History(scene)
    steps = []
    h.feed.subscribe(steps.append)
    for cmd in (SetRecordCommand([g], {"class": "IfcBeam", "name": "B"}),
                SetStructureNodeCommand([g], "level-1"),
                SetPluginDataCommand("ext", {"a": [1, 2]})):
        h.execute(cmd)
        assert not h.last_error, h.last_error
        assert cmd._doc_txn is not None, type(cmd).__name__
    assert g.ifc["class"] == "IfcBeam" and g.ext == {"structure": {"node": "level-1"}}
    assert scene.plugin_data["ext"] == {"a": [1, 2]}
    # The record's placement, named by the journal.
    assert steps[0].placements["modified"] == [g.uid]
    while h.undo():
        pass
    assert g.ifc is None and g.ext is None and "ext" not in scene.plugin_data
    while h.redo():
        pass
    assert g.ifc["name"] == "B" and scene.plugin_data["ext"]["a"] == [1, 2]


# ------------------------------------------------------ scene tables

def test_scene_tables_live_on_the_root():
    scene = Scene()
    assert scene.layer_folders == [] and scene.folder_parents == {}

    def edit():
        scene.layer_folders.append("Site")
        scene.folder_visibility["Site"] = False
        scene.active_layer = "Tag"
    t = txn(edit)
    changes = t.changes(root=scene._root._doc_key)
    assert set(changes["scene"]["modified"]) == {"layer_folders",
                                                 "folder_visibility",
                                                 "active_layer"}
    t.undo()
    assert scene.layer_folders == [] and scene.folder_visibility == {}
    assert scene.active_layer is None


def test_tag_styles_are_document_fields():
    ly = Layer("Walls", linetype="dashed", hatch="brick", opacity=0.5)
    assert "linetype" not in vars(ly)
    values = layer_values(ly)
    assert all(f in values for f in STYLE_FIELDS)
    assert values["linetype"] == "dashed" and values["opacity"] == 0.5
    scene = Scene()
    scene.layers.append(ly)
    t = txn(lambda: setattr(ly, "lineweight", 0.35))
    assert t.changes()["tags"]["modified"] == ["Walls"]
    t.undo()
    assert ly.lineweight is None


# ------------------------------------------------ registry and kept

def test_the_registry_is_a_live_mapping():
    scene = Scene()
    red, blue = Material("Red", color=(1, 0, 0)), Material("Blue")
    scene.materials["Red"] = red
    assert scene.materials["Red"] is red and "Red" in scene.materials
    assert scene.materials == {"Red": red} and dict(scene.materials) == {"Red": red}
    assert scene.materials.get("Nope") is None and len(scene.materials) == 1

    def edit():
        scene.materials["Blue"] = blue
        scene.materials["Red"] = Material("Red", color=(0.5, 0, 0))
        scene.materials.pop("Blue")
        scene.materials.update({"Green": Material("Green")})
    t = txn(edit)
    assert [op[0] for op in t.ops()] == ["registry"]     # coalesced
    m = t.changes(root=scene._root._doc_key)["materials"]
    assert m["created"] == ["Green"] and m["replaced"] == ["Red"]
    assert m["removed"] == []
    t.undo()
    assert list(scene.materials) == ["Red"] and scene.materials["Red"] is red
    with pytest.raises(TypeError):
        scene.materials["x"] = {"color": (1, 0, 0)}


def test_the_registry_keeps_its_materials_alive():
    scene = Scene()
    scene.materials["Tmp"] = Material("Tmp", color=(0, 1, 0))
    gc.collect()
    assert scene.materials["Tmp"].color == (0, 1, 0)


def test_kept_components_are_a_document_list():
    scene = Scene()
    g = Group(Mesh(), name="def")
    t = txn(lambda: scene.kept_components.append(g))
    assert scene.kept_components == [g]
    # Kept definitions are not in the model: no placement created.
    assert t.changes(root=scene._root._doc_key)["placements"]["created"] == []
    t.undo()
    assert scene.kept_components == []


# ---------------------------------------------------- tree invariant

def test_a_placement_has_one_parent():
    a, b, c = Group(Mesh(), name="a"), Group(Mesh(), name="b"), Group(Mesh(), name="c")
    a.children.append(c)
    with pytest.raises(ValueError, match="one parent"):
        b.children.append(c)
    with pytest.raises(ValueError, match="one parent"):
        a.children.append(c)                 # twice in one list
    with pytest.raises(ValueError, match="one parent"):
        b.children = [c]
    scene = Scene()
    with pytest.raises(ValueError, match="one parent"):
        scene.groups.append(c)
    a.children.remove(c)
    b.children.append(c)                     # taken out first: fine
    b.children = [c]                         # the same list again: fine
    assert c in b.children and c not in a.children


def test_undo_replays_a_move_between_parents():
    a, b, c = Group(Mesh(), name="a"), Group(Mesh(), name="b"), Group(Mesh(), name="c")
    a.children.append(c)

    def move():
        a.children.remove(c)
        b.children.append(c)
    t = txn(move)
    t.undo()
    assert list(a.children) == [c] and list(b.children) == []
    t.redo()
    assert list(b.children) == [c]


# ------------------------------------------------------ bulk writes

def test_a_bulk_write_is_one_op_and_undoes_exactly():
    gs = [Group(Mesh(), name=f"g{i}") for i in range(50)]
    gs[3].locked = True
    t = txn(lambda: N.doc_set_many(gs, "locked", True))
    assert t.ops() == [("set_many", 49, "locked")]   # g3 did not change
    assert all(g.locked for g in gs)
    t.undo()
    assert [g.locked for g in gs] == [i == 3 for i in range(50)]
    assert N.doc_get_many(gs[:4], "locked") == [False, False, False, True]


def test_a_single_write_after_a_bulk_one_undoes_in_order():
    gs = [Group(Mesh(), name=f"g{i}") for i in range(3)]

    def edit():
        N.doc_set_many(gs, "layer", "A")
        gs[1].layer = "B"
    t = txn(edit)
    assert [g.layer for g in gs] == ["A", "B", "A"]
    t.undo()
    assert [g.layer for g in gs] == [None, None, None]
    t.redo()
    assert [g.layer for g in gs] == ["A", "B", "A"]


def test_lock_hide_and_tag_of_a_selection_write_in_bulk():
    scene = Scene()
    gs = [Group(Mesh(), name=f"g{i}") for i in range(20)]
    scene.groups.extend(gs)
    h = History(scene)
    for cmd in (LockCommand(gs, True), HideCommand(gs, True),
                AssignLayerCommand(gs, "New tag")):
        h.execute(cmd)
        assert not h.last_error
        assert any(op[0] == "set_many" for op in cmd._doc_txn.ops())
    assert all(g.locked and g.hidden and g.layer == "New tag" for g in gs)
    while h.undo():
        pass
    assert not any(g.locked or g.hidden or g.layer for g in gs)


# ------------------------------------------------------- bulk reads

def _tree():
    defs = [Mesh() for _ in range(3)]
    top = Group(Mesh(), name="top")
    t = QMatrix4x4()
    t.translate(QVector3D(5, 0, 0))
    top.xform = t
    mid = Group(Mesh(), name="mid")          # no matrix: inherits
    top.children.append(mid)
    for i, m in enumerate(defs):
        leaf = Group(m, name=f"leaf{i}")
        r = QMatrix4x4()
        r.rotate(10.0 * i, QVector3D(0, 0, 1))
        r.translate(QVector3D(i, 0.5, 0))
        leaf.xform = r
        mid.children.append(leaf)
    site = Group(defs[0], name="site")
    site.xform = built.placement([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0,
                                  650000.013, 6860000.027, 1.5, 1])
    deep = Group(defs[1], name="deep")
    deep.xform = QMatrix4x4()
    deep.xform.scale(2.0)
    site.children.append(deep)
    return [top, site, Group(Mesh(), name="classic")]


def test_doc_tree_composes_as_model_placements():
    from views.wgpu_scene import _doc_worlds
    roots = _tree()
    handles, parent, local, has, hidden = N.doc_tree(roots)
    world, defined = _doc_worlds(local, has, parent)
    ref = [p for r in roots for p in model_placements(r)]
    assert [g for g, _m in ref] == list(handles)
    for i, (_g, m) in enumerate(ref):
        if m is None:
            assert not defined[i]
        else:
            assert defined[i] and np.array_equal(world[i], m)   # bit for bit


def test_doc_walk_is_iter_groups():
    from core import purge
    roots = _tree()
    roots.append(roots[0])                   # a root listed twice: once
    fast = list(N.doc_walk(roots))
    doc_model.ENABLED, was = False, doc_model.ENABLED
    try:
        slow = list(purge.iter_groups(roots))
    finally:
        doc_model.ENABLED = was
    assert fast == slow


# -------------------------------------------------- viewport proxies

def test_a_viewport_proxy_is_a_group_outside_the_document():
    gc.collect()
    before = N.doc_stats()["placements"]
    p = PlacementProxy(Mesh(), name="part", uid="u1")
    assert isinstance(p, Group) and p._doc_key == 0
    assert N.doc_stats()["placements"] == before
    p.layer, p.hidden, p.material = "T", True, {"color": (1, 0, 0)}
    p.xform = QMatrix4x4()
    assert (p.name, p.uid, p.layer, p.hidden) == ("part", "u1", "T", True)
    assert p.children == [] and p.ifc is None


# ------------------------------------------ renderer dirtiness by txn

def test_hide_dirties_only_the_hidden_subtree():
    scene = Scene()
    a, b = Group(Mesh(), name="a"), Group(Mesh(), name="b")
    kid = Group(Mesh(), name="kid")
    kid.xform = QMatrix4x4()
    a.adopt([kid])
    scene.groups.extend([a, b])
    for g in (a, b, kid):
        g.mesh._chunk_dirty = g.mesh._attrs_dirty = False
    h = History(scene)
    h.execute(HideCommand([a], True))
    assert a.mesh._attrs_dirty and kid.mesh._attrs_dirty
    assert not b.mesh._attrs_dirty and not b.mesh._chunk_dirty
    a.mesh._attrs_dirty = kid.mesh._attrs_dirty = False
    h.undo()
    assert a.mesh._attrs_dirty and kid.mesh._attrs_dirty and not b.mesh._attrs_dirty


def test_a_transaction_names_the_placements_it_wrote():
    a, b = Group(Mesh(), name="a"), Group(Mesh(), name="b")

    def edit():
        a.hidden = True
        b.name = "renamed"
    t = txn(edit)
    assert list(t.placements(["hidden", "layer"])) == [a]
    assert list(t.placements()) == [a, b]


def test_a_held_container_sees_the_undo():
    """Undo by transaction refills the attribute's tracked container in
    place, nested ones included, as a command's own undo editing the dict
    showed it (a tree view holding a links-registry entry across Ctrl+Z)."""
    scene = Scene()
    scene.plugin_data["links"] = {"a.igz": {"overrides": {}}}
    doc_model.flush()
    entry = scene.plugin_data["links"]["a.igz"]
    t = txn(lambda: entry["overrides"].update({"A / Rails": False}))
    t.undo()
    assert entry["overrides"] == {} and scene.plugin_data["links"]["a.igz"] is entry
    t.redo()
    assert entry["overrides"] == {"A / Rails": False}


def test_tracked_values_are_no_reference_cycles():
    import weakref
    g = Group(Mesh(), name="g")
    g.ext = {"link": {"path": "a.igz", "parts": [{"uid": "x"}]}}
    inner = weakref.ref(g.ext["link"]["parts"][0])
    top = weakref.ref(g.ext)
    gc.disable()
    try:
        del g
        assert inner() is None and top() is None     # no collector needed
    finally:
        gc.enable()
