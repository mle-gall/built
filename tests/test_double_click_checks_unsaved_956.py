# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Opening a file handed to the running app preserves the user's drawing (#956)."""
import pytest

from built_core.scene import Scene
from formats import igz
from built_core import Vec3 as V
from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.fixture
def donor(tmp_path):
    scene = Scene()
    scene.mesh.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)])
    path = tmp_path / "donor.igz"
    igz.save_scene(scene, path)
    return path


@pytest.mark.parametrize(("answer", "closed"), [("cancel", False), ("discard", True)])
def test_discard_is_asked_only_when_closing_the_original_drawing(native_host, donor,
                                                                 monkeypatch, answer, closed):
    win = native_host.window
    dirty = win.active_document()
    dirty.viewport.scene.version += 1
    asked = []
    def reply(*args, **kwargs):
        asked.append(win.active_document())
        return answer
    monkeypatch.setattr("built_render.chrome_dialogs.ask", reply)
    assert native_host.open_path(donor)
    assert not asked and dirty in win.documents()
    assert win.close_document(dirty) is closed
    assert asked == [dirty] and (dirty not in win.documents()) == closed
    assert any(doc.path == donor for doc in win.documents())


def test_an_external_open_needs_no_discard_hook(native_host, donor, monkeypatch):
    monkeypatch.setattr(native_host.window, "_confirm_discard",
                        lambda *_: pytest.fail("external open consulted discard"))
    assert native_host.open_path(donor)
    assert native_host.window.active_document().path == donor


def test_an_import_is_not_asked_it_only_adds(native_host, tmp_path, monkeypatch):
    path = tmp_path / "model.skp"
    path.touch()
    win = native_host.window
    calls = []
    monkeypatch.setattr(win, "import_skp_path", lambda p: calls.append(p) or True)
    monkeypatch.setattr(win, "_confirm_discard", lambda *args: pytest.fail("unexpected discard"))
    assert native_host.open_path(path)
    assert calls == [path]


def test_the_drawing_comes_in_a_tab_and_the_unsaved_one_stays(native_host, donor, monkeypatch):
    win = native_host.window
    mine = win.active_document()
    mine.viewport.scene.mesh.add_face([V(9, 9, 9), V(11, 9, 9), V(11, 11, 9), V(9, 11, 9)])
    mine.viewport.scene.version += 1
    assert win._is_dirty()
    monkeypatch.setattr("built_render.chrome_dialogs.ask", lambda *a, **k: pytest.fail("unexpected question"))
    assert native_host.open_path(donor)
    assert len(win.documents()) == 2 and win._current_path == donor
    faces = list(win.viewport.scene.mesh.faces)
    assert len(faces) == 1 and faces[0].vertices[0].z() == 0.0
    assert mine.path is None and win._doc_is_dirty(mine)
    kept = list(mine.viewport.scene.mesh.faces)
    assert len(kept) == 1 and kept[0].vertices[0].z() == 9.0


def test_a_clean_window_is_never_asked(native_host, donor, monkeypatch):
    monkeypatch.setattr("built_render.chrome_dialogs.ask", lambda *a, **k: pytest.fail("unexpected question"))
    original = native_host.window.active_document()
    assert native_host.open_path(donor)
    assert native_host.window.documents() == [original]
    assert original.path == donor
