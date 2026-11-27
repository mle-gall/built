# SPDX-License-Identifier: GPL-3.0-or-later
"""See-through while carrying (.skp parity, the owner's observation): the
objects a Move or Rotate carries draw translucent, with their edges, so a
grip or a snap behind them shows without orbiting; the pick and the
inference look through them; the end of the carry (or Esc) puts them back.

The renderer draws only the carried rows in its see-through pass: one
indirect command each, never a second pass over the scene."""
from __future__ import annotations

import numpy as np
import pytest

from tests.headless_gpu import HAS_RENDERER


@pytest.fixture
def native():
    if not HAS_RENDERER:
        pytest.skip("requires the render crate")
    from built_core import Matrix4 as QMatrix4x4
    from tests.headless_gpu import renderer as headless
    renderer = headless(64, 64, 1.)
    view = QMatrix4x4(); view.translate(0, 0, -3)
    proj = QMatrix4x4(); proj.ortho(-1, 1, -1, 1, .1, 10)
    renderer.set_camera(list(view.data()), list(proj.data()))
    renderer.set_antialias(False); renderer.set_edges(False)
    yield renderer
    del renderer


def two_objects(renderer):
    """A red square (row 0, carried) in front of a blue one (row 1)."""
    from built_core import VERTEX, INSTANCE, COMMAND
    vertices = np.zeros(12, dtype=VERTEX)
    quad = np.array([[-.8, -.8, 0], [.8, -.8, 0], [.8, .8, 0], [-.8, -.8, 0], [.8, .8, 0], [-.8, .8, 0]], dtype=np.float32)
    vertices["pos"][:6] = quad
    quad[:, 2] = -.4
    vertices["pos"][6:] = quad
    vertices["normal"][:, 2] = 1
    vertices["color"][:6] = [255, 0, 0, 255]
    vertices["color"][6:] = [0, 0, 255, 255]
    indices = np.arange(12, dtype=np.uint32)
    bounds = np.array([[[-.8, -.8, 0, 0], [.8, .8, 0, 0]], [[-.8, -.8, -.4, 0], [.8, .8, -.4, 0]]], dtype=np.float32)
    commands = np.zeros(4, dtype=COMMAND)
    commands["count"][:2] = 6
    commands["first"][1] = 6
    commands["start"][1] = 1
    commands["start"][3] = 1
    rows = np.zeros(2, dtype=INSTANCE)
    rows["world"][:] = np.eye(4, dtype=np.float32)
    rows["info"][1, 0] = 1
    renderer.set_scene(tuple(x.view(np.uint8).ravel() for x in (vertices, indices, bounds, commands)),
                       rows.view(np.uint8).ravel())
    return rows


def capture(renderer):
    renderer.request_capture(); renderer.render(False)
    return np.frombuffer(bytes(renderer.take_capture()), np.uint8).reshape(64, 64, 4)


@pytest.mark.parametrize("aa", [False, True])
def test_the_carried_object_shows_what_is_behind_it_and_esc_restores(native, aa):
    two_objects(native)
    native.set_antialias(aa)
    native.set_style(dict(sky=False, edges=False))
    before = capture(native)
    assert before[32, 32, 0] > 200 and before[32, 32, 2] < 10      # red in front
    assert native.pick(32, 32) == 0
    native.set_see_through([0])
    assert native.see_through_rows == [0]
    seen = capture(native)[32, 32, :3].astype(int)
    # The blue behind shows through the red at the kit's 35 %.
    assert seen[2] > 120 and 40 < seen[0] < 160, seen
    # The pick looks through the carried object to the one behind.
    assert native.pick(32, 32) == 1
    native.set_see_through([])
    np.testing.assert_array_equal(before, capture(native))
    assert native.pick(32, 32) == 0


def test_a_carried_row_keeps_its_see_through_across_pose_updates(native):
    rows = two_objects(native)
    native.set_style(dict(sky=False, edges=False))
    native.set_see_through([0])
    rows["world"][0, 3, 0] = .1                     # the carry moves it
    native.update_instances([0], rows[:1].view(np.uint8).ravel())
    seen = capture(native)[32, 32, :3].astype(int)
    assert seen[2] > 120, seen
    native.set_see_through([])
    assert capture(native)[32, 32, 0] > 200


def test_out_of_range_rows_are_ignored(native):
    two_objects(native)
    native.set_see_through([7])
    assert native.see_through_rows == []
