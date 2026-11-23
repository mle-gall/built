# SPDX-License-Identifier: GPL-3.0-or-later
"""Protocol verbs of R2.5 slice 3: add_face, move_placements and
copy_placements (docs/core-protocol.md §1)."""
import numpy as np
import pytest
from ingetrazo_core import Matrix4, Vec3

from core.group import Group
from core.history import History
from core.mesh import Mesh
from core.protocol.commands import run
from core.scene import Scene


def _scene_with_box():
    scene = Scene()
    box = Mesh()
    box.add_face([(0., 0., 0.), (1., 0., 0.), (1., 1., 0.), (0., 1., 0.)])
    g = Group(box)
    g.uid = "box"
    g.xform = Matrix4()
    scene.groups.append(g)
    return scene, History(scene), g


def test_add_face_draws_the_ring_and_its_face_in_one_step():
    scene = Scene()
    history = History(scene)
    answer = run(history, "add_face", outer=[[0, 0, 0], [2, 0, 0], [2, 1, 0], [0, 1, 0]])
    assert answer["ok"]
    assert len(scene.mesh.faces) == 1 and len(scene.mesh.edges) == 4
    assert len(history.undo_stack) == 1
    assert history.undo()
    assert len(scene.mesh.faces) == 0 and len(scene.mesh.edges) == 0


def test_add_face_with_a_hole_and_over_a_face_fills_it():
    scene = Scene()
    history = History(scene)
    run(history, "add_face", outer=[[0, 0, 0], [4, 0, 0], [4, 4, 0], [0, 4, 0]],
        holes=[[[1, 1, 0], [1, 3, 0], [3, 3, 0], [3, 1, 0]]])
    assert len(scene.mesh.faces) == 1
    assert len(scene.mesh.faces[0].hole_loops) == 1
    run(history, "add_face", outer=[[1, 1, 0], [3, 1, 0], [3, 3, 0], [1, 3, 0]])
    assert len(scene.mesh.faces) == 2


def test_add_face_offsets_land_exactly_far_from_the_origin():
    scene = Scene()
    history = History(scene)
    start = np.array([650000.012345678, 6860000.023456789, 0.])
    scene.mesh.add_edge(tuple(start - [.02, 0., 0.]), tuple(start))
    w, h = .022345678912, .038912345678
    offsets = [[0, 0, 0], [w, 0, 0], [w, h, 0], [0, h, 0]]
    assert run(history, "add_face", anchor=list(start), offsets=offsets)["ok"]
    far = start + [w, h, 0.]
    np.testing.assert_array_equal(scene.mesh.vertex_at(Vec3(*far)).position_tuple(), far)


def test_move_placements_is_one_step():
    scene, history, g = _scene_with_box()
    assert run(history, "move_placements", placements=["box"], delta=[1, 2, 0])["ok"]
    assert g.xform.map(Vec3(0, 0, 0)).toTuple() == (1.0, 2.0, 0.0)
    assert history.undo()
    assert g.xform.map(Vec3(0, 0, 0)).toTuple() == (0.0, 0.0, 0.0)
    with pytest.raises(KeyError):
        run(history, "move_placements", placements=["nothing"], delta=[1, 0, 0])


@pytest.mark.parametrize("mode, xs", [("x", [2.0, 4.0, 6.0]), ("/", [2 / 3, 4 / 3, 2.0])])
def test_copy_placements_lays_an_array_in_one_step(mode, xs):
    scene, history, g = _scene_with_box()
    answer = run(history, "copy_placements", placements=["box"], delta=[2, 0, 0],
                 count=3, mode=mode)
    assert answer["ok"] and len(answer["placements"]) == 3
    assert len(scene.groups) == 4 and len(history.undo_stack) == 1
    got = sorted(c.xform.map(Vec3(0, 0, 0)).x() for c in scene.groups if c is not g)
    assert got == pytest.approx(xs)
    assert len({c.uid for c in scene.groups}) == 4
    history.undo()
    assert scene.groups == [g]


def test_move_selection_moves_the_selection_in_one_step():
    scene, history, g = _scene_with_box()
    face = scene.mesh.add_face([(5., 0., 0.), (6., 0., 0.), (6., 1., 0.), (5., 1., 0.)])
    scene.select([g, face])
    assert run(history, "move_selection", delta=[0, 0, 2])["ok"]
    assert g.xform.map(Vec3(0, 0, 0)).toTuple() == (0.0, 0.0, 2.0)
    assert all(abs(v.z() - 2.0) < 1e-12 for v in scene.mesh.faces[0].vertices)
    assert len(history.undo_stack) == 1
    history.undo()
    assert all(abs(v.z()) < 1e-12 for v in scene.mesh.faces[0].vertices)


def test_move_selection_by_keys_and_as_an_array_of_copies():
    scene, history, g = _scene_with_box()
    scene.mesh.add_face([(5., 0., 0.), (6., 0., 0.), (6., 1., 0.), (5., 1., 0.)])
    (face,) = scene.mesh.faces
    assert run(history, "move_selection", delta=[1, 0, 0], faces=[face.key])["ok"]
    assert min(v.x() for v in scene.mesh.faces[0].vertices) == pytest.approx(6.0)
    answer = run(history, "move_selection", delta=[0, 3, 0], placements=["box"],
                 faces=[scene.mesh.faces[0].key], copy=True, count=2)
    assert answer["ok"] and len(history.undo_stack) == 2
    assert len(scene.groups) == 3 and len(scene.mesh.faces) == 3
