# SPDX-License-Identifier: GPL-3.0-or-later
"""A linked file's definition is held by the document (the scene root),
one per file, subset and nested-link policy, and no placement owns it
(``core/links.py``, ``model/links.rs``): every placement of the file is
projected from it, a copy of a link is one more projection (no read),
its parts take their uids by rule, and a reload swaps the definition and
projects it again, keeping the parts it matches, so what holds them (the
selection, a dimension, Unlink's pairs) keeps holding them."""
from __future__ import annotations

import gc

import built_core
import pytest
from built_core import Vec3 as V

from core import links
from core.group import copy_group, iter_placements
from core.history import History, UnlinkCommand
from core.layers import Layer
from core.scene import Scene
from formats import igz
from tests.test_links_unlink import _source, _parts, _meshes


def _host(tmp_path):
    src = _source(tmp_path / "Site.igz")
    host = Scene()
    link = links.make_link(host, src, host_path=tmp_path / "host.igz")
    host.groups.append(link)
    return host, link, src


def _definition(group):
    return built_core.link_definition(group)


def _edit_tag_only(src):
    """The source saved again with a new tag: its parts unchanged."""
    s = Scene()
    igz.load_into(s, src)
    s.layers.append(Layer("Extra"))
    igz.save_scene(s, src)


def test_the_root_holds_the_definition_and_no_placement_owns_it(tmp_path):
    host, link, src = _host(tmp_path)
    comp, tags = _definition(link)
    assert comp is not link and comp not in host.groups
    assert comp._parent is None                     # in no scene list
    assert [c.uid for c in comp.children] == [p.source_uid for p in link.children]
    assert all(p.mesh is c.mesh for p, c in zip(link.children, comp.children))
    assert host._root._link_configs() == [
        built_core.link_definition_key(src.resolve(), None, True)]


def test_a_copy_is_projected_without_reading_the_file(tmp_path, monkeypatch):
    host, link, src = _host(tmp_path)
    from formats import igz as _igz
    monkeypatch.setattr(_igz, "load_into", lambda *a, **k: pytest.fail("read"))
    copy = copy_group(link, V(9, 0, 0))
    host.groups.append(copy)
    for part in _parts(copy):
        assert part.uid == built_core.link_stable_uid(copy.uid, part.source_uid)
        assert built_core.link_linked_ancestor(part) is copy
    # The same geometry as the link it copies, as every copy of a link.
    assert set(_meshes(copy)) == set(_meshes(link))
    assert [p.source_uid for p in _parts(copy)] == [p.source_uid for p in _parts(link)]


def test_a_reload_swaps_the_definition(tmp_path):
    host, link, src = _host(tmp_path)
    host.groups.append(copy_group(link, V(9, 0, 0)))
    old = _definition(link)
    _edit_tag_only(src)
    links.reload(host, src)
    new = _definition(link)[0]
    assert new is not old[0]                        # one definition, the new
    del old
    gc.collect()
    assert len(host._root._link_configs()) == 1
    for g in links.linked_groups(host):
        assert all(p.mesh is c.mesh for p, c in zip(g.children, new.children))


def test_what_holds_a_part_keeps_holding_it_through_a_reload(tmp_path):
    from core.dimension import Dimension
    host, link, src = _host(tmp_path)
    part = link.children[0]
    host.selection.add(part)
    dim = Dimension(V(0, 0, 1), V(3, 0, 1), V(0, -1, 0))
    host.dimensions.append(dim)
    dim.bind(host)
    assert dim.anchored
    _edit_tag_only(src)
    links.reload(host, src)
    assert link.children[0] is part and part in host.selection
    assert dim.anchored and dim.anchor_a.alive(host)
    # Unlink after the reload: the dimension follows the copy.
    History(host).execute(UnlinkCommand([link]))
    assert dim.anchored and id(dim.anchor_a.mesh) in set(_meshes(link))


def test_unlinking_the_last_placement_releases_the_definition(tmp_path):
    host, link, src = _host(tmp_path)
    History(host).execute(UnlinkCommand([link]))
    assert host._root._link_configs() == []
    assert _definition(link) is None


def test_a_link_read_back_from_its_file_is_projected_on_open(tmp_path):
    host, link, src = _host(tmp_path)
    path = tmp_path / "host.igz"
    igz.save_scene(host, path)
    back = Scene()
    igz.load_into(back, path)
    (g,) = links.linked_groups(back)
    comp, _tags = _definition(g)
    assert [c.uid for c in comp.children] == [p.source_uid for p in g.children]
