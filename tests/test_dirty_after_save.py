# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #159 (@pacaeiro): open a drawing, erase the scale figure, Ctrl+S,
close -- "there are unsaved changes", three times in a row. The document was
clean after the save; the next click (on empty space, on an object, in the
Components list) changed the SELECTION, and every selection change bumped
``scene.version``, which is also what "unsaved changes" compared. Selecting
is not editing: only a change of the document may make it dirty."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
import pytest
from built_core import Vec3 as QVector3D


@pytest.fixture
def doc(tmp_path):
    from built_core.group import Group
    from built_core.mesh import Mesh
    from built_core.scene import Scene
    from formats.igz import save_scene

    scene = Scene()
    for i in range(6):
        m = Mesh()
        x = i * 3.0
        m.add_face([QVector3D(x, 0, 0), QVector3D(x + 2, 0, 0),
                    QVector3D(x + 2, 2, 0), QVector3D(x, 2, 0)])
        scene.groups.append(Group(m, name=f"B{i}"))
    scene.version += 1
    path = tmp_path / "plan.built"
    save_scene(scene, path, backup=False)
    return path


def _window(native_host, path):
    win = native_host.window
    assert win.open_path(path)
    return win


def test_selection_after_ctrl_s_does_not_ask_to_save_again(native_host, doc):
    win = _window(native_host, doc)
    scene = win.viewport.scene
    figure = scene.groups[0]                  # a group from the saved file
    scene.select([figure])
    win._on_delete_selection()                # erase it: a real edit
    assert win._is_dirty()
    win._on_save()                            # Ctrl+S
    assert not win._is_dirty()
    # what a user does before closing: click empty space, click an
    # object, Ctrl+Shift+I, Esc
    scene.clear_selection()
    scene.select([scene.groups[0]])
    scene.select([], mode="replace")
    scene.invert_selection()
    scene.clear_selection()
    assert not win._is_dirty(), "a selection made the saved drawing 'modified'"
    assert "*" not in native_host._title

    # the inverse: a real edit after the save does make it dirty
    scene.select([scene.groups[0]])
    win._on_delete_selection()
    assert win._is_dirty()


def test_selection_keeps_bumping_the_render_version(native_host, doc):
    """The GL colour caches key on scene.version: a selection must still
    bump it, only not count as an edit."""
    win = _window(native_host, doc)
    scene = win.viewport.scene
    before, content = scene.version, scene.content_version
    scene.select([scene.groups[0]])
    assert scene.version == before + 1
    assert scene.content_version == content


def test_marking_saved_with_scene_version_still_means_clean(native_host, doc):
    """Every caller (and 100+ tests) marks "saved" by assigning the scene
    version; after selections that must still read as clean."""
    win = _window(native_host, doc)
    scene = win.viewport.scene
    scene.select([scene.groups[0]])
    scene.clear_selection()
    win._saved_version = scene.version
    assert not win._is_dirty()
    win._saved_version = -1                   # "never saved" stays dirty
    assert win._is_dirty()
