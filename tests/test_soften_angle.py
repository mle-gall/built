# SPDX-License-Identifier: GPL-3.0-or-later
"""Edit ▸ Soften / Smooth Edges…: angle smoothing as one undo step, with
the soft flag an import sets (formats.fuse.soften_smooth_edges), shaded
smooth in the GL viewport and in the Metal view."""
import numpy as np
from PySide6.QtGui import QMatrix4x4, QVector3D as V

from core.group import Group, copy_group
from core.history import History
from core.mesh import Mesh
from core.scene import Scene
from core.soften import SoftenByAngleCommand, selected_edges
from tests.test_viewport_smooth_shading import _cylinder, _stub

SEAMS = 12          # the cylinder's side seams: 30° apart


def _soft(mesh):
    return sum(bool(e.soft) for e in mesh.edges)


def _scene_with(group):
    scene = Scene()
    scene.groups = [group]
    scene.selection = {group}
    return scene


def test_angle_softens_cylinder_sides_keeps_caps_and_undoes():
    group = Group(_cylinder(Mesh(), soft=False))
    scene = _scene_with(group)
    before = list(group.mesh.edges)
    hist = History(scene)
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    assert hist.last_error is None
    assert _soft(group.mesh) == SEAMS            # the 90° cap rims stay hard
    assert list(group.mesh.edges) == before      # no topology change
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 10))
    assert _soft(group.mesh) == 0
    hist.undo()
    assert _soft(group.mesh) == SEAMS
    hist.undo()
    assert _soft(group.mesh) == 0
    hist.redo()
    assert _soft(group.mesh) == SEAMS


def test_the_angle_reads_like_the_import_whatever_the_winding():
    """A side face drawn the other way round still smooths, as the import
    pass (abs of the normals' dot) smooths it."""
    from formats.fuse import soften_smooth_edges
    mesh = _cylinder(Mesh(), soft=False)
    mesh.faces[3].loop.reverse()
    mesh.mark_modified()
    imported = _cylinder(Mesh(), soft=False)
    imported.faces[3].loop.reverse()
    imported.mark_modified()
    soften_smooth_edges(imported, cos_threshold=0.85)       # ~31.8°
    group = Group(mesh)
    scene = _scene_with(group)
    History(scene).execute(SoftenByAngleCommand(selected_edges(scene), 31.8))
    assert _soft(mesh) == _soft(imported) == SEAMS


def test_a_material_boundary_keeps_its_line_unless_asked():
    mesh = _cylinder(Mesh(), soft=False)
    mesh.faces[0].attrs["color"] = (1.0, 0.0, 0.0)
    group = Group(mesh)
    scene = _scene_with(group)
    hist = History(scene)
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    assert _soft(mesh) == SEAMS - 2
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40,
                                      materials=True))
    assert _soft(mesh) == SEAMS


def test_face_selection_does_not_soften_its_outer_boundary():
    scene = Scene()
    _cylinder(scene.mesh, soft=False)
    scene.selection = set(scene.mesh.faces[:2])
    targets = selected_edges(scene)
    assert len(targets[scene.mesh]) == 1
    History(scene).execute(SoftenByAngleCommand(targets, 180))
    assert _soft(scene.mesh) == 1


def test_nested_definitions_are_processed_once_and_hidden_flags_survive():
    child = Group(_cylinder(Mesh(), soft=False))
    child.xform = QMatrix4x4()
    parent = Group()
    parent.adopt([child, copy_group(child)])
    scene = _scene_with(parent)
    edge = child.mesh.edges[0]
    edge.hidden = True
    targets = selected_edges(scene)
    assert len(targets[child.mesh]) == len(child.mesh.edges)
    History(scene).execute(SoftenByAngleCommand(targets, 40))
    assert edge.hidden
    assert parent.children[0].mesh is parent.children[1].mesh
    assert _soft(child.mesh) == SEAMS


def test_locked_and_linked_groups_are_left_alone(tmp_path):
    from core import links
    from formats import igz
    locked = Group(_cylinder(Mesh(), soft=False))
    locked.locked = True
    src = Scene()
    src.groups.append(Group(_cylinder(Mesh(), soft=False), name="drum"))
    path = tmp_path / "Site.igz"
    igz.save_scene(src, path)
    scene = Scene()
    link = links.make_link(scene, path, host_path=tmp_path / "host.igz")
    scene.groups = [locked, link]
    scene.selection = {locked, link, link.children[0]}
    assert selected_edges(scene) == {}


def test_smoothing_invalidates_the_cached_chunk_and_undo_restores_it():
    group = Group(_cylinder(Mesh(), soft=False))
    scene = _scene_with(group)
    vp = _stub(scene)
    before = vp._group_chunk(group)["vcol"]
    hist = History(scene)
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    after = vp._group_chunk(group)["vcol"]
    assert before != after
    hist.undo()
    assert vp._group_chunk(group)["vcol"] == before


def test_coplanar_and_unsoften_controls():
    scene = Scene()
    a = scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    b = scene.mesh.add_face([V(0, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    scene.selection = {a, b}
    hist = History(scene)
    targets = selected_edges(scene)
    hist.execute(SoftenByAngleCommand(targets, 1, coplanar=False))
    assert _soft(scene.mesh) == 0
    hist.execute(SoftenByAngleCommand(targets, 1))
    assert _soft(scene.mesh) == 1
    hist.execute(SoftenByAngleCommand(targets, harden=True))
    assert _soft(scene.mesh) == 0
    hist.undo()
    assert _soft(scene.mesh) == 1


def test_the_command_replays_by_key_across_other_undos():
    """Undo of an earlier mesh edit restores the mesh's elements by key:
    the smoothing still replays on them."""
    from core.edits import build_add_edge
    scene = Scene()
    _cylinder(scene.mesh, soft=False)
    hist = History(scene)
    scene.selection = set(scene.mesh.faces)
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    assert _soft(scene.mesh) == SEAMS
    hist.execute(build_add_edge(scene, V(5, 5, 0), V(6, 5, 0)))
    hist.undo()
    hist.undo()
    assert _soft(scene.mesh) == 0
    hist.redo()
    assert _soft(scene.mesh) == SEAMS


def test_metal_rebakes_the_smoothed_definition_without_a_repack(monkeypatch):
    """Both copies of a component shade smooth from one rebake of their
    definition; undo brings the facets back. No full pack, no rescan of
    the placements."""
    from tests.test_wgpu_updates import Renderer, assert_matches_fresh_pack
    from views.wgpu_scene import pack_scene
    from views.wgpu_updates import InstanceUpdates
    mesh = _cylinder(Mesh(), soft=False)
    a, b = Group(mesh, name="a"), Group(mesh, name="b")
    a.xform, b.xform = QMatrix4x4(), QMatrix4x4()
    b.xform.translate(5, 0, 0)
    scene = Scene(groups=[a, b])
    scene.selection = {a}
    updates = InstanceUpdates(scene, pack_scene(scene))
    hist = History(scene)
    updates.observe(hist)
    flat = updates.packed.vertices.copy()
    bakes = []
    monkeypatch.setattr(
        "views.wgpu_updates.pack_scene",
        lambda s, **kw: (bakes.append(len(s.groups)), pack_scene(s, **kw))[1])

    def settle():
        bakes.clear()
        delta = updates.diff(scene)
        assert delta is not None
        updates.apply(*delta, Renderer())
        assert updates.last_local
        assert bakes == [1]                         # one definition, once
        assert_matches_fresh_pack(updates, scene)

    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    settle()
    code = int(updates.packed.instances["info"][updates.rows[a.uid], 0])
    smooth = updates.packed.vertices.copy()
    assert code == int(updates.packed.instances["info"][updates.rows[b.uid], 0])
    assert not np.array_equal(flat, smooth)       # the normals moved
    hist.undo()
    settle()
    hist.redo()
    settle()


def test_dialog_cancel_and_accept_use_one_undo_step(monkeypatch):
    from PySide6.QtWidgets import QDialog
    from views.main_window import MainWindow
    from views.soften_dialog import SoftenDialog
    win = MainWindow()
    try:
        scene = win.viewport.scene
        group = Group(_cylinder(Mesh(), soft=False))
        scene.groups = [group]
        scene.selection = {group}
        monkeypatch.setattr(SoftenDialog, "exec", lambda self: QDialog.Rejected)
        win._on_soften_edges()
        assert _soft(group.mesh) == 0
        assert not win.viewport.history.undo_stack

        def accept(dialog):
            dialog.angle.setValue(40)
            return QDialog.Accepted
        monkeypatch.setattr(SoftenDialog, "exec", accept)
        win._on_soften_edges()
        assert _soft(group.mesh) == SEAMS
        assert len(win.viewport.history.undo_stack) == 1
        win.viewport.history.undo()
        assert _soft(group.mesh) == 0
    finally:
        win._saved_version = win.viewport.scene.version
        win.close()


def test_smoothing_inside_a_component_reaches_its_copies(monkeypatch):
    """Inside an open component the command runs on its world copy; the
    share-back carries the soft flags to every copy, one undo step."""
    from PySide6.QtWidgets import QDialog
    from views.main_window import MainWindow
    from views.soften_dialog import SoftenDialog
    mesh = _cylinder(Mesh(), soft=False)
    a, b = Group(mesh, name="a"), Group(mesh, name="b")
    a.xform, b.xform = QMatrix4x4(), QMatrix4x4()
    b.xform.translate(5, 0, 0)
    win = MainWindow()
    try:
        vp = win.viewport
        vp.scene.groups.extend([a, b])
        vp.scene.version += 1
        steps = len(vp.history.undo_stack)
        vp.begin_group_edit(a)
        vp.scene.selection = set(vp.scene.mesh.faces)

        def accept(dialog):
            dialog.angle.setValue(40)
            return QDialog.Accepted
        monkeypatch.setattr(SoftenDialog, "exec", accept)
        win._on_soften_edges()
        vp.end_group_edit()
        assert a.mesh is b.mesh is mesh
        assert _soft(mesh) == SEAMS
        assert len(vp.history.undo_stack) == steps + 1
        vp.history.undo()
        assert _soft(mesh) == 0
    finally:
        win._saved_version = win.viewport.scene.version
        win.close()
