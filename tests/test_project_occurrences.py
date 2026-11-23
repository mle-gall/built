# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Occurrence traversal, drop rules and section records without a browser widget."""
from __future__ import annotations

import random

from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from core import outliner
from core.group import Group, copy_group
from core.mesh import Mesh

_QUADS = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6),
          (0, 2, 6, 4), (1, 5, 7, 3)]


def _box(at=(0.0, 0.0, 0.0), size=0.1) -> Mesh:
    mesh = Mesh()
    x0, y0, z0 = at
    c = [QVector3D(x0 + dx * size, y0 + dy * size, z0 + dz * size)
         for dz in (0, 1) for dy in (0, 1) for dx in (0, 1)]
    for q in _QUADS:
        mesh.add_face([c[k] for k in q])
    return mesh


def _group(name, at=(0.0, 0.0, 0.0)) -> Group:
    return Group(_box(at), name=name)


def _container(name, kids) -> Group:
    """A group of groups — what Make Group over groups builds."""
    g = Group(name=name)
    g.adopt(kids)
    g.component = False
    return g


def _building():
    """Building ▸ Level 1 ▸ (Wall, Slab) and Level 2 ▸ (Wall), plus a
    loose Tree beside it."""
    wall1, slab = _group("Wall", (0, 0, 0)), _group("Slab", (1, 0, 0))
    wall1.ifc = {"class": "IfcWall"}
    level1 = _container("Level 1", [wall1, slab])
    level2 = _container("Level 2", [_group("Wall", (0, 0, 3))])
    building = _container("Building", [level1, level2])
    tree = _group("Tree", (5, 0, 0))
    return building, tree


def _random_tree(n: int, seed: int = 7) -> list:
    """``n`` groups hung at random under the ones made before them."""
    rnd = random.Random(seed)
    roots: list = []
    made: list = []
    for i in range(n):
        g = Group(Mesh(), name=f"Node {i}")
        g.component = False
        if made and rnd.random() < 0.8:
            parent = rnd.choice(made)
            parent.adopt(list(parent.children) + [g])
            parent.component = False
        else:
            roots.append(g)
        made.append(g)
    return roots


def test_walk_lists_every_group_at_every_depth():
    building, tree = _building()
    rows = [(g.name, p.name if p else None, d)
            for g, p, d in outliner.walk([building, tree])]
    assert rows == [("Building", None, 0), ("Level 1", "Building", 1),
                    ("Wall", "Level 1", 2), ("Slab", "Level 1", 2),
                    ("Level 2", "Building", 1), ("Wall", "Level 2", 2),
                    ("Tree", None, 0)]


def test_path_to_names_the_containers_to_open():
    building, tree = _building()
    level1 = building.children[0]
    wall = level1.children[0]
    groups = [building, tree]
    assert outliner.path_to(groups, wall) == [building, level1]
    assert outliner.path_to(groups, tree) == []
    assert outliner.path_to(groups, Group(name="elsewhere")) is None


def test_a_random_tree_keeps_every_path_consistent():
    roots = _random_tree(2000)
    nodes = list(outliner.walk(roots))
    assert len(nodes) == 2000
    up = outliner.parents(roots)
    for g, parent, depth in nodes:
        path = outliner.path_to(roots, g)
        assert len(path) == depth
        assert (path[-1] if path else None) is parent is up[id(g)]


def test_the_shape_ignores_names_and_visibility_not_structure():
    building, tree = _building()
    groups = [building, tree]
    shape = outliner.structure(groups)
    building.name = "Tower"
    tree.hidden = True
    assert outliner.structure(groups) == shape
    level2 = building.children[1]
    level2.adopt(list(level2.children) + [_group("Roof")])
    assert outliner.structure(groups) != shape


def test_kind_class_and_copies():
    proto = Group(_box(), name="Chair")
    proto.xform = QMatrix4x4()
    copies = [proto] + [copy_group(proto, QVector3D(i, 0, 0))
                        for i in (1, 2)]
    building, _tree = _building()
    wall = building.children[0].children[0]
    assert outliner.kind(proto) == "component"
    assert outliner.kind(building) == "group"
    assert outliner.ifc_class(wall) == "IfcWall"
    assert outliner.ifc_class(building) == ""
    assert outliner.instance_counts(copies + [building]) == {
        id(proto.mesh): 3}


def test_filter_keep_in_the_core():
    building, tree = _building()
    keep, hit = outliner.filter_keep([building, tree], "wall")
    walls = {id(w) for lvl in building.children for w in lvl.children
             if w.name == "Wall"}
    assert hit == walls
    assert keep == walls | {id(building)} | {id(lvl)
                                             for lvl in building.children}


def test_isolate_in_the_core_hides_the_siblings_on_the_way_down():
    building, tree = _building()
    level1, level2 = building.children
    wall, slab = level1.children
    hide = outliner.isolate_hides([building, tree], [wall])
    assert {g.name for g in hide} == {"Slab", "Level 2", "Tree"}
    assert outliner.isolate_hides([building, tree], [building]) == [tree]


def _world_points(groups, group) -> list:
    """Every vertex of ``group``'s own mesh, in world coordinates, rounded."""
    m = outliner.world_matrix(groups, group)
    out = []
    for v in group.mesh.vertices:
        p = m.map(v.position) if m is not None else v.position
        out.append((round(p.x(), 6), round(p.y(), 6), round(p.z(), 6)))
    return sorted(out)


def _moved_container():
    """A container moved and turned: its frame is not the world's."""
    m = QMatrix4x4()
    m.translate(QVector3D(10, 5, 0))
    m.rotate(30, 0, 0, 1)
    kid = Group(_box((0, 0, 0)), name="Kid")
    kid.xform = QMatrix4x4()
    kid.component = False
    box = _container("Shelf", [kid])
    box.xform = m
    return box


def test_a_classic_group_dropped_into_a_turned_group_stays_put():
    from core.history import ReparentGroupsCommand
    from core.scene import Scene
    scene = Scene()
    shelf = _moved_container()
    loose = _group("Loose", (3, 4, 0))
    scene.groups[:] = [shelf, loose]
    before = _world_points(scene.groups, loose)
    mesh0 = loose.mesh
    cmd = ReparentGroupsCommand([loose], shelf)
    cmd.do(scene)
    assert loose in shelf.children and loose not in scene.groups
    assert _world_points(scene.groups, loose) == before
    cmd.undo(scene)
    assert scene.groups == [shelf, loose] and shelf.children[-1] is not loose
    assert loose.mesh is mesh0 and loose.xform is None


def test_an_instance_taken_out_keeps_its_place_and_children():
    from core.history import ReparentGroupsCommand
    from core.scene import Scene
    scene = Scene()
    shelf = _moved_container()
    kid = shelf.children[0]
    scene.groups[:] = [shelf]
    before = _world_points(scene.groups, kid)
    cmd = ReparentGroupsCommand([kid], None)
    cmd.do(scene)
    assert scene.groups == [shelf, kid] and not shelf.children
    assert _world_points(scene.groups, kid) == before
    cmd.undo(scene)
    assert shelf.children == [kid] and scene.groups == [shelf]
    assert _world_points(scene.groups, kid) == before


def test_a_plain_target_becomes_a_group_of_groups_not_a_component():
    from core.history import ReparentGroupsCommand
    from core.scene import Scene
    scene = Scene()
    a, b = _group("A"), _group("B", (2, 0, 0))
    scene.groups[:] = [a, b]
    cmd = ReparentGroupsCommand([b], a)
    cmd.do(scene)
    assert a.xform is not None and not a.is_component()
    assert outliner.kind(a) == "group"
    cmd.undo(scene)
    assert a.xform is None


def test_moving_a_group_and_its_child_moves_the_group_once():
    building, tree = _building()
    level1 = building.children[0]
    assert outliner.topmost([building, tree],
                            [level1.children[0], level1, tree]) == [
        level1, tree]


def test_what_cannot_be_re_nested():
    building, tree = _building()
    level1 = building.children[0]
    groups = [building, tree]
    assert outliner.reparent_problem(groups, [building], level1) == "cycle"
    assert outliner.reparent_problem(groups, [level1], level1) == "cycle"
    chair = Group(_box(), name="Chair")
    chair.xform = QMatrix4x4()
    part = Group(_box(), name="Seat")
    part.xform = QMatrix4x4()
    part.component = False
    chair.adopt([part])
    chair.component = True
    groups = [building, tree, chair]
    assert outliner.reparent_problem(groups, [tree], chair) == "component"
    assert outliner.reparent_problem(groups, [part], None) == "component"
    tree.locked = True
    assert outliner.reparent_problem(groups, [tree], building) == "locked"
    assert outliner.reparent_problem(groups, [level1], None) is None


def test_a_locked_group_cannot_be_picked_nor_what_it_holds():
    from core.scene import Scene
    building, tree = _building()
    scene = Scene()
    scene.groups[:] = [building, tree]
    building.locked = True
    assert not scene.entity_selectable(building)
    proxy = Group(name="placement")
    proxy.owner = building                     # what the viewport picks
    assert not scene.entity_selectable(proxy)
    assert scene.entity_selectable(tree)


def test_the_lock_is_saved_in_the_igz_and_kept_by_copies(tmp_path):
    from core.scene import Scene
    from formats.igz import load_into, save_scene
    building, tree = _building()
    tree.locked = True
    assert copy_group(tree).locked
    scene = Scene()
    scene.groups[:] = [building, tree]
    path = tmp_path / "locked.igz"
    save_scene(scene, path)
    back = Scene()
    load_into(back, path)
    by_name = {g.name: g for g, _p, _d in outliner.walk(back.groups)}
    assert by_name["Tree"].locked and not by_name["Building"].locked


def _two_planes(scene):
    from core.section import SectionPlane
    a = SectionPlane(QVector3D(0, 0, 1.2), QVector3D(0, 0, 1),
                     name="Plan cut", symbol="A", active=True)
    b = SectionPlane(QVector3D(2, 0, 0), QVector3D(1, 0, 0),
                     name="Cross", symbol="B")
    scene.section_planes[:] = [a, b]
    return a, b


def test_section_modes_in_the_core():
    from core.scene import Scene
    scene = Scene()
    a, b = _two_planes(scene)
    assert outliner.section_mode(a) == "both"
    assert outliner.section_mode(b) == "frame"
    b.hidden = True
    assert outliner.section_mode(b) == "off"
    a.hidden = True
    assert outliner.section_mode(a) == "cut"
    assert outliner.section_label(a) == "A — Plan cut"


def test_a_hidden_frame_is_saved_and_not_picked(tmp_path):
    from core.scene import Scene
    from core.section import SectionPlane
    from formats.igz import load_into, save_scene
    scene = Scene()
    a, b = _two_planes(scene)
    a.hidden = True
    path = tmp_path / "sections.igz"
    save_scene(scene, path)
    back = Scene()
    load_into(back, path)
    assert [sp.hidden for sp in back.section_planes] == [True, False]
    assert back.section_planes[0].active
    assert SectionPlane.from_dict(b.to_dict()).hidden is False


def test_world_bounds_follow_the_matrices_down_the_tree():
    shelf = _moved_container_for_bounds()
    lo, hi = outliner.world_bounds([shelf], shelf.children[0])
    assert abs(lo[0] - 10.0) < 1e-6 and abs(lo[2] - 2.0) < 1e-6
    assert abs(hi[0] - 10.1) < 1e-6
    assert outliner.world_bounds([Group(name="empty")],
                                 Group(name="empty")) is None


def _moved_container_for_bounds():
    m = QMatrix4x4()
    m.translate(QVector3D(10, 0, 2))
    kid = Group(_box((0, 0, 0)), name="Kid")
    kid.xform = QMatrix4x4()
    kid.component = False
    box = _container("Shelf", [kid])
    box.xform = m
    return box


def test_free_slot_fills_the_page_then_steps():
    from core.composition import Composicion, MarcoVista
    sheet = Composicion(name="S", paper="A3")
    pw, ph = sheet.page_size_mm()
    w, h = (pw - 20) / 2 - 15, (ph - 20) / 2 - 15
    spots = []
    for _ in range(4):
        x, y = outliner.free_slot(sheet, w, h)
        sheet.frames.append(MarcoVista(x_mm=x, y_mm=y, w_mm=w, h_mm=h))
        spots.append((x, y))
    assert len(set(spots)) == 4
    assert all(x + w <= pw - 10 and y + h <= ph - 10 for x, y in spots)


def test_component_copies_are_counted():
    from tests.test_project_drawing_filters import Project
    p = Project()
    proto = Group(_box(), name="Chair")
    proto.xform = QMatrix4x4()
    other = copy_group(proto, QVector3D(1, 0, 0))
    p.scene.groups.extend([proto, other])
    p.action("toggle", "n:")
    definition, = [t for t, _ in p.view.renderer.calls["ui_set_tree"][0] if t[0].startswith("d:")]
    assert definition[1] == "Chair" and definition[3:5] == ["×2", "component"]
    rows = p.action("toggle", definition[0])
    assert all(rows["i:" + g.uid][0] == "Chair" for g in (proto, other))


def test_rename_in_the_tree_is_one_undo_step_and_keeps_the_uid():
    from tests.test_project_drawing_filters import Project
    from views.wgpu_ui import SEP
    p = Project()
    tree = _group("Tree")
    p.scene.groups.append(tree)
    uid = tree.uid
    p.action("toggle", "n:")
    rows = p.action("rename", "i:" + uid + SEP + "Oak")
    assert tree.name == rows["i:" + uid][0] == "Oak" and tree.uid == uid
    assert len(p.win.viewport.history.undo_stack) == 1
    assert p.win.viewport.history.undo() and tree.name == p.rows()["i:" + uid][0] == "Tree"


def test_the_padlock_locks_and_unlocks_in_one_undo_step():
    from tests.test_project_drawing_filters import Project
    p = Project()
    tree = _group("Tree")
    p.scene.groups.append(tree)
    row = "i:" + tree.uid
    p.action("toggle", "n:")
    assert p.action("lock", row)[row][1][6] == 1 and tree.locked
    p.action("item", row)
    assert not p.scene.selection
    assert len(p.win.viewport.history.undo_stack) == 1 and p.win.viewport.history.undo()
    assert not tree.locked and p.rows()[row][1][6] == 0


def test_a_scene_remembers_what_the_outliner_hid():
    from core.camera import OrbitCamera
    from core.saved_views import SavedView
    from tests.test_project_drawing_filters import Project
    p, camera = Project(), OrbitCamera()
    tree = _group("Tree")
    p.scene.groups.append(tree)
    row = "i:" + tree.uid
    p.action("toggle", "n:")
    assert p.action("eye", row)[row][1][5] == 0
    view = SavedView.capture("Without trees", p.scene, camera)
    assert p.action("eye", row)[row][1][5] == 1
    view.apply(p.scene, camera)
    p.scene.version += 1
    assert tree.hidden and p.rows()[row][1][5] == 0


def test_several_rows_go_to_a_layer_in_one_step():
    from core import structure
    from tests.test_project_drawing_filters import Project
    from tests.test_structure import model
    scene, _, _, _ = model()
    p = Project(scene=scene)
    wall, slab = _group("Wall"), _group("Slab")
    p.scene.groups.extend([wall, slab])
    p.rows()
    p.scene.select([wall, slab])
    p.action("filehere", "n:sections")
    assert all(structure.node_of(g) == "sections" and scene.layer(g.layer).name == "Track Sections" for g in (wall, slab))
    assert len(p.win.viewport.history.undo_stack) == 1 and p.win.viewport.history.undo()
    assert all(g.layer is None and structure.node_of(g) is None for g in (wall, slab))
