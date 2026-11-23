# SPDX-License-Identifier: GPL-3.0-or-later
"""A copy of a linked file is the linked file again.

However it was made — Copy and Paste, a copy dragged with Move or Rotate,
an array, Make Unique of what holds it, Explode of what holds it, or the
document saved and opened again — the copy is a live link of the same
source: its parts are the source's geometry, read-only (going into any of
them offers the source instead), they carry the uids a fresh link gives
them, and a change saved in the source reaches the copy too.

A copy used to keep the link record on its root only: its parts were not
owned by it, so the read-only check missed them, and an edit inside one
reached the other copies in memory while the source never saw it."""
from __future__ import annotations

import math
import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest  # noqa: E402
from PySide6.QtCore import QPointF, Qt  # noqa: E402
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V
from PySide6.QtWidgets import QApplication, QMessageBox  # noqa: E402

from core import links  # noqa: E402
from core.group import Group, copy_group  # noqa: E402
from core.history import History  # noqa: E402
from core.mesh import Mesh  # noqa: E402
from core.purge import iter_groups  # noqa: E402
from core.scene import Scene  # noqa: E402
from formats import igz  # noqa: E402
from tools.base import ToolContext  # noqa: E402

_app = QApplication.instance() or QApplication([])


def _square(x=0.0, z=0.0):
    m = Mesh()
    m.add_face([V(x, 0, z), V(x + 1, 0, z), V(x + 1, 1, z), V(x, 1, z)])
    return m


def _source(path):
    """An assembly holding a part, and a classic group beside it (no
    matrix: what a copy of a component used to rewrite)."""
    s = Scene()
    part = Group(_square(z=1.0), name="part")
    part.xform = QMatrix4x4()
    asm = Group(_square(), name="asm")
    asm.adopt([part])
    s.groups.append(asm)
    s.groups.append(Group(_square(x=3.0), name="classic"))
    igz.save_scene(s, path)
    return path


def _host(tmp_path):
    src = _source(tmp_path / "Site.igz")
    host = Scene()
    link = links.make_link(host, src, host_path=tmp_path / "host.igz")
    host.groups.append(link)
    host.version += 1
    return host, link, src


def _parts(g):
    return list(iter_groups(g.children or ()))


def _state(g):
    """What a link's parts are, to compare across a copy."""
    return [(p.name, None if p.xform is None else tuple(p.xform.data()),
             p.component) for p in _parts(g)]


def _faces(g):
    from core.group import iter_placements
    return sum(len(p.mesh.faces) for p, _m in iter_placements(g))


def assert_live_link(copy, original):
    """``copy`` is ``original``'s file placed again: same source, parts
    read-only and sharing its geometry, uids as a fresh link gives them."""
    assert links.is_linked(copy)
    assert copy is not original
    assert links.link_of(copy)["path"] == links.link_of(original)["path"]
    parts, theirs = _parts(copy), _parts(original)
    assert parts and len(parts) == len(theirs)
    for p, q in zip(parts, theirs):
        assert p is not q
        assert links.linked_ancestor(p) is copy, p.name
        assert p.mesh is q.mesh, p.name
        assert p.name == q.name
        assert p.uid == links.stable_uid(copy.uid, q.source_uid)
    assert _state(copy) == _state(original)


def _edit_source(src):
    """The source opened, two faces drawn in it, and saved: its groups keep
    their uids."""
    s = Scene()
    igz.load_into(s, src)
    for i in range(2):
        s.mesh.add_face([V(i, 5, 0), V(i + 1, 5, 0), V(i + 1, 6, 0), V(i, 6, 0)])
    igz.save_scene(s, src)


def assert_reloads_with_the_source(scene, src, *copies):
    """A change saved in the source reaches every copy, which keeps its
    parts (found again by uid) and stays read-only."""
    before = {c: {p.uid: p for p in _parts(c)} for c in copies}
    _edit_source(src)
    links.reload(scene, src)
    for c in copies:
        assert _faces(c) == 5
        kept = {p.uid: p for p in _parts(c)}
        assert kept == before[c]           # the same objects, by uid
        assert all(links.linked_ancestor(p) is c for p in _parts(c))


# ---- copy_group: what every copy is made with -----------------------------

def test_a_copied_link_is_a_live_link_and_leaves_the_original_alone(tmp_path):
    host, link, src = _host(tmp_path)
    state = _state(link)
    uids = [p.uid for p in _parts(link)]
    copy = copy_group(link, V(5, 0, 0))
    host.groups.append(copy)
    assert_live_link(copy, link)
    assert _state(link) == state            # the classic part kept no matrix
    assert [p.uid for p in _parts(link)] == uids
    assert_reloads_with_the_source(host, src, link, copy)


def test_a_copy_of_a_copy_and_of_what_holds_a_link(tmp_path):
    host, link, src = _host(tmp_path)
    holder = Group(Mesh(), name="holder")
    holder.adopt([link])
    host.groups = [holder]
    twice = copy_group(copy_group(link))
    assert_live_link(twice, link)
    held = copy_group(holder, V(0, 9, 0))          # a component around it
    host.groups.append(held)
    (inner,) = held.children
    assert_live_link(inner, link)
    assert_reloads_with_the_source(host, src, link, inner)


# ---- The ways the user copies ---------------------------------------------

class _Vp:
    def __init__(self, scene):
        self.scene = scene
        self.history = History(scene)
        self.flashed = []

    def update(self):
        pass

    def set_hover(self, *_a):
        pass

    def flash_status(self, msg, *a, **k):
        self.flashed.append(msg)

    def pick_edge(self, x, y):
        return None

    def pick_face(self, x, y):
        return None

    def pick_group(self, x, y):
        return None


def _ctx(vp, x, y, z=0.0):
    return ToolContext(viewport=vp, world=V(x, y, z), screen=QPointF(0, 0),
                       modifiers=Qt.NoModifier, snap=None)


def _copies(scene, link):
    return [g for g in links.linked_groups(scene) if g is not link]


def test_ctrl_drag_copy_and_a_move_array(tmp_path):
    from tools.move import MoveTool
    host, link, src = _host(tmp_path)
    host.select([link])
    vp = _Vp(host)
    tool = MoveTool()
    tool.on_click(_ctx(vp, 0, 0))
    tool.on_key(vp, Qt.Key_Control, Qt.NoModifier)        # Ctrl: copy
    tool.on_click(_ctx(vp, 4, 0))
    (copy,) = _copies(host, link)
    assert_live_link(copy, link)
    assert tool.on_array_value(vp, 3, "x")                # «3x»
    copies = _copies(host, link)
    assert len(copies) == 3
    for c in copies:
        assert_live_link(c, link)
    assert len({p.uid for c in [link, *copies] for p in _parts(c)}) == 4 * 3
    assert_reloads_with_the_source(host, src, link, *copies)


def test_rotate_copy_and_a_polar_array(tmp_path):
    from tools.rotate import RotateTool
    host, link, src = _host(tmp_path)
    host.select([link])
    vp = _Vp(host)
    tool = RotateTool()
    tool.on_click(_ctx(vp, 0, 0))
    tool.on_click(_ctx(vp, 2, 0))
    tool.on_key(vp, Qt.Key_Control, Qt.NoModifier)
    tool.on_click(_ctx(vp, 2 * math.cos(math.radians(30)),
                       2 * math.sin(math.radians(30))))
    (copy,) = _copies(host, link)
    assert_live_link(copy, link)
    assert tool.on_array_value(vp, 4, "x")
    copies = _copies(host, link)
    assert len(copies) == 4
    for c in copies:
        assert_live_link(c, link)
    assert_reloads_with_the_source(host, src, link, *copies)


@pytest.fixture
def win(monkeypatch):
    from views.main_window import MainWindow
    w = MainWindow()
    asked = []
    monkeypatch.setattr(QMessageBox, "exec",
                        lambda box: asked.append(box.text()) or 0)
    w._asked = asked
    yield w
    for d in w.documents():
        d.saved_content = d.viewport.scene.content_version
    w._saved_version = w.viewport.scene.version
    w.close()


def _place(win, tmp_path):
    src = _source(tmp_path / "Site.igz")
    scene = win.viewport.scene
    link = links.make_link(scene, src, host_path=tmp_path / "host.igz")
    scene.groups.append(link)
    scene.version += 1
    return scene, link, src


def assert_read_only(win, copy):
    """Going into the copy, or into any part of it, offers the source and
    enters nothing."""
    vp, scene = win.viewport, win.viewport.scene
    steps = len(vp.history.undo_stack)
    for g in [copy, *_parts(copy)]:
        asked = len(win._asked)
        vp.begin_group_edit(g)
        assert scene.edit_group is None, g.name
        assert len(win._asked) == asked + 1 and "Site.igz" in win._asked[-1]
    assert len(vp.history.undo_stack) == steps


def test_copy_and_paste(win, tmp_path):
    from tools.paste import PasteTool
    scene, link, src = _place(win, tmp_path)
    vp = win.viewport
    scene.select([link])
    assert vp.copy_selection()
    win._on_paste()
    assert isinstance(vp.active_tool, PasteTool)
    vp.active_tool.on_click(ToolContext(
        viewport=vp, world=V(6, 0, 0), screen=QPointF(0, 0),
        modifiers=Qt.NoModifier, snap=None))
    (copy,) = _copies(scene, link)
    assert_live_link(copy, link)
    assert_read_only(win, copy)
    assert_reloads_with_the_source(scene, src, link, copy)
    assert_read_only(win, copy)


def test_make_unique_of_what_holds_a_link_and_of_the_link(win, tmp_path):
    from core.history import MakeUniqueCommand
    scene, link, src = _place(win, tmp_path)
    vp = win.viewport
    holder = Group(Mesh(), name="holder")
    holder.adopt([link])
    other = copy_group(holder, V(0, 9, 0))
    scene.groups = [holder, other]
    scene.version += 1
    vp.history.execute(MakeUniqueCommand(other))
    (inner,) = other.children
    assert other.mesh is not holder.mesh                 # made its own
    assert_live_link(inner, link)                        # the link did not
    assert_read_only(win, inner)
    vp.history.undo()
    (inner,) = other.children
    assert_live_link(inner, link)
    vp.history.redo()
    (inner,) = other.children
    assert_live_link(inner, link)
    assert_reloads_with_the_source(scene, src, link, inner)
    # Make Unique on the link itself breaks that link and keeps the
    # geometry (tests/test_links_unlink.py); the other copy stays linked.
    meshes = [p.mesh for p in _parts(inner)]
    steps = len(vp.history.undo_stack)
    scene.select([inner])
    win._on_make_unique()
    assert len(vp.history.undo_stack) == steps + 1
    assert not links.is_linked(inner)
    assert not {id(p.mesh) for p in _parts(inner)} & {id(m) for m in meshes}
    assert all(links.linked_ancestor(p) is None for p in _parts(inner))
    assert links.is_linked(link)
    assert_read_only(win, link)


def test_explode_of_a_copied_holder_keeps_the_link(win, tmp_path):
    from core.history import ExplodeGroupCommand
    scene, link, src = _place(win, tmp_path)
    vp = win.viewport
    holder = Group(Mesh(), name="holder")
    holder.adopt([link])
    holder.component = False                 # a group of groups
    other = copy_group(holder, V(0, 9, 0))
    scene.groups = [holder, other]
    scene.version += 1
    (inner,) = other.children
    vp.history.execute(ExplodeGroupCommand(other))
    assert inner in scene.groups
    assert_live_link(inner, link)
    assert_read_only(win, inner)
    vp.history.undo()
    assert_live_link(other.children[0], link)
    vp.history.redo()
    assert_reloads_with_the_source(scene, src, link, inner)


@pytest.mark.parametrize("packed", [False, True])
def test_save_and_reopen(tmp_path, packed):
    host, link, src = _host(tmp_path)
    copy = copy_group(link, V(5, 0, 0))
    host.groups.append(copy)
    if packed:
        links.set_packed(host, src, True)
    path = tmp_path / "host.igz"
    igz.save_scene(host, path)
    back = Scene()
    igz.load_into(back, path)
    assert back.link_errors == {}
    a, b = links.linked_groups(back)
    assert {a.uid, b.uid} == {link.uid, copy.uid}
    for g, was in ((a, link if a.uid == link.uid else copy),
                   (b, link if b.uid == link.uid else copy)):
        assert _state(g) == _state(was)
        assert [p.uid for p in _parts(g)] == [p.uid for p in _parts(was)]
        assert all(links.linked_ancestor(p) is g for p in _parts(g))
    if not packed:
        again = copy_group(a)
        back.groups.append(again)
        assert_live_link(again, a)
        assert_reloads_with_the_source(back, src, a, b, again)
