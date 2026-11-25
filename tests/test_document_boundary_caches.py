# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Opening a document forgets every viewport cache keyed by ``id()`` (#75).

@pacaeiro: new drawing, some changes, Open a saved (big) design — «Sumari Man
got transported to the freshly opened design». The viewport caches per-group
data by ``id()``, and CPython hands a dead object's address to the next one:
a group of the opened file could meet the scale figure's entry. Every such
cache has to go at the document boundary."""
from __future__ import annotations

from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401
import re
from pathlib import Path


def test_open_forgets_every_id_keyed_cache(native_window, tmp_path):
    from core.scene import Scene
    from formats import igz
    path = tmp_path / "other.igz"
    igz.save_scene(Scene(), path)
    win = native_window
    vp = win.viewport
    _seed_inference_references(vp)
    for name in vp._DOCUMENT_CACHES:
        setattr(vp, name, {12345: "stale"})
    win._saved_version = vp.scene.version
    assert win.open_path(path)
    for name in vp._DOCUMENT_CACHES:
        assert not getattr(vp, name), name
    _assert_inference_references_cleared(vp)


def _seed_inference_references(vp):
    from built_core import Vec3 as QVector3D

    inference = vp.inference
    inference._hover_edge = object()
    inference._hover_center = object()
    inference._center_ref = object()
    inference._acquired_edge = object()
    inference._acquired_face_normal = QVector3D(0, 0, 1)
    inference._acquired_point = QVector3D(1, 2, 3)
    inference._encouraged = [QVector3D(4, 5, 6)]
    inference._dwell_point = QVector3D(7, 8, 9)
    inference._dwell_timer.start()


def _assert_inference_references_cleared(vp):
    inference = vp.inference
    assert inference._hover_edge is None
    assert inference._hover_center is None
    assert inference._center_ref is None
    assert inference._acquired_edge is None
    assert inference._acquired_face_normal is None
    assert inference._acquired_point is None
    assert inference._encouraged == []
    assert inference._dwell_point is None
    assert not inference._dwell_timer.isActive()


def test_new_drawing_drops_acquired_inference_references(native_window):

    win = native_window
    _seed_inference_references(win.viewport)
    win._confirm_discard = lambda *_a, **_k: True
    win._on_new()
    _assert_inference_references_cleared(win.viewport)


def test_undo_drops_acquired_inference_references(native_window):
    from built_core import Vec3 as QVector3D

    from core.history import AddEdgeCommand

    win = native_window
    vp = win.viewport
    vp.history.execute(AddEdgeCommand(QVector3D(0, 0, 0),
                                       QVector3D(1, 0, 0)))
    _seed_inference_references(vp)
    vp.axis_lock = "x"
    win._on_undo()
    _assert_inference_references_cleared(vp)
    assert vp.axis_lock == "x"
    assert not vp.history.undo_stack


def test_native_document_boundary_discards_each_placement_and_mesh_cache(native_window):
    """Exercise the reset for every dictionary allocated by native placement cache helpers."""
    crates = Path(__file__).resolve().parents[1] / "rust"
    cache_helpers = {
        "built_core/src/document_snap": "dict_slot",
        "built_render/src/document_chunks": "memo",
        "built_render/src/document_placements": "cache",
    }
    caches = set()
    for directory, helper in cache_helpers.items():
        allocation = re.compile(rf'\b{helper}\(\s*\w+\s*,\s*"(_\w+)"\s*\)')
        for source in (crates / directory).rglob("*.rs"):
            caches.update(allocation.findall(source.read_text(encoding="utf-8")))
    assert {"_proto_pts_store", "_group_chunks", "_placement_frames"} <= caches
    view = native_window.viewport
    for name in caches:
        setattr(view, name, {12345: "from previous document"})
    view.reset_document_caches()
    assert not {name for name in caches if getattr(view, name, None)}
