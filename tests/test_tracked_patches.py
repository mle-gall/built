# SPDX-License-Identifier: GPL-3.0-or-later
"""Tracked attribute values written as keyed patches (architecture review
11), and the references plugins hold to them: a nested dict read once and
kept, a plain dict stored in and kept, a container moved inside its value."""
from core import doc_model
from core.group import Group
from core.mesh import Mesh
from core.scene import Scene


def txn(fn):
    doc_model.begin()
    try:
        fn()
    finally:
        t = doc_model.commit()
    return t


def group(ext):
    g = Group(Mesh(), name="g")
    g.ext = ext
    return g


def test_each_entry_edit_is_one_patch_and_undoes_in_place():
    g = group({"reg": {f"k{i}": {"n": i} for i in range(50)}})
    held = g.ext["reg"]["k7"]
    t = txn(lambda: (held.__setitem__("n", -7), g.ext["reg"].pop("k8"), g.ext["reg"].__setitem__("new", {"n": 1})))
    assert [op[0] for op in t.ops()] == ["patch", "patch", "patch"]
    assert g.ext["reg"]["k7"] == {"n": -7} and "k8" not in g.ext["reg"]
    t.undo()
    assert held == {"n": 7}                     # the held dict sees the undo
    assert g.ext["reg"]["k8"] == {"n": 8} and "new" not in g.ext["reg"]
    t.redo()
    assert held == {"n": -7} and g.ext["reg"]["new"] == {"n": 1}


def test_a_plain_dict_stored_and_held_is_written_at_the_boundary():
    g = group({"reg": {}})
    plain = {"path": "a.igz"}
    txn(lambda: g.ext["reg"].__setitem__("a", plain))
    # Through the plain reference, inside a step: that step's (its end
    # reads the stored subtree again).
    t = txn(lambda: plain.__setitem__("rel", "a.igz"))
    assert g.ext["reg"]["a"] == {"path": "a.igz", "rel": "a.igz"}
    t.undo()
    assert g.ext["reg"]["a"] == {"path": "a.igz"}


def test_a_container_moved_inside_its_value_stays_one_value():
    g = group({"a": {"x": 1}, "b": None})
    moved = g.ext["a"]
    txn(lambda: g.ext.__setitem__("b", moved))
    t = txn(lambda: moved.__setitem__("x", 2))
    assert g.ext["a"]["x"] == 2 and g.ext["b"]["x"] == 2
    t.undo()
    assert g.ext["b"]["x"] == 1


def test_list_entries_and_whole_list_edits():
    g = group({"items": [1, 2]})
    items = g.ext["items"]
    t = txn(lambda: (items.append(3), items.__setitem__(0, 9), items.insert(0, 0), items.sort()))
    assert g.ext["items"] == [0, 2, 3, 9]
    t.undo()
    assert g.ext["items"] == [1, 2] and items == [1, 2]


def test_edits_outside_a_step_are_written_before_the_next_and_detached_ones_not_at_all():
    g = group({"a": {"n": 1}, "b": {"n": 2}})
    a = g.ext["a"]
    a["n"] = 10                                  # outside a step: waits
    t = txn(lambda: g.ext["b"].__setitem__("n", 20))
    assert g.ext["a"]["n"] == 10
    t.undo()                                     # not that step's
    assert g.ext["a"]["n"] == 10 and g.ext["b"]["n"] == 2
    old = g.ext["b"]
    g.ext = {"b": {"n": 3}}                      # replaced: old is plain now
    txn(lambda: old.__setitem__("n", 99))
    assert g.ext == {"b": {"n": 3}}


def test_a_scene_table_registry_patch_leaves_other_entries_alone():
    scene = Scene()
    scene.plugin_data = {"reg": {f"k{i}": {"n": i} for i in range(1000)}}
    reg = scene.plugin_data["reg"]
    t = txn(lambda: reg["k5"].__setitem__("n", 0))
    assert [op[0] for op in t.ops()] == ["patch"]
    assert scene.plugin_data["reg"]["k5"] == {"n": 0} and scene.plugin_data["reg"]["k6"] == {"n": 6}
