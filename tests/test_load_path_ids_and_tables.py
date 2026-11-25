# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The open path's cheap ids and shared tables (2026-11-13, ride load):
link copies' ids are still exactly the UUID5 ones saved documents name,
fresh uids keep their form and stay unique, renamed tag tables shared by
deferred meshes are never edited through one of them, and the collector
stays off through link resolution without being left off."""
from __future__ import annotations

import built_core

import gc
import random
import uuid

import numpy as np
from built_core import Vec3 as QVector3D

from core import links
from core.layers import assign_layer, layer_of


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


# ---- stable_uid --------------------------------------------------------------

def _uuid5_uid(placement_uid, source_uid):
    """The definition the saved ids were made with (before 2026-11-13)."""
    return uuid.uuid5(uuid.NAMESPACE_URL, 'ingetrazo:link:v1:' + placement_uid
                      + ':' + source_uid).hex[:16]


def test_stable_uid_is_byte_identical_to_the_uuid5_definition():
    rng = random.Random(7)
    samples = [("", ""), ("a", "b"), ("élément", "ü€✓"), ("x" * 300, "y")]
    for _ in range(5000):
        samples.append((uuid.UUID(int=rng.getrandbits(128)).hex[:16],
                        uuid.UUID(int=rng.getrandbits(128)).hex[:16]))
    for p, s in samples:
        assert built_core.link_stable_uid(p, s) == _uuid5_uid(p, s), (p, s)


def test_stable_uid_known_value():
    # Pinned: a link copy's id is persisted and referenced by scenes.
    assert built_core.link_stable_uid("0123456789abcdef", "fedcba9876543210") == \
        "a3e4efbb4124549c"


# ---- new_uid -----------------------------------------------------------------

def test_new_uid_keeps_the_uuid4_form_and_is_unique():
    from core.group import new_uid
    seen = set()
    for _ in range(50_000):           # several pool refills
        u = new_uid()
        assert len(u) == 16 and all(c in "0123456789abcdef" for c in u)
        assert u[12] == "4"           # uuid4's version nibble, as before
        seen.add(u)
    assert len(seen) == 50_000


def test_a_forked_child_starts_with_an_empty_pool():
    from core import group
    group.new_uid()
    assert group._UID_POOL            # something pooled in this process
    group._drop_uid_pool()            # what os.register_at_fork runs
    assert not group._UID_POOL
    assert len(group.new_uid()) == 16  # refills on demand


def test_a_loaded_group_keeps_its_stored_uid_and_one_without_gets_a_fresh_one(tmp_path):
    import json
    from core.group import Group
    from core.scene import Scene
    from formats import igz
    s = Scene()
    a = Group(name="A")
    a.mesh.add_face([V(0, 0), V(1, 0), V(1, 1)])
    b = Group(name="B")
    s.groups += [a, b]
    path = tmp_path / "doc.igz"
    igz.save_scene(s, path)
    back = Scene()
    igz.load_into(back, path)
    assert [g.uid for g in back.groups] == [a.uid, b.uid]
    # An older document without uids (small documents are plain JSON):
    # each group gets a fresh, distinct one.
    doc = json.loads(path.read_text("utf-8"))
    for g in doc["scene"]["groups"]:
        g.pop("uid", None)
    old = tmp_path / "old.igz"
    old.write_text(json.dumps(doc), "utf-8")
    fresh = Scene()
    igz.load_into(fresh, old)
    uids = [g.uid for g in fresh.groups]
    assert len(set(uids)) == 2 and a.uid not in uids and b.uid not in uids
    assert all(len(u) == 16 for u in uids)


# ---- _populate ---------------------------------------------------------------

def test_populate_clones_with_stable_ids_and_copied_records():
    from core.group import Group
    src = Group(name="Source")
    child = Group(name="Child")
    child.ext = {"plugin": {"k": [1, 2]}}
    child.ifc = {"class": "IfcBeam",
                 "openings": [{"group_uid": "abc"}]}
    leaf = Group(name="Leaf")          # ext/ifc None
    child.children = [leaf]
    leaf.owner = child
    src.children = [child]
    child.owner = src
    placement = Group(name="Copy")
    built_core.link_populate(placement, src)
    (c,) = placement.children
    (l,) = c.children
    assert c.uid == _uuid5_uid(placement.uid, child.uid)
    assert l.uid == _uuid5_uid(placement.uid, leaf.uid)
    assert c.owner is placement and l.owner is c
    assert c.name == "Child" and c.mesh is child.mesh   # geometry shared
    assert c.ifc["openings"][0]["group_uid"] == _uuid5_uid(placement.uid, "abc")
    assert child.ifc["openings"][0]["group_uid"] == "abc"   # source untouched
    c.ext["plugin"]["k"].append(3)
    assert child.ext == {"plugin": {"k": [1, 2]}}
    assert l.ext is None and l.ifc is None
    # Every slot the source had is there, as copy.copy gave.
    from core.group import Group as G
    for name in G.__slots__:
        if name == "source_uid":
            continue                 # set on a link's parts (73ca3c85)
        assert hasattr(l, name) == hasattr(leaf, name), name
    assert c.source_uid == child.uid and l.source_uid == leaf.uid


def test_populate_keeps_an_empty_record_a_copy_of_its_own():
    from core.group import Group
    src = Group(name="Source")
    child = Group(name="Child")
    child.ext = {}
    src.children = [child]
    placement = Group(name="Copy")
    built_core.link_populate(placement, src)
    (c,) = placement.children
    assert c.ext == {} and c.ext is not child.ext


# ---- a definition renamed as it is read: tables shared ----------------------

def _deferred_pair():
    """Two deferred meshes with the SAME table objects, as a document's
    identical definitions can be."""
    from core.mesh import DeferredMesh, Mesh
    meta = {"face_attrs": [{"layer": "Wood"}], "edge_layers": ["Wood"]}

    def make():
        def build(m):
            f = m.add_face([V(0, 0), V(1, 0), V(1, 1), V(0, 1)])
            f.attrs = {"layer": "Wood"}
        m = Mesh()
        src = {"meta": meta,
               "counts": np.array([1], dtype=np.int32),
               "face_attr": np.array([0], dtype=np.int32),
               "edges": np.zeros((1, 2), dtype=np.int32),
               "layers": np.array([0], dtype=np.int32)}
        assert DeferredMesh.defer(m, build, 1, src)
        return m
    return make(), make(), meta


def _install(*meshes):
    """The meshes as one source's definition, renamed into a host."""
    from core.group import Group
    comp = Group(name="Site")
    parts = [Group(m, name=f"P{i}") for i, m in enumerate(meshes)]
    comp.children = parts
    for p in parts:
        p.owner = comp
    built_core.link_install_definition(comp, "/x/Site.igz", set(), {})
    return comp, parts


def test_meshes_with_one_table_share_the_renamed_one():
    a, b, meta = _deferred_pair()
    _comp, (ga, gb) = _install(a, b)
    ta = a.__dict__["_deferred_source"]["meta"]["face_attrs"]
    tb = b.__dict__["_deferred_source"]["meta"]["face_attrs"]
    assert ta is tb and ta[0]["layer"] == "Site / Wood"
    assert meta["face_attrs"][0]["layer"] == "Wood"      # source untouched
    # The per-mesh index arrays are each mesh's own.
    assert (a.__dict__["_deferred_source"]["face_attr"]
            is not b.__dict__["_deferred_source"]["face_attr"])
    assert layer_of(ga) == layer_of(gb) == "Site / Untagged"


def test_untagged_faces_do_not_share_a_table_with_tagged_ones():
    from core.mesh import DeferredMesh, Mesh
    meta = {"face_attrs": [{"layer": "Wood"}], "edge_layers": []}
    out = []
    for fa in ([0], [0, -1]):
        m = Mesh()
        src = {"meta": meta, "counts": np.ones(len(fa), dtype=np.int32),
               "face_attr": np.array(fa, dtype=np.int32)}
        assert DeferredMesh.defer(m, lambda m: None, len(fa), src)
        out.append(m)
    _install(*out)
    t0, t1 = (m.__dict__["_deferred_source"]["meta"]["face_attrs"] for m in out)
    assert [r["layer"] for r in t0] == ["Site / Wood"]
    assert [r["layer"] for r in t1] == ["Site / Wood", "Site / Untagged"]
    fa1 = out[1].__dict__["_deferred_source"]["face_attr"]
    assert [t1[i]["layer"] for i in fa1] == ["Site / Wood", "Site / Untagged"]


def test_editing_a_tag_in_one_mesh_never_changes_another():
    a, b, _meta = _deferred_pair()
    _install(a, b)
    table = b.__dict__["_deferred_source"]["meta"]["face_attrs"]
    # Build A and retag its face.
    (fa,) = a.faces
    assert layer_of(fa) == "Site / Wood"
    assign_layer(fa, "Other")
    assert table[0]["layer"] == "Site / Wood"
    # Repaint A's material while B is still deferred: B's table stays.
    built_core.link_restamp_materials(a, {"x": "y"})
    assert b.__dict__["_deferred_source"]["meta"]["face_attrs"] is table
    (fb,) = b.faces
    assert layer_of(fb) == "Site / Wood"
    assign_layer(fb, "Third")
    assert layer_of(fa) == "Other"


# ---- the collector during link resolution ------------------------------------

def test_links_resolve_with_the_collector_off_and_it_comes_back_on(tmp_path, monkeypatch):
    from core.layers import Layer
    from core.scene import Scene
    from formats import igz
    src = Scene()
    src.layers.append(Layer("Wood"))
    f = src.mesh.add_face([V(0, 0), V(1, 0), V(1, 1)])
    f.attrs = {"layer": "Wood"}
    igz.save_scene(src, tmp_path / "Site.igz")
    host = Scene()
    host.groups.append(links.make_link(host, tmp_path / "Site.igz",
                                       host_path=tmp_path / "host.igz"))
    igz.save_scene(host, tmp_path / "host.igz")
    states = []
    real = built_core.link_resolve

    def spy(*a, **k):
        states.append(gc.isenabled())
        return real(*a, **k)
    monkeypatch.setattr(built_core, "link_resolve", spy)
    was = gc.isenabled()
    gc.enable()
    try:
        back = Scene()
        igz.load_into(back, tmp_path / "host.igz")
        assert states and not states[0]
        assert gc.isenabled()
        assert not back.link_errors
    finally:
        if not was:
            gc.disable()
