# SPDX-License-Identifier: GPL-3.0-or-later
"""After a change of a huge selection the viewport shows it first; the
panels follow one a frame (the selection bar, the tree's rows, then the
inspector), each reading the selection by kind rather than handle by handle,
so no frame of the catch-up reads 600k entries three times."""
import pytest

import built_render
from tests.shell_isolation import native_gpu_host  # noqa: F401  (fixture)
from tests.test_huge_selection import grid


@pytest.fixture
def big(native_gpu_host):
    win = native_gpu_host.window
    scene = win.viewport.scene
    grid(scene.mesh, 160)                 # 25,600 faces and 51,520 edges
    native_gpu_host.idle()
    native_gpu_host.frame()
    return native_gpu_host


def test_panels_catch_up_one_a_frame(big):
    host = big
    win, view = host.window, host.view
    scene = win.viewport.scene
    win._on_select_all()
    assert len(scene.selection) > 50_000
    stages = []
    for _ in range(5):
        host.idle()
        host.frame()
        stages.append(view._panels_stage)
    assert stages == [0, 1, 2, 3, 3]
    # A small selection reaches every panel on its own frame.
    scene.select(scene.mesh.faces[:3])
    host.idle()
    host.frame()
    assert view._panels_stage == 3


def test_bulk_facts_are_the_handle_by_handle_facts(big):
    """The selection bar's facts read by kind equal those read entry by entry."""
    win = big.window
    scene = win.viewport.scene
    win._on_select_all()
    by_kind = built_render.inspector_verbs(win, scene.selection)
    by_entry = built_render.inspector_verbs(win, list(scene.selection))
    strip = lambda verbs: [(v[0], v[1], v[2], v[4], v[5]) for v in verbs]
    assert strip(by_kind) == strip(by_entry)
    assert built_render.selection_kind(scene, scene.selection) == built_render.selection_kind(scene, list(scene.selection))


def test_tree_rows_read_only_what_rows_can_be(big):
    """The tree's selected ids leave the mesh elements out (no row is one)."""
    win = big.window
    scene = win.viewport.scene
    win._on_select_all()
    ids = built_render.browser_row_selection(scene)
    assert ids == set()
    assert len(built_render.browser_selection(scene)) == len(scene.selection)
