# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Pending document work waits for a picture of the newly opened document."""
from core.scene import Scene
from formats import igz
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_open_runs_optional_work_after_the_first_picture(native_host, tmp_path, monkeypatch):
    path = tmp_path / "empty.igz"
    igz.save_scene(Scene(), path)
    win = native_host.window
    order = []
    monkeypatch.setattr(win, "_after_links_loaded", lambda: order.append("links"))
    monkeypatch.setattr(win, "settle_heap", lambda: order.append("heap"))
    assert win.open_path(path)
    native_host.idle()
    assert order == []
    monkeypatch.setattr(native_host.view, "draw_frame", lambda: (0., 0., 0.))
    native_host.frame()
    assert order == []
    native_host.idle()
    assert order == ["links"]
    native_host.idle()
    assert order == ["links", "heap"]


def test_previous_frame_cannot_release_new_work(native_host, monkeypatch):
    draw = iter([(0., 0., 0.), None, (1., 0., 1.)])
    monkeypatch.setattr(native_host.view, "draw_frame", lambda: next(draw))
    native_host.frame()
    calls = []
    native_host.after_picture(lambda: calls.append("work"))
    native_host.idle()
    assert not calls
    native_host.frame()
    native_host.idle()
    assert not calls
    native_host.frame()
    native_host.idle()
    assert calls == ["work"]
