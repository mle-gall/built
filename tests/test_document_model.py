"""The Rust document itself (rust/ingetrazo_core/src/doc, core/doc_model.py):
live lists that behave as Python lists, handle lifetimes, the journal
(rollback, savepoints, coalescing), transactions' undo/redo and change
records, matrix views, and History's use of them."""
import gc
import random

import pytest
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from core import doc_model
from core.group import Group
from core.history import (CommandRejected, Command, CompoundCommand, History,
                          HideCommand, LockCommand, MoveGroupCommand,
                          RenameGroupCommand)
from core.layers import Layer
from core.materials import Material
from core.mesh import Mesh
from core.scene import Scene
from formats import built

pytestmark = pytest.mark.skipif(not doc_model.ENABLED,
                                reason="the Rust document is not built")
N = doc_model.NATIVE


def test_shared_child_append_rolls_back_if_occurrence_copy_fails(monkeypatch):
    from core import group as groups

    scene = Scene()
    parent, child = Group(), Group()
    parent.adopt([child])
    peer = groups.copy_group(parent)
    scene.groups.extend([parent, peer])
    counterpart, = peer.children
    addition = Group()

    def fail_copy(*args, **kwargs):
        raise RuntimeError("occurrence copy failed")

    with monkeypatch.context() as patch:
        patch.setattr(groups, "copy_group", fail_copy)
        with pytest.raises(RuntimeError, match="occurrence copy failed"):
            parent.children.append(addition)
    assert list(parent.children) == [child]
    assert list(peer.children) == [counterpart]
    assert addition._parent is None
    assert N.doc_depth() == 0

    parent.children.append(addition)
    assert peer.children[0] is counterpart
    assert peer.children[1] is not addition
    assert peer.children[1].mesh is addition.mesh


def test_copy_of_open_document_edits_its_copied_definition():
    from copy import deepcopy
    from ingetrazo_core import Matrix4, Vec3

    scene = Scene()
    g = Group()
    g.xform = Matrix4()
    g.xform.translate(12, 3, 0)
    scene.groups.append(g)
    scene.begin_group_edit(g)
    other = deepcopy(scene)
    other.mesh.add_edge(Vec3(12, 3, 0), Vec3(14, 3, 0))
    assert other.groups[0].mesh.vertex_at(Vec3(2, 0, 0)) is not None
    assert not g.mesh.edges


def placements():
    gc.collect()
    return N.doc_stats()["placements"]


# ------------------------------------------------------------------ lists

@pytest.fixture
def no_parent_check():
    """Lists exercised as plain lists (the same placement twice): the tree
    invariant is off meanwhile."""
    old = doc_model.parent_mode("off")
    yield
    doc_model.parent_mode(old)


def test_lists_behave_as_python_lists(no_parent_check):
    rng = random.Random(7)
    pool = [Group(Mesh(), name=f"g{i}") for i in range(8)]
    owner = Group(Mesh(), name="owner")
    live, ref = owner.children, []
    for step in range(600):
        op = rng.randrange(13)
        g = rng.choice(pool)
        if op == 0:
            live.append(g); ref.append(g)
        elif op == 1:
            i = rng.randrange(-len(ref) - 2, len(ref) + 3)
            live.insert(i, g); ref.insert(i, g)
        elif op == 2 and g in ref:
            live.remove(g); ref.remove(g)
        elif op == 3 and ref:
            i = rng.randrange(-len(ref), len(ref))
            assert live.pop(i) is ref.pop(i)
        elif op == 4:
            items = rng.sample(pool, rng.randrange(4))
            live.extend(items); ref.extend(items)
        elif op == 5 and ref:
            a, b = sorted(rng.randrange(len(ref) + 1) for _ in range(2))
            items = rng.sample(pool, rng.randrange(3))
            live[a:b] = items; ref[a:b] = items
        elif op == 6 and ref:
            i = rng.randrange(len(ref))
            live[i] = g; ref[i] = g
        elif op == 7 and ref:
            del live[::2]; del ref[::2]
        elif op == 8:
            live.reverse(); ref.reverse()
        elif op == 9:
            live.sort(key=lambda x: x.name); ref.sort(key=lambda x: x.name)
        elif op == 10:
            live += [g]; ref += [g]
        elif op == 11 and len(ref) > 12:
            live.clear(); ref.clear()
        elif op == 12 and ref:
            del live[-1]; del ref[-1]
        assert list(live) == ref, step
        assert len(live) == len(ref) and bool(live) == bool(ref)
        assert live == ref and (live != ref) is False
        assert live[1:3] == ref[1:3] and live[::-1] == ref[::-1]
        assert (g in live) == (g in ref) and live.count(g) == ref.count(g)
        if g in ref:
            assert live.index(g) == ref.index(g)
    assert owner.children is live          # one view per list
    assert repr(live) == repr(ref)
    assert live + [pool[0]] == ref + [pool[0]] and [pool[0]] + live == [pool[0]] + ref


def test_a_list_refuses_what_it_cannot_hold():
    g = Group()
    with pytest.raises(TypeError):
        g.children.append(object())
    with pytest.raises(TypeError):
        g.children.append(Layer("tag"))
    with pytest.raises(ValueError):
        g.children.remove(Group())
    with pytest.raises(IndexError):
        g.children.pop()


def test_a_replaced_list_keeps_what_it_held():
    a, b, c = Group(name="a"), Group(name="b"), Group(name="c")
    g = Group()
    g.children = [a, b]
    old = g.children
    g.children = [c]                    # `old` is the old list object now
    assert list(old) == [a, b] and list(g.children) == [c]
    old.append(c)                       # an ordinary list from now on
    assert list(g.children) == [c]
    old.remove(c)
    g.children = old                    # assigned back: it is the list again
    assert g.children is old and list(old) == [a, b]
    old.append(c)
    assert list(g.children) == [a, b, c]
    same = g.children
    g.children += [Group()]             # in place: the same list
    assert g.children is same and len(same) == 4


def test_scene_lists_and_constructor():
    a, b = Group(name="a"), Group(name="b")
    scene = Scene(groups=[a, b])
    assert list(scene.groups) == [a, b]
    assert scene.layers[0].name == "Layer 0"
    scene.groups = scene.groups[::-1]
    assert list(scene.groups) == [b, a]
    scene.clear()
    assert len(scene.groups) == 0 and len(scene.layers) == 1


# -------------------------------------------------------------- lifetimes

def test_a_list_keeps_its_members_alive():
    base = placements()
    owner = Group()
    owner.children.append(Group(name="only held by the list"))
    assert placements() == base + 2
    assert owner.children[0].name == "only held by the list"
    owner.children.clear()
    assert placements() == base + 1
    del owner
    assert placements() == base


def test_cycles_through_the_document_are_collected():
    base = placements()
    a, b = Group(), Group()
    a.children.append(b)
    b.owner = a                      # a Python slot pointing back
    del a, b
    assert placements() == base


def test_a_transaction_keeps_what_it_removed():
    base = placements()
    scene = Scene()
    scene.groups.append(Group(name="doomed"))
    N.doc_begin()
    scene.groups.pop()
    txn = N.doc_commit()
    assert placements() == base + 1         # only the transaction holds it
    txn.undo()
    assert scene.groups[0].name == "doomed"
    scene.groups.clear()
    del txn, scene
    assert placements() == base


# ---------------------------------------------------------------- journal

def test_rollback_puts_back_fields_and_lists():
    scene = Scene()
    g = Group(name="before")
    scene.groups.append(g)
    N.doc_begin()
    g.name = "after"
    g.hidden = True
    g.material = {"color": (1, 0, 0)}
    scene.groups.append(Group())
    scene.layers.append(Layer("new"))
    N.doc_rollback()
    assert (g.name, g.hidden, g.material) == ("before", False, None)
    assert list(scene.groups) == [g] and len(scene.layers) == 1
    assert N.doc_depth() == 0


def test_savepoints_roll_back_only_the_inner_level():
    g = Group(name="a")
    N.doc_begin()
    g.name = "b"
    N.doc_begin()
    g.name = "c"
    g.locked = True
    N.doc_rollback()
    assert (g.name, g.locked) == ("b", False)
    txn = N.doc_commit()
    txn.undo()
    assert g.name == "a"
    txn.redo()
    assert g.name == "b"


def test_writes_coalesce_and_no_op_writes_vanish():
    g = Group(name="a")
    g.hidden = False
    N.doc_begin()
    for i in range(50):
        g.name = f"n{i}"
    g.hidden = True
    g.hidden = False
    txn = N.doc_commit()
    assert [op[2] for op in txn.ops()] == ["name"]


def test_fields_of_a_node_made_in_the_step_are_not_journalled():
    scene = Scene()
    N.doc_begin()
    ly = Layer("Fresh")
    scene.layers.append(ly)
    txn = N.doc_commit()
    assert [op[0] for op in txn.ops()] == ["insert"]
    assert txn.changes(root=scene._root._doc_key)["tags"]["created"] == ["Fresh"]
    txn.undo()
    assert txn.changes(inverse=True, root=scene._root._doc_key)["tags"]["removed"] == ["Fresh"]
    assert ly.key == "Fresh"


def test_change_records():
    scene = Scene()
    kept, gone = Group(name="kept", uid="k"), Group(name="gone", uid="g")
    scene.groups.extend([kept, gone])
    m = Material("Brick", color=(0.5, 0.2, 0.1))
    N.doc_begin()
    kept.name = "renamed"
    scene.groups.remove(gone)
    scene.groups.append(Group(name="new", uid="n"))
    scene.layers[0].visible = False
    m.finish = "matte"
    txn = N.doc_commit()
    root = scene._root._doc_key
    ch = txn.changes(root=root)
    assert ch["placements"] == {"created": ["n"], "removed": ["g"], "modified": ["k"]}
    assert ch["tags"]["modified"] == ["Layer 0"]
    assert ch["materials"]["modified"] == ["Brick"]
    inv = txn.changes(inverse=True, root=root)["placements"]
    assert (inv["created"], inv["removed"]) == (["g"], ["n"])


# --------------------------------------------------------------- matrices

def test_matrix_views_write_back():
    g = Group()
    m = QMatrix4x4()
    g.xform = m
    assert g.xform is m                      # a matrix Python made becomes the view
    m.translate(QVector3D(1, 2, 3))
    assert g.xform.data()[12:15] == (1.0, 2.0, 3.0)
    g.xform.rotate(90, QVector3D(0, 0, 1))
    assert abs(g.xform.data()[1] - 1.0) < 1e-6
    old = g.xform
    g.xform = QMatrix4x4() * QMatrix4x4()    # Qt made this one: copied
    old.translate(QVector3D(5, 0, 0))        # detached: no effect
    assert g.xform.data()[12] == 0.0
    h = Group()
    h.xform = g.xform                        # one view names one placement
    h.xform.translate(QVector3D(0, 7, 0))
    assert g.xform.data()[13] == 0.0 and h.xform.data()[13] == 7.0


def test_a_built_placement_keeps_its_doubles():
    vals = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 650000.013, 6860000.027, 1.5, 1]
    g = Group()
    g.xform = built.placement(vals)
    gc.collect()
    assert built.placement_values(g.xform) == [float(v) for v in vals]
    N.doc_begin()
    g.xform = QMatrix4x4()
    N.doc_commit().undo()                    # the view is rebuilt from the document
    assert built.placement_values(g.xform) == [float(v) for v in vals]


def test_copy_and_pickle():
    import copy
    import pickle
    g = Group(name="a")
    g.adopt([Group(name="kid")])
    g.material = {"color": (0.1, 0.2, 0.3)}
    for c in (copy.deepcopy(g), pickle.loads(pickle.dumps(g))):
        assert c is not g and c.uid == g.uid and c.material == g.material
        assert c.children[0].name == "kid" and c.children[0] is not g.children[0]
        assert c.xform == g.xform and c.xform is not g.xform
    m = Material("Brick", color=(1, 0, 0))
    assert copy.deepcopy(m) == m and pickle.loads(pickle.dumps(m)) == m


# ---------------------------------------------------------------- History

class _RenameThenFail(Command):
    def __init__(self, group, reject=False):
        self.group, self.reject = group, reject

    def do(self, scene):
        self.group.name = "half done"
        scene.groups.append(Group())
        if self.reject:
            raise CommandRejected("no")
        raise RuntimeError("boom")

    def undo(self, scene):
        pass


@pytest.mark.parametrize("reject", [False, True])
def test_a_failed_command_leaves_the_document_as_it_was(tmp_path, reject):
    scene = Scene()
    g = Group(name="intact")
    scene.groups.append(g)
    h = History(scene)
    h.error_log = str(tmp_path / "errors.log")
    h.execute(_RenameThenFail(g, reject))
    assert g.name == "intact" and list(scene.groups) == [g]
    assert not h.undo_stack


def test_history_undoes_document_commands_by_their_transaction():
    scene = Scene()
    g = Group(Mesh(), name="part")
    t = QMatrix4x4()
    t.translate(QVector3D(0.1, 0.25, 0.3))
    g.xform = t
    scene.groups.append(g)
    h = History(scene)
    before = g.xform.data()
    h.execute(MoveGroupCommand(g, QVector3D(0.7, 0.2, 0.0)))
    h.execute(RenameGroupCommand(g, "moved"))
    h.execute(HideCommand([g], True))
    assert all(c._doc_txn is not None for c in h.undo_stack)
    for _ in range(3):
        h.undo()
    assert g.xform.data() == before and g.name == "part" and not g.hidden
    for _ in range(3):
        h.redo()
    assert g.name == "moved" and g.hidden
    assert h.feed.last.placements["modified"] == [g.uid]


def test_a_compound_of_document_commands_is_one_transaction():
    scene = Scene()
    gs = [Group(Mesh(), name=f"g{i}") for i in range(50)]
    scene.groups.extend(gs)
    h = History(scene)
    h.execute(CompoundCommand([RenameGroupCommand(g, "R") for g in gs]
                              + [LockCommand(gs, True)]))
    assert h.undo_stack[-1]._doc_txn is not None
    h.undo()
    assert [g.name for g in gs] == [f"g{i}" for i in range(50)]
    assert not any(g.locked for g in gs)
    h.redo()
    assert all(g.name == "R" and g.locked for g in gs)


def test_geometry_definition_swap_and_rollback():
    a = Group()
    b = Group(a.mesh)
    original = a.definition
    assert b.definition is original
    N.doc_begin()
    a.mesh = Group().mesh
    assert a.definition.key != original.key
    N.doc_rollback()
    assert a.definition is original and a.mesh is b.mesh


def test_definition_does_not_pin_a_geometry_cycle():
    base = placements()
    a = Group()
    b = Group(a.mesh)
    a.mesh.placements = (a, b)
    del a, b
    assert placements() == base


def test_scoped_delivery_copies_kept_placements_and_material_mapping():
    from core.deliveries import scoped
    scene = Scene()
    kept = Group(name="Kept")
    kept.adopt([Group(name="Child")])
    scene.kept_components.append(kept)
    scene.materials["Oak"] = Material("Oak")
    out = scoped(scene, {"scope": "whole"})
    assert out.materials["Oak"] is scene.materials["Oak"]
    projected, = out.kept_components
    assert projected is not kept and projected.definition is kept.definition
    assert projected.children[0] is not kept.children[0]
    assert kept._parent is scene._root


def test_geometry_savepoints_across_occurrences_and_placement_fields():
    from core.history import SnapshotMutation
    from core.mesh import Mesh
    from core.store_mesh import instance_view

    scene = Scene()
    group = Group(Mesh(), name="before")
    scene.groups.append(group)
    mesh = group.mesh
    transform = QMatrix4x4()
    transform.translate(10, 0, 0)
    a = instance_view(mesh, transform.data())
    b = instance_view(mesh, QMatrix4x4().data())
    if a is None:
        pytest.skip("requires the Rust geometry store")
    history = History(scene)
    original = mesh.capture_state()
    N.doc_begin()
    try:
        group.name = "outer"
        history.execute(SnapshotMutation(lambda _: a.add_edge(QVector3D(10, 0, 0),
                                                              QVector3D(11, 0, 0)), mesh=a))
        assert history.last_error is None
        edge = mesh.edges[0].key
        N.doc_begin()
        group.name = "discard"
        history.execute(SnapshotMutation(lambda _: b.remove_edge(b.edges[0]), mesh=b))
        assert history.last_error is None and not mesh.edges
        N.doc_rollback()
        assert group.name == "outer" and b.edges[0].key == edge
        history.execute(SnapshotMutation(lambda _: b.add_edge(QVector3D(1, 0, 0),
                                                              QVector3D(2, 0, 0)), mesh=b))
        assert history.last_error is None
        txn = N.doc_commit()
    finally:
        while N.doc_depth():
            N.doc_rollback()
    after = mesh.capture_state()
    assert len(txn) == 2  # one name and one canonical mesh
    txn.undo()
    assert mesh.capture_state() == original and group.name == "before"
    txn.redo()
    assert mesh.capture_state() == after and group.name == "outer"
    assert a.edges[0].key == edge


def test_singular_store_session_keeps_shared_keys_on_replay():
    from core.history import AddEdgeCommand
    from core.reference_edit import record_leave

    scene = Scene()
    a = Group()
    a.mesh.add_edge(QVector3D(0, 0, 0), QVector3D(1, 0, 0))
    definition = a.mesh
    b = Group(definition)
    matrix = QMatrix4x4()
    matrix.scale(1, 1, 0)
    a.xform, b.xform = matrix, QMatrix4x4()
    scene.groups.extend((a, b))
    h = History(scene)
    before = definition.capture_state()
    scene.begin_group_edit(a)
    level = scene._edit_stack[-1]
    level["undo_mark"] = 0
    assert level["entry_transaction"] is not None
    h.execute(AddEdgeCommand(QVector3D(0, 1, 0), QVector3D(1, 1, 0)))
    share = level.pop("share")
    scene.end_group_edit()
    assert record_leave(h, level, share) and not h.last_error
    after = definition.capture_state()
    keys = [e.key for e in definition.edges]
    assert len(keys) == 2 and a.mesh is b.mesh is definition
    assert h.feed.last.command == "ReshareInstanceCommand"
    for _ in range(2):
        assert h.undo() and a.mesh is b.mesh and definition.capture_state() == before
        assert h.redo() and a.mesh is b.mesh and definition.capture_state() == after
        assert [e.key for e in definition.edges] == keys
