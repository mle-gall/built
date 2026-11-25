# SPDX-License-Identifier: GPL-3.0-or-later
"""A Library component keeps focus across a selection and a sheet refresh."""
from core.group import copy_group
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_in_model_nested import _component, _quad
from tests.test_library_sheet import act, frame, tiles, win  # noqa: F401
from built_render.wgpu_library import open_library


def test_the_clicked_row_is_still_lit_after_the_refresh(win):
    scene = win.viewport.scene
    silla = _component(_quad(), "Silla")
    mesa = _component(_quad(2.), "Mesa", dx=5.)
    scene.groups.extend([silla, copy_group(silla), mesa])
    scene.version += 1
    assert open_library(win, "components")
    key = next(row["id"] for row in tiles(frame(win)) if row["label"] == "Silla")
    act(win, "focus", key)
    act(win, "verb", "select")
    assert {g.name for g in scene.selection} == {"Silla"}
    scene.version += 1
    assert frame(win)["focus"] == key
    scene.select([])
    assert not scene.selection
    assert frame(win)["focus"] == key
