# SPDX-License-Identifier: GPL-3.0-or-later
"""Make Unique of a linked file's placement breaks the link and keeps the
geometry (core.history.UnlinkCommand).

The placement becomes the host's own component: meshes of its own (none
shared with the source or with the file's other placements), parts that
are editable, saved in the host and no longer read from the source — and
it looks exactly as it did: the same matrices, paint, tags and tag
visibility, the parts under the same uids. The file's other placements
stay live links. When none is left, the source leaves the host's link
registry (nothing watches it), and its tags move out of «Linked files» to
the top of the tags tree as the host's own. One
undo brings the live link back exactly; redo breaks it again."""
from __future__ import annotations

import ingetrazo_core
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest  # noqa: E402
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V
from tests.native_tree import action
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from core import _native, links  # noqa: E402
from core.group import Group, copy_group, iter_placements  # noqa: E402
from core.history import History, MakeUniqueCommand, UnlinkCommand  # noqa: E402
from core.layers import Layer, layer_of  # noqa: E402
from core.materials import Material  # noqa: E402
from core.mesh import DeferredMesh, Mesh  # noqa: E402
from core.purge import iter_groups  # noqa: E402
from core.scene import Scene  # noqa: E402
from formats import igz  # noqa: E402


# ---- a small source -------------------------------------------------------

def _square(x=0.0, y=0.0, z=0.0, n=1):
    m = Mesh()
    for i in range(n):
        m.add_face([V(x + i, y, z), V(x + i + 1, y, z), V(x + i + 1, y + 1, z),
                    V(x + i, y + 1, z)])
    return m


def _source(path):
    """An assembly holding a part placed twice (one definition), painted
    with a PBR material and tagged, and a classic group beside it."""
    s = Scene()
    s.layers.append(Layer("Wood"))
    s.layers.append(Layer("Steel"))
    oak = s.materials["Oak"] = Material(
        "Oak", color=(0.6, 0.4, 0.2),
        pbr={"roughness": 0.7, "metallic": 0.1})
    proto = _square(z=1.0, n=3)
    for f in proto.faces:
        f.attrs.update(oak.face_attrs())
        f.attrs["layer"] = "Wood"
    parts = []
    for dx in (0.0, 5.0):
        part = Group(proto, name="part")
        part.xform = QMatrix4x4()
        part.xform.translate(dx, 0, 0)
        part.layer = "Steel"
        parts.append(part)
    asm = Group(_square(n=2), name="asm")
    asm.adopt(parts)
    asm.material = {"color": [0.1, 0.2, 0.3]}
    s.groups.append(asm)
    s.groups.append(Group(_square(x=3.0), name="classic"))
    s.mesh.add_face([V(0, 9, 0), V(1, 9, 0), V(1, 10, 0)])
    igz.save_scene(s, path)
    return path


def _host(tmp_path, packed=False):
    src = _source(tmp_path / "Site.igz")
    host = Scene()
    link = links.make_link(host, src, host_path=tmp_path / "host.igz")
    host.groups.append(link)
    other = copy_group(link, V(20, 0, 0))
    host.groups.append(other)
    if packed:
        ingetrazo_core.link_set_packed(host, src, True)
    host.version += 1
    return host, link, other, src


def _parts(g):
    return list(iter_groups(g.children or ()))


def _meshes(g):
    return {id(p.mesh): p.mesh for p, _m in iter_placements(g)}


def _key(p):
    return tuple(round(c, 6) for c in (p.x(), p.y(), p.z()))


def _look(g, uids=True):
    """What ``g`` shows, in the world: each face's corners and attributes
    (paint, PBR, tag), with the placement's own tag and paint; each edge's
    ends, tag and flags. ``uids=False`` leaves the parts' uids out (two
    links of one file)."""
    out = []
    for placed, m in iter_placements(g):
        def world(p):
            return _key(m.map(p) if m is not None else p)
        # The root's name aside: Make Unique makes it free («Site #2»).
        who = (placed.uid if uids else "",
               "" if placed is g else placed.name, placed.layer or "",
               repr(placed.material))
        for f in placed.mesh.faces:
            a = {k: v for k, v in (f.attrs or {}).items() if k != "layer"}
            out.append(("f", who,
                        tuple(sorted(world(v) for v in f.vertices)),
                        repr(sorted(a.items())), layer_of(f)))
        for e in placed.mesh.edges:
            out.append(("e", who, tuple(sorted((world(e.a), world(e.b)))),
                        repr(layer_of(e)), bool(getattr(e, "soft", False)),
                        bool(getattr(e, "hidden", False))))
    return sorted(out)


def assert_live(g, src):
    assert links.is_linked(g)
    assert links.link_of(g)["path"] == str(src)
    for p in _parts(g):
        assert ingetrazo_core.link_linked_ancestor(p) is g
        if hasattr(p, "source_uid"):      # not on a packed link read back
            assert p.uid == ingetrazo_core.link_stable_uid(g.uid, p.source_uid)


def assert_own(g, others):
    """``g`` is the host's own: no link, parts editable, no mesh shared
    with ``others`` (the source's other placements)."""
    assert not links.is_linked(g)
    assert g.is_component()
    for p in _parts(g):
        assert ingetrazo_core.link_linked_ancestor(p) is None
        assert p.owner is None
        assert not hasattr(p, "source_uid")
        assert not links.is_linked(p)
    theirs = {k for o in others for k in _meshes(o)}
    assert not set(_meshes(g)) & theirs


def _edit_source(src):
    """The source opened, two faces drawn in it, and saved."""
    s = Scene()
    igz.load_into(s, src)
    for i in range(2):
        s.mesh.add_face([V(i, 5, 0), V(i + 1, 5, 0), V(i + 1, 6, 0), V(i, 6, 0)])
    igz.save_scene(s, src)


def _faces(g):
    return sum(len(p.mesh.faces) for p, _m in iter_placements(g))


# ---- 1. the command ---------------------------------------------------------

@pytest.mark.parametrize("packed", [False, True])
def test_unlink_keeps_the_look_and_owns_its_geometry(tmp_path, packed):
    host, link, other, src = _host(tmp_path, packed)
    look, other_look = _look(link), _look(other)
    before_parts = _parts(link)
    before_meshes = [p.mesh for p in before_parts]
    uids = [p.uid for p in before_parts]
    history = History(host)
    history.execute(UnlinkCommand([link]))
    assert history.undo_stack and history.last_error is None
    assert_own(link, [other])
    assert _look(link) == look                       # looks as it did
    assert [p.uid for p in _parts(link)] == uids     # same keys
    # Two placements of one part in the source still share their copy.
    a, b = [p for p in _parts(link) if p.name == "part"]
    assert a.mesh is b.mesh
    assert_live(other, src)
    assert _look(other) == other_look
    # The source still linked by ``other``: still registered, still watched.
    assert links.sources(host) == [ingetrazo_core.link_norm(src)]
    assert ingetrazo_core.link_is_packed(host, src) == packed
    after_parts = _parts(link)

    history.undo()
    assert_live(link, src)
    assert _parts(link) == before_parts              # the very same objects
    assert [p.mesh for p in _parts(link)] == before_meshes
    assert _look(link) == look
    history.redo()
    assert_own(link, [other])
    assert _parts(link) == after_parts
    assert _look(link) == look


def test_the_unlinked_content_is_editable_and_the_others_are_not_touched(tmp_path):
    host, link, other, src = _host(tmp_path)
    History(host).execute(UnlinkCommand([link]))
    part = next(p for p in _parts(link) if p.name == "classic")
    theirs = next(p for p in _parts(other) if p.name == "classic")
    n = len(theirs.mesh.faces)
    part.mesh.add_face([V(0, 0, 3), V(1, 0, 3), V(1, 1, 3)])
    assert len(theirs.mesh.faces) == n


def test_the_name_is_made_free_like_make_unique(tmp_path):
    host, link, other, src = _host(tmp_path)
    assert link.name == other.name == "Site"
    History(host).execute(UnlinkCommand([link]))
    assert link.name == "Site #2" and other.name == "Site"


def test_unlinking_both_placements_in_one_step(tmp_path):
    host, link, other, src = _host(tmp_path)
    looks = _look(link), _look(other)
    history = History(host)
    history.execute(UnlinkCommand([link, other]))
    assert_own(link, [other])
    assert_own(other, [link])
    # The last placements: the file's «Untagged» is the host's now.
    assert (_look(link), _look(other)) == tuple(_released(x, src) for x in looks)
    assert links.sources(host) == []
    history.undo()
    assert_live(link, src)
    assert_live(other, src)
    assert links.sources(host) == [ingetrazo_core.link_norm(src)]


def test_a_part_or_a_host_component_is_not_unlinked(tmp_path):
    host, link, other, src = _host(tmp_path)
    plain = Group(_square(), name="plain")
    plain.xform = QMatrix4x4()
    cmd = UnlinkCommand([_parts(link)[0], plain])
    assert cmd.groups == []
    history = History(host)
    history.execute(cmd)
    assert not history.undo_stack
    assert_live(link, src)


def test_deferred_meshes_copy_without_building_and_build_the_same(
        tmp_path, monkeypatch):
    """A mesh the document left deferred is copied deferred (nothing is
    built for Make Unique), and built later it shows what the link did."""
    monkeypatch.setattr(_native, "wanted", lambda: True)
    (tmp_path / "a").mkdir()
    (tmp_path / "b").mkdir()
    ref_host, ref, _o, _s = _host(tmp_path / "a")
    reference = _look(ref, uids=False)        # the link's look, built
    host, link, other, src = _host(tmp_path / "b")
    deferred = [m for m in _meshes(link).values() if type(m) is DeferredMesh]
    assert deferred, "the source's meshes load deferred"
    History(host).execute(UnlinkCommand([link]))
    assert all(type(m) is DeferredMesh for m in _meshes(link).values())
    assert all(type(m) is DeferredMesh for m in deferred)   # nor the source's
    assert_own(link, [other])
    assert _look(link, uids=False) == reference
    # Building the copies left the link's shared meshes alone.
    assert all(type(m) is DeferredMesh for m in deferred)
    assert _look(other, uids=False) != reference      # elsewhere: 20 m away
    assert len(_look(other)) == len(reference)


def test_a_dimension_on_the_link_follows_its_own_copy(tmp_path):
    from core.dimension import Dimension
    host, link, other, src = _host(tmp_path)
    dim = Dimension(V(0, 0, 1), V(3, 0, 1), V(0, -1, 0))
    host.dimensions.append(dim)
    dim.bind(host)
    assert dim.anchored
    history = History(host)
    history.execute(UnlinkCommand([link]))
    mine = set(_meshes(link))
    assert dim.anchored and id(dim.anchor_a.mesh) in mine
    assert dim.anchor_a.chain[0] is link
    link.xform.translate(0, 0, 2)          # the dimension goes with it
    host.version += 1
    assert _key(dim.a) == (0.0, 0.0, 3.0) and _key(dim.b) == (3.0, 0.0, 3.0)
    link.xform.translate(0, 0, -2)
    history.undo()
    assert _key(dim.a) == (0.0, 0.0, 1.0)
    assert dim.anchor_a.alive(host)
    assert id(dim.anchor_a.mesh) not in mine


def _nested_host(tmp_path):
    """``Mid.igz`` links ``Leaf.igz`` (a face on its tag «Steel»); the host
    places Mid twice."""
    leaf = Scene()
    leaf.layers.append(Layer("Steel"))
    leaf.mesh.add_face([V(0, 0, 7), V(1, 0, 7), V(1, 1, 7)])
    for f in leaf.mesh.faces:
        f.attrs["layer"] = "Steel"
    leaf_path = tmp_path / "Leaf.igz"
    igz.save_scene(leaf, leaf_path)
    mid = Scene()
    mid.groups.append(links.make_link(mid, leaf_path))
    mid.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    mid_path = tmp_path / "Mid.igz"
    igz.save_scene(mid, mid_path)
    host = Scene()
    link = links.make_link(host, mid_path)
    host.groups.append(link)
    other = copy_group(link, V(9, 0, 0))
    host.groups.append(other)
    return host, link, other, mid_path, leaf_path


def _shape(g):
    """What ``g`` shows in the world, tags and uids aside."""
    return sorted(row[2] for row in _look(g) if row[0] == "f")


def _tags_and_registry(scene):
    from copy import deepcopy
    return _tag_state(scene), deepcopy(scene.link_registry), \
        dict(scene.layer_folder_names), sorted(scene.materials)


def test_a_file_linked_inside_the_source_stays_a_live_link(tmp_path):
    host, link, other, mid_path, leaf_path = _nested_host(tmp_path)
    (inner,) = [p for p in _parts(link) if links.is_linked(p)]
    mirror = host.layer_by_key("Mid / Leaf / Steel")      # Mid's copy of it
    mirror.visible = False                                 # switched here
    shape = _shape(link)
    history = History(host)
    history.execute(UnlinkCommand([link]))
    assert not links.is_linked(link)
    # Mid's own geometry is the host's; Leaf, linked inside it, stays a link
    # of the host's own: at the top of its chain, its parts read-only and
    # under the uids a reload of Leaf gives them.
    (nested,) = [p for p in _parts(link) if links.is_linked(p)]
    assert links.link_of(nested)["path"] == str(leaf_path)
    assert ingetrazo_core.link_linked_ancestor(nested.owner) is None
    assert nested.uid == inner.uid
    leaf_parts = _parts(nested)
    for p in _parts(link):
        if p is nested or p in leaf_parts:
            continue
        assert ingetrazo_core.link_linked_ancestor(p) is None and p.owner is None
    for p, _m in iter_placements(nested):
        if p is not nested:
            assert ingetrazo_core.link_linked_ancestor(p) is nested
            assert p.uid == ingetrazo_core.link_stable_uid(nested.uid, p.source_uid)
    assert _shape(link) == shape                        # looks as it did
    # Leaf's tags are the host's link of Leaf now: its own folder in
    # «Linked files», the switch made on Mid's copy kept as an override.
    steel = host.layer_by_key(ingetrazo_core.link_tag_name(leaf_path, "Steel"))
    assert steel is not None and steel.folder == "Leaf"
    assert host.folder_parents["Leaf"] == links.LINKED_FOLDER
    assert steel.visible is False
    entry = host.link_registry[ingetrazo_core.link_norm(leaf_path)]
    assert entry["overrides"] == {steel.key: False}
    face_tags = {layer_of(f) for p, _m in iter_placements(nested)
                 for f in p.mesh.faces}
    assert face_tags == {steel.key}
    # Still watched (and listed): both files are linked.
    assert sorted(links.sources(host)) == sorted(
        [ingetrazo_core.link_norm(mid_path), ingetrazo_core.link_norm(leaf_path)])
    rows = [r for r in ingetrazo_core.link_query(host) if r["uid"] == nested.uid]
    assert rows and rows[0]["status"] == "resolved"
    # It follows its file.
    n, n_other = _faces(nested), _faces(other)
    _edit_source(leaf_path)
    links.reload(host, leaf_path)
    assert _faces(nested) == n + 2
    assert _faces(other) == n_other + 2       # through Mid, as before



def test_undo_and_redo_around_a_nested_link_are_exact(tmp_path):
    host, link, other, mid_path, leaf_path = _nested_host(tmp_path)
    before = list(link.children)
    parts = _parts(link)
    state = _tags_and_registry(host)
    history = History(host)
    history.execute(UnlinkCommand([link]))
    after = _parts(link)
    after_state = _tags_and_registry(host)
    history.undo()
    assert link.children == before and _parts(link) == parts
    assert links.is_linked(link)
    assert all(ingetrazo_core.link_linked_ancestor(p.owner) is link for p in link.children)
    assert _tags_and_registry(host) == state
    history.redo()
    assert _parts(link) == after                 # the very same objects
    (nested,) = [p for p in _parts(link) if links.is_linked(p)]
    assert ingetrazo_core.link_linked_ancestor(nested.owner) is None
    assert _tags_and_registry(host) == after_state
    history.undo()
    assert _tags_and_registry(host) == state


def test_a_nested_file_that_cannot_be_read_keeps_what_it_showed(tmp_path):
    host, link, other, mid_path, leaf_path = _nested_host(tmp_path)
    (inner,) = [p for p in _parts(link) if links.is_linked(p)]
    shape, inner_meshes = _shape(link), _meshes(inner)
    os.remove(leaf_path)
    history = History(host)
    history.execute(UnlinkCommand([link]))
    (nested,) = [p for p in _parts(link) if links.is_linked(p)]
    assert ingetrazo_core.link_linked_ancestor(nested.owner) is None
    assert all(ingetrazo_core.link_linked_ancestor(p) is nested for p in _parts(nested))
    assert _shape(link) == shape
    assert set(_meshes(nested)) <= set(inner_meshes) | {id(nested.mesh)}
    history.undo()
    assert _parts(link)[0].owner is link


def test_a_nested_link_in_the_window_is_watched_and_listed(native_host, tmp_path):
    win = native_host.window
    host_scene = win.viewport.scene
    _h, _l, _o, mid_path, leaf_path = _nested_host(tmp_path)
    link = links.make_link(host_scene, mid_path, host_path=tmp_path / "host.igz")
    host_scene.groups.append(link)
    host_scene.version += 1
    watcher = win._link_watcher()
    watcher.refresh()
    watched = lambda: sorted(os.path.normcase(p) for p in watcher.paths)
    both = sorted([ingetrazo_core.link_norm(mid_path), ingetrazo_core.link_norm(leaf_path)])
    assert watched() == both
    host_scene.select([link])
    win._on_make_unique()
    assert watched() == [ingetrazo_core.link_norm(leaf_path)]
    from tests.test_tree_links import _View
    from views import wgpu_ui
    view = _View()
    wgpu_ui._tree(view, win)
    assert [n["path"] for n in view._ui_tree["nodes"].values() if n["kind"] == "link"] == [str(leaf_path)]
    win._on_undo()
    assert watched() == both
    win._on_redo()
    assert watched() == [ingetrazo_core.link_norm(leaf_path)]


# ---- 2. tags and the registry ------------------------------------------------

def _tag_state(scene):
    return ([(ly.key, ly.name, ly.visible, ly.folder, repr(ly.color))
             for ly in scene.layers],
            list(scene.layer_folders), dict(scene.folder_parents),
            dict(scene.folder_visibility))


def _released(look, src):
    """``look`` with the file's «Untagged» read as the host's Untagged:
    what a last unlinked placement wears (they draw the same)."""
    from core.layers import DEFAULT_LAYER
    u = ingetrazo_core.link_tag_name(src, None)
    swap = {u: DEFAULT_LAYER, repr(u): repr(DEFAULT_LAYER)}
    who = {u: ""}                       # a placement's own tag: none
    return sorted(tuple(swap.get(x, x) if isinstance(x, str) else
                        tuple(who.get(y, y) for y in x) if isinstance(x, tuple)
                        and len(x) == 4 else x for x in row)
                  for row in look)


def _full_tag_state(scene):
    from copy import deepcopy
    return (_tag_state(scene), dict(scene.layer_folder_names),
            dict(scene.layer_folder_uids), deepcopy(scene.link_registry))


def _foldered_source(path):
    """A source whose tags sit in folders: «Structure» holding «Beams»
    and a subfolder «Steel» holding «Columns»; «Wood» at its top."""
    s = Scene()
    s.layer_folders = ["Structure", "Steel"]
    s.folder_parents = {"Steel": "Structure"}
    s.layers.append(Layer("Wood", color=(0.5, 0.3, 0.1)))
    s.layers.append(Layer("Beams", folder="Structure", color=(0.1, 0.2, 0.9)))
    s.layers.append(Layer("Columns", folder="Steel", visible=False))
    for i, tag in enumerate(("Wood", "Beams", "Columns", None)):
        m = _square(x=2.0 * i)
        for f in m.faces:
            if tag:
                f.attrs["layer"] = tag
        g = Group(m, name=f"piece {i}")
        g.xform = QMatrix4x4()
        s.groups.append(g)
    igz.save_scene(s, path)
    return path


def _one_link(tmp_path, source=_foldered_source, host=None):
    src = source(tmp_path / "Site.igz")
    host = host or Scene()
    link = links.make_link(host, src, host_path=tmp_path / "host.igz")
    host.groups.append(link)
    host.version += 1
    return host, link, src


def _faces_on(g, key):
    return sum(1 for p, _m in iter_placements(g) for f in p.mesh.faces
               if layer_of(f) == key)


def test_the_last_placement_moves_its_tags_to_the_top(tmp_path):
    host, link, src = _one_link(tmp_path)
    key = lambda tag: ingetrazo_core.link_tag_name(src, tag)
    host.layer_by_key(key("Wood")).visible = False      # switched in the host
    assert ingetrazo_core.link_note_visibility(host, key("Wood"), False)
    before = _full_tag_state(host)
    entry = dict(host.link_registry[ingetrazo_core.link_norm(src)])
    shown = {ly.key: (ly.name, ly.visible, repr(ly.color)) for ly in host.layers}
    look = _look(link)
    history = History(host)
    history.execute(UnlinkCommand([link]))
    assert host.link_registry == {}
    assert ingetrazo_core.link_source_of_tag(host, key("Wood")) is None     # the host's own
    # The same tags, by key, label, colour and visibility; out at the top.
    for tag in ("Wood", "Beams", "Columns"):
        ly = host.layer_by_key(key(tag))
        assert (ly.name, ly.visible, repr(ly.color)) == shown[ly.key]
    assert host.layer_by_key(key("Wood")).folder is None
    # The file's folders came along as top-level folders, nested as they were.
    structure = host.layer_by_key(key("Beams")).folder
    steel = host.layer_by_key(key("Columns")).folder
    assert structure == key("Structure") and steel == key("Steel")
    assert host.folder_parents.get(structure) is None
    assert host.folder_parents[steel] == structure
    assert (ingetrazo_core.tag_edits.folder_label(host, structure), ingetrazo_core.tag_edits.folder_label(host, steel)) == (
        "Structure", "Steel")
    # Neither «Linked files» nor the file's folder is left.
    assert links.LINKED_FOLDER not in host.layer_folders
    assert "Site" not in host.layer_folders
    # The file's Untagged is the host's Untagged; the rest looks the same.
    assert host.layer_by_key(key(None)) is None
    assert _look(link) == _released(look, src)
    assert not ingetrazo_core.link_note_visibility(host, key("Wood"), True)
    after = _full_tag_state(host)

    history.undo()
    assert _full_tag_state(host) == before
    assert host.link_registry[ingetrazo_core.link_norm(src)] == entry
    assert ingetrazo_core.link_source_of_tag(host, key("Wood")) == ingetrazo_core.link_norm(src)
    assert _look(link) == look
    history.redo()
    assert _full_tag_state(host) == after
    assert _look(link) == _released(look, src)


def test_tags_stay_in_their_folder_while_another_placement_links_the_file(tmp_path):
    host, link, src = _one_link(tmp_path)
    other = copy_group(link, V(20, 0, 0))
    host.groups.append(other)
    before = _full_tag_state(host)
    history = History(host)
    history.execute(UnlinkCommand([link]))
    assert _tag_state(host) == before[0]
    assert host.layer_by_key(ingetrazo_core.link_tag_name(src, "Beams")).folder != None
    history.execute(UnlinkCommand([other]))                   # the last one
    assert host.layer_by_key(ingetrazo_core.link_tag_name(src, "Wood")).folder is None
    assert links.LINKED_FOLDER not in host.layer_folders
    history.undo()
    history.undo()
    assert _full_tag_state(host) == before


@pytest.mark.parametrize("deferred", [False, True])
def test_a_tag_and_folder_named_as_the_hosts_merge_into_them(
        tmp_path, monkeypatch, deferred):
    from core.layers import folders_of
    monkeypatch.setattr(_native, "wanted", lambda: deferred)
    host = Scene()
    host.layers.append(Layer("Wood", color=(0.9, 0.9, 0.0), visible=False))
    host.layer_folders = ["Structure"]
    host.layers.append(Layer("Beams", folder="Structure", color=(0.0, 1.0, 0.0)))
    folders_of(host)
    host, link, src = _one_link(tmp_path, host=host)
    key = lambda tag: ingetrazo_core.link_tag_name(src, tag)
    before = _full_tag_state(host)
    old_parts = _parts(link)
    history = History(host)
    history.execute(UnlinkCommand([link]))
    if deferred:          # relabelled in their tables, not built for it
        assert all(type(m) is DeferredMesh for m in _meshes(link).values()
                   if m is not link.mesh)
    n_wood, n_beams = 1, 1
    # «Wood» is the host's «Wood», which keeps its colour and visibility.
    assert host.layer_by_key(key("Wood")) is None
    assert _faces_on(link, "Wood") == n_wood
    wood = host.layer_by_key("Wood")
    assert wood.color == (0.9, 0.9, 0.0) and wood.visible is False
    # «Structure» is the host's folder: its «Beams» merged, «Steel» inside it.
    assert key("Structure") not in folders_of(host)
    assert host.layer_by_key(key("Beams")) is None
    assert _faces_on(link, "Beams") == n_beams
    assert host.folder_parents[key("Steel")] == "Structure"
    assert host.layer_by_key(key("Columns")).folder == key("Steel")
    history.undo()
    assert _full_tag_state(host) == before
    assert _parts(link) == old_parts
    assert _faces_on(link, key("Wood")) == n_wood          # untouched copies
    history.redo()
    assert _faces_on(link, "Wood") == n_wood
    assert host.layer_by_key(key("Wood")) is None


def test_a_tag_worn_outside_the_link_is_not_merged_away(tmp_path):
    host = Scene()
    host.layers.append(Layer("Wood"))
    host, link, src = _one_link(tmp_path, host=host)
    mine = Group(_square(x=50), name="mine")
    mine.xform = QMatrix4x4()
    mine.layer = ingetrazo_core.link_tag_name(src, "Wood")       # the host's own, on it
    host.groups.append(mine)
    History(host).execute(UnlinkCommand([link]))
    ly = host.layer_by_key(ingetrazo_core.link_tag_name(src, "Wood"))
    assert ly is not None and ly.folder is None
    assert _faces_on(link, ly.key)


def test_a_hidden_file_folder_leaves_its_tags_hidden(tmp_path):
    host, link, src = _one_link(tmp_path)
    host.folder_visibility["Site"] = False
    look = _look(link)
    History(host).execute(UnlinkCommand([link]))
    # What came out to the top is hidden: a tag itself, a folder as a folder.
    assert host.layer_by_key(ingetrazo_core.link_tag_name(src, "Wood")).visible is False
    assert host.folder_visibility[ingetrazo_core.link_tag_name(src, "Structure")] is False
    assert host.layer_by_key(ingetrazo_core.link_tag_name(src, "Beams")).visible is True
    assert _look(link) == _released(look, src)


def test_the_nested_files_copies_of_their_tags_go(tmp_path):
    """Mid's own copies of Leaf's tags («Mid / Leaf / Steel», in Mid's
    «Linked files») are worn by nothing once Leaf is the host's own link."""
    from core.layers import folders_of
    host, link, other, mid_path, leaf_path = _nested_host(tmp_path)
    host.groups.remove(other)
    host.version += 1
    History(host).execute(UnlinkCommand([link]))
    assert host.layer_by_key("Mid / Leaf / Steel") is None
    assert not [f for f in folders_of(host) if f.startswith("Mid")]
    # Leaf's own tags, as a link of the host's, stay where links' tags go.
    steel = host.layer_by_key(ingetrazo_core.link_tag_name(leaf_path, "Steel"))
    assert steel.folder == "Leaf"
    assert host.folder_parents["Leaf"] == links.LINKED_FOLDER


# ---- 3. the window ---------------------------------------------------------

def _place(win, tmp_path):
    src = _source(tmp_path / "Site.igz")
    scene = win.viewport.scene
    link = links.make_link(scene, src, host_path=tmp_path / "host.igz")
    scene.groups.append(link)
    other = copy_group(link, V(20, 0, 0))
    scene.groups.append(other)
    scene.version += 1
    win._link_watcher().refresh()
    return scene, link, other, src


def test_make_unique_on_a_link_unlinks_it(native_host, tmp_path, monkeypatch):
    win = native_host.window
    scene, link, other, src = _place(win, tmp_path)
    vp = win.viewport
    shown = []
    monkeypatch.setattr(vp, "flash_status", lambda text, *a: shown.append(text))
    look = _look(link)
    scene.select([link])
    win._on_make_unique()
    assert isinstance(vp.history.undo_stack[-1], UnlinkCommand)
    assert shown and "Site.igz" in shown[-1] and "no longer linked" in shown[-1]
    assert_own(link, [other])
    assert _look(link) == look
    assert_live(other, src)
    # Going into it now edits it (no offer to open the source).
    vp.begin_group_edit(link)
    assert scene.edit_group is link
    vp.end_group_edit()
    win._on_undo()
    assert_live(link, src)
    win._on_redo()
    assert_own(link, [other])


def test_the_last_placement_is_no_longer_watched(native_host, tmp_path):
    win = native_host.window
    scene, link, other, src = _place(win, tmp_path)
    scene.groups.remove(other)
    scene.version += 1
    watcher = win._link_watcher()
    watcher.refresh()
    assert [os.path.normcase(p) for p in watcher.paths] == [ingetrazo_core.link_norm(src)]
    scene.select([link])
    win._on_make_unique()
    assert watcher.paths == []
    win._on_undo()
    assert [os.path.normcase(p) for p in watcher.paths] == [ingetrazo_core.link_norm(src)]
    win._on_redo()
    assert watcher.paths == []


def test_make_unique_of_what_holds_a_link_keeps_the_link(native_host, tmp_path):
    win = native_host.window
    scene, link, other, src = _place(win, tmp_path)
    holder = Group(Mesh(), name="holder")
    holder.adopt([link])
    scene.groups = [holder, other]
    scene.version += 1
    scene.select([holder])
    win._on_make_unique()
    assert isinstance(win.viewport.history.undo_stack[-1], MakeUniqueCommand)
    (inner,) = holder.children
    assert_live(inner, src)


def test_make_unique_from_the_browser_unlinks_too(native_host, tmp_path, monkeypatch):
    win = native_host.window
    scene, link, other, src = _place(win, tmp_path)
    plain = Group(_square(x=40), name="Silla")
    plain.xform = QMatrix4x4()
    scene.groups.append(plain)
    scene.version += 1
    vp = win.viewport
    shown = []
    monkeypatch.setattr(vp, "flash_status", lambda text, *a: shown.append(text))
    scene.select([link, plain])
    look, proto = _look(link), plain.mesh
    steps = len(vp.history.undo_stack)
    action(native_host, "unique", "i:" + plain.uid)
    assert len(vp.history.undo_stack) == steps + 1          # one step
    assert_own(link, [other])
    assert _look(link) == look and plain.mesh is not proto
    assert shown and "no longer linked" in shown[-1]
    win._on_undo()
    assert_live(link, src) and plain.mesh is proto
    win._on_redo()
    assert_own(link, [other])


# ---- 4. the Metal view -------------------------------------------------------

def test_metal_view_updates_in_place_and_draws_the_same(tmp_path, monkeypatch):
    from tests.test_wgpu_updates import _prototype, assert_matches_fresh_pack
    from views.wgpu_scene import pack_scene
    from views.wgpu_updates import InstanceUpdates
    host, link, other, src = _host(tmp_path)
    before = pack_scene(host)
    updates = InstanceUpdates(host, pack_scene(host))
    history = History(host)
    updates.observe(history)
    history.execute(UnlinkCommand([link]))
    ids, rows = updates.diff(host)                 # not None: no full pack
    assert updates.last_local                      # only the link's root
    assert updates.removed == [] and updates.added == 0
    # Only the unlinked placement's rows move, onto its own meshes, each
    # baked once; the other placement's rows are not written.
    mine = {updates.resident.row(p.uid) for p, _m in iter_placements(link)
            if updates.resident.row(p.uid) is not None}
    assert set(ids.tolist()) <= mine
    assert {id(m) for m, _s, _c in updates.changed} <= set(_meshes(link))
    updates.apply(ids, rows)
    assert_matches_fresh_pack(updates, host)
    after = pack_scene(host)
    # Pack equality: every row draws the same geometry, paint and flags.
    assert sorted(before.uids) == sorted(after.uids)
    for uid in before.uids:
        b, a = before.uids.index(uid), after.uids.index(uid)
        assert (before.instances[b]["world"] == after.instances[a]["world"]).all()
        assert (before.instances[b]["info"][1:] == after.instances[a]["info"][1:]).all()
        assert (_prototype(before, int(before.instances[b]["info"][0]))
                == _prototype(after, int(after.instances[a]["info"][0])))
    # Undo: back on the shared prototypes still resident, no bake.
    history.undo()
    ids, rows = updates.diff(host)
    assert updates.last_local
    assert updates.changed == []
    updates.apply(ids, rows)
    assert_matches_fresh_pack(updates, host)


# ---- 5. save and reopen ------------------------------------------------------

@pytest.mark.parametrize("packed", [False, True])
def test_save_reopen_and_the_source_no_longer_reaches_it(tmp_path, packed):
    host, link, other, src = _host(tmp_path, packed)
    History(host).execute(UnlinkCommand([link]))
    look, uids = _look(link), [p.uid for p in _parts(link)]
    path = tmp_path / "host.igz"
    igz.save_scene(host, path)
    back = Scene()
    igz.load_into(back, path)
    assert back.link_errors == {}
    (mine,) = [g for g in back.groups if g.uid == link.uid]
    (theirs,) = [g for g in back.groups if g.uid == other.uid]
    assert not links.is_linked(mine)
    assert all(ingetrazo_core.link_linked_ancestor(p) is None for p in _parts(mine))
    assert [p.uid for p in _parts(mine)] == uids
    assert _look(mine) == look
    assert_live(theirs, src)
    n_mine, n_theirs = _faces(mine), _faces(theirs)
    _edit_source(src)
    if packed:
        ingetrazo_core.link_set_packed(back, src, False)
    links.reload(back, src)
    assert _faces(mine) == n_mine                    # no longer follows
    assert _faces(theirs) == n_theirs + 2            # still follows
    assert _look(mine) == look


@pytest.mark.parametrize("packed", [False, True])
def test_a_link_read_back_from_the_file_unlinks_too(tmp_path, monkeypatch, packed):
    """Opened again (packed: the parts come from the host file, deferred),
    the link unlinks like a fresh one, and the result saves as content."""
    monkeypatch.setattr(_native, "wanted", lambda: True)
    host, link, other, src = _host(tmp_path, packed)
    path = tmp_path / "host.igz"
    igz.save_scene(host, path)
    back = Scene()
    igz.load_into(back, path)
    (theirs,) = [g for g in back.groups if g.uid == link.uid]
    (rest,) = [g for g in back.groups if g.uid == other.uid]
    history = History(back)
    history.execute(UnlinkCommand([theirs]))
    assert_own(theirs, [rest])
    look = _look(theirs)
    history.undo()
    assert _look(theirs) == look
    assert_live(theirs, src)
    history.redo()
    igz.save_scene(back, path)
    again = Scene()
    igz.load_into(again, path)
    (mine,) = [g for g in again.groups if g.uid == link.uid]
    assert not links.is_linked(mine) and _look(mine) == look


def test_undo_leaves_independent_root_table_edits_in_place(tmp_path):
    host, link, other, _src = _host(tmp_path)
    h = History(host)
    h.execute(UnlinkCommand([link, other]))
    host.plugin_data["unrelated"] = {"revision": 7}
    host.layer_folders.append("Independent folder")
    for step in (h.undo, h.redo):
        assert step()
        assert host.plugin_data["unrelated"] == {"revision": 7}
        assert "Independent folder" in host.layer_folders
