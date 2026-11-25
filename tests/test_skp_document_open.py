# SPDX-License-Identifier: GPL-3.0-or-later
from pathlib import Path

import pytest

from built_core.history import History
from built_core.scene import Scene
import built_core as native

DATA = Path(__file__).parent / "data"
PANEL = DATA / "skp_nested_panel_2021.skp"


def test_worker_can_open_while_another_document_is_being_edited():
    current = Scene()
    from built_core.group import Group
    current.groups.append(Group(name="original"))
    native.doc_begin()
    try:
        current.groups[0].name = "edit in progress"
        scene, (packed, _, _), history, empty = native.DocumentOpen(PANEL).wait()
        assert not empty and len(packed.instances) > 0
        first, second = scene.groups[0].children
        assert first.mesh is second.mesh
        assert len(next(iter(first.mesh.faces)).hole_loops) == 1
        assert scene.layer("Panels").folder == "Fabrication"
        assert current.groups[0].name == "edit in progress"
    finally:
        native.doc_rollback()
    assert current.groups[0].name == "original"
    history.undo()
    assert not scene.groups and not scene.layer_folders and scene.back_face_color is None
    history.redo()
    assert scene.groups[0].children[1] is second
    assert scene.layer("Panels").folder == "Fabrication"


def test_repeated_import_retains_placement_identity_across_undo():
    scene = Scene()
    history = History(scene)
    before = scene.back_face_color
    history.execute(native.DocumentOpen(PANEL, insert=True).wait())
    assert not history.last_error
    first = set(scene.groups_by_uid())
    assert len(first) == 3
    history.execute(native.DocumentOpen(PANEL, insert=True).wait())
    assert not history.last_error
    both = set(scene.groups_by_uid())
    assert first < both and len(both) == 6
    assert len(scene.layer_folders) == 1
    history.undo()
    assert set(scene.groups_by_uid()) == first
    history.undo()
    assert not scene.groups and scene.back_face_color == before
    history.redo()
    history.redo()
    assert set(scene.groups_by_uid()) == both
    assert scene.groups[1].children[0].mesh is scene.groups[1].children[1].mesh


def test_cancel_skp_does_not_deliver_a_partial_result():
    worker = native.DocumentOpen(PANEL, insert=True)
    worker.cancel()
    assert worker.take() is None
    with pytest.raises(InterruptedError):
        worker.wait()


def test_first_insertion_keeps_callbacks_for_a_queued_document():
    from types import SimpleNamespace
    from built_render.document_open import after_open

    path = DATA / "skp_arcs_2017.skp"
    active = SimpleNamespace(path=PANEL, target=object(), queue=[path])
    window = SimpleNamespace(_pending_open=active)
    results = []
    after_open(window, path, results.append)
    assert not results
    window._open_callbacks[path][0]("queued drawing")
    assert results == ["queued drawing"]
