# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""A window-close request survives file loading, while job cancellation leaves it open."""
from pathlib import Path
import sys

from ingetrazo_core import Vec3
import pytest

from core.group import Group
from core.mesh import Mesh
from core.scene import Scene
from formats import igz
from scripts import isolated_launch
from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.fixture
def doc(tmp_path):
    scene = Scene()
    for i in range(6):
        mesh = Mesh()
        x = i * 3.0
        mesh.add_face([Vec3(x, 0, 0), Vec3(x + 2, 0, 0),
                       Vec3(x + 2, 2, 0), Vec3(x, 2, 0)])
        scene.groups.append(Group(mesh, name=f"B{i}"))
    path = tmp_path / "plan.igz"
    igz.save_scene(scene, path)
    return path


def test_open_queued_behind_an_accepted_close_is_refused(native_host, doc, monkeypatch):
    assert native_host.request_close()
    monkeypatch.setattr(igz, "load_into", lambda *a, **k: pytest.fail("opened after close"))
    assert not native_host.open_path(doc)
    assert native_host.window._current_path is None


@pytest.mark.parametrize("command", ["close", "quit"])
def test_close_during_the_load_unwinds_it_and_closes(doc, tmp_path, command):
    root = Path(__file__).resolve().parents[1]
    binary = root / "rust/built_app/target/release/built"
    if sys.platform != "darwin" or not binary.is_file():
        pytest.skip("requires the macOS release shell")
    profile = tmp_path / "settings"
    plugin = profile / "data/ingetrazo/plugins/paused_reader.py"
    plugin.parent.mkdir(parents=True)
    # Keep the second worker at a progress boundary independently of CPU speed.
    # The real open job still creates, polls, cancels and releases the worker.
    plugin.write_text("""from pathlib import Path
import ingetrazo_core

def setup(app):
    reader = ingetrazo_core.DocumentOpen

    class HeldReader:
        def __init__(self, worker):
            self.worker = worker

        def progress(self):
            return 0.4, "Loading"

        def ready(self):
            return False

        def cancel(self):
            self.worker.cancel()
            Path(__file__).with_suffix(".cancelled").touch()

    def start(path, *args, **kwargs):
        worker = reader(path, *args, **kwargs)
        return HeldReader(worker) if Path(path).name == "plan.igz" else worker

    ingetrazo_core.DocumentOpen = start
""")
    script = tmp_path / "close.txt"
    # Start from a real window, then close during a worker open. A lost close
    # reaches the second state. The script's own quit is just a timeout guard.
    script.write_text(f"state\nopen-async {doc}\nmenu {command}\nwait 1200\nstate\nquit\n")
    result = isolated_launch.run(
        [binary, "--new-instance", "--root", root, "--venv", root / ".venv", "--size",
         "900x600", "--script", script, root / "tests/fixtures/shell/slab-and-cube.igz"],
        settings=profile, capture_output=True, text=True, timeout=60)
    assert result.returncode == 0, result.stderr
    assert plugin.with_suffix(".cancelled").exists(), result.stderr
    assert result.stdout.count("[built] state ") == 1, result.stdout
    assert "[built] error" not in result.stderr


def test_cancelling_the_job_during_the_load_keeps_the_window(native_host, doc, monkeypatch):
    from ingetrazo_render.chrome_dialogs import ChromeBar
    win = native_host.window
    bar = ChromeBar(win, "Opening")
    monkeypatch.setattr(win, "_import_progress", lambda *a, **kw: (bar, lambda *a: None))
    milestones = []

    def load(scene, path, progress):
        for i in range(10):
            scene.version += 1
            milestones.append(i)
            progress(i / 10.0, "Loading")
            if i == 1:
                bar.job.cancel()

    monkeypatch.setattr(igz, "load_into", load)
    assert not native_host.open_path(doc)
    assert milestones == [0, 1, 2]
    assert not win._is_dirty() and not win._closed
    assert win._current_path is None


def test_the_bar_closing_after_the_load_leaves_the_window_open(native_host, doc, monkeypatch):
    from ingetrazo_render.chrome_dialogs import ChromeBar
    win = native_host.window
    bar = ChromeBar(win, "Opening")
    monkeypatch.setattr(win, "_import_progress", lambda *a, **kw: (bar, lambda *a: None))
    assert native_host.open_path(doc)
    assert not bar.isVisible() and not win._closed and not win._open_aborted
    assert native_host.open_path(doc)


def test_a_failed_open_does_not_cut_the_next_one(native_host, doc, monkeypatch):
    with monkeypatch.context() as patch:
        patch.setattr("ingetrazo_render.chrome_dialogs.critical", lambda *a, **kw: None)
        def broken(scene, path, progress):
            progress(0.5, "Loading")
            raise ValueError("corrupt")
        patch.setattr(igz, "load_into", broken)
        assert not native_host.open_path(doc)
    assert native_host.open_path(doc)
    assert not native_host.window._closed


def test_a_normal_open_still_opens(native_host, doc):
    assert native_host.open_path(doc)
    assert native_host.window._current_path == doc
    assert not native_host.window._closed
