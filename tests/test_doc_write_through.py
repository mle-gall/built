# SPDX-License-Identifier: GPL-3.0-or-later
"""In-place edits reach the Rust document as they are made
(``core/doc_model.py``): what Rust reads of a placement's custom attribute
or a scene table is what Python sees, outside a transaction too. A plain
container stored into a tracked one keeps its identity while anything else
holds it (Rust readers flush it first); once nothing does, a flush puts a
tracked copy in its place."""
from __future__ import annotations

import gc

import ingetrazo_core
import pytest

import core.doc_model as dm
from core.group import Group
from core.scene import Scene


def doc(handle, name):
    return ingetrazo_core.doc_attr_value(handle, name)


def suspect(value) -> bool:
    """Whether ``value`` waits to be written back (other tests' may too)."""
    return any(entry[3] is value for entry in dm._SUSPECT.values())


def test_a_nested_edit_is_in_the_document_at_once():
    g = Group(name="g")
    g.ext = {"a": {"b": 1, "l": [1, 2]}}
    ext = g.ext
    ext["a"]["b"] = 2
    assert not suspect(ext["a"])
    assert doc(g, "ext") == {"a": {"b": 2, "l": [1, 2]}}
    ext["a"]["l"].append(3)
    ext["a"]["l"][0] = 9
    assert doc(g, "ext")["a"]["l"] == [9, 2, 3]
    del ext["a"]["b"]
    ext["a"].setdefault("c", 4)
    ext["a"]["l"].sort()
    assert doc(g, "ext") == {"a": {"l": [2, 3, 9], "c": 4}}
    ext["a"].pop("c")
    ext["a"]["l"].clear()
    assert doc(g, "ext") == {"a": {"l": []}}


def test_a_root_table_edit_is_in_the_document_at_once():
    s = Scene()
    s.link_registry = {"/a.igz": {"overrides": {}}}
    s.link_registry["/a.igz"]["overrides"]["Wood"] = False
    assert doc(s._root, "link_registry") == {"/a.igz": {"overrides": {"Wood": False}}}


def test_a_plain_container_stored_keeps_its_identity_while_held():
    s = Scene()
    reg = s.link_registry
    entry = reg["/a.igz"] = {}
    entry["x"] = 1                       # through the plain object
    assert reg["/a.igz"] is entry
    assert doc(s._root, "link_registry") == {"/a.igz": {"x": 1}}
    dm.flush()
    assert reg["/a.igz"] is entry        # still held: not replaced
    entry["y"] = 2
    assert doc(s._root, "link_registry") == {"/a.igz": {"x": 1, "y": 2}}


def test_one_nothing_else_holds_is_tracked_from_the_next_flush():
    s = Scene()
    reg = s.link_registry
    value = {"inner": {}}
    reg["/a.igz"] = value
    assert suspect(value)
    del value
    gc.collect()
    dm.flush()
    assert type(reg["/a.igz"]) is dm.TrackedDict
    inner = reg["/a.igz"]["inner"]
    assert type(inner) is dm.TrackedDict
    inner["z"] = 3
    assert doc(s._root, "link_registry") == {"/a.igz": {"inner": {"z": 3}}}


def test_a_container_held_inside_keeps_the_whole_value_suspect():
    s = Scene()
    reg = s.link_registry
    outer = {"inner": {}}
    inner = outer["inner"]
    reg["/a.igz"] = outer
    del outer
    dm.flush()
    assert suspect(dict.__getitem__(reg, "/a.igz"))   # inner is still held
    inner["z"] = 1
    assert doc(s._root, "link_registry") == {"/a.igz": {"inner": {"z": 1}}}
    assert reg["/a.igz"]["inner"] is inner
    del inner
    dm.flush()
    assert type(reg["/a.igz"]) is dm.TrackedDict


def test_rust_reads_a_link_path_edited_in_place():
    s = Scene()
    g = Group(name="Site")
    g.ext = {"link": {"path": "/a/Site.igz"}}
    s.groups.append(g)
    assert ingetrazo_core.link_sources(s) == ["/a/Site.igz"]
    g.ext["link"]["path"] = "/b/Site.igz"
    assert ingetrazo_core.link_sources(s) == ["/b/Site.igz"]
    rec = {"path": "/c/Site.igz"}
    g.ext["link"] = rec                  # a plain record stored, still held
    rec["path"] = "/d/Site.igz"
    assert ingetrazo_core.link_sources(s) == ["/d/Site.igz"]


def test_undo_puts_back_an_edit_made_in_a_step():
    from core.history import Command, History

    class Edit(Command):
        doc_undo = True

        def do(self, scene):
            scene.plugin_data["k"]["n"] = 2

        def undo(self, scene):
            scene.plugin_data["k"]["n"] = 1

    s = Scene()
    s.plugin_data = {"k": {"n": 1}}
    h = History(s)
    h.execute(Edit())
    assert doc(s._root, "plugin_data") == {"k": {"n": 2}}
    h.undo()
    assert s.plugin_data == {"k": {"n": 1}} == doc(s._root, "plugin_data")
    h.redo()
    assert s.plugin_data == {"k": {"n": 2}} == doc(s._root, "plugin_data")
