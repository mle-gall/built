# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""core/black_box.py: what the crash log learns besides fatal signals —
Rust panics, deliberate exits, and a main thread stuck in Python (a hang
the user force-quits leaves nothing else: 2026-11-18)."""
from __future__ import annotations

import threading
import time

import pytest

from core import black_box


@pytest.fixture
def log(tmp_path):
    path = tmp_path / "crash.log"
    f = open(path, "a", encoding="utf-8")
    saved = black_box._log
    black_box.attach(f)
    yield path
    black_box._log = saved
    f.close()


def test_note_appends_a_stamped_line(log):
    black_box.note("exit 0 (normal quit, os._exit)")
    text = log.read_text()
    assert text.startswith("[20") and "exit 0 (normal quit" in text


def test_note_without_a_log_is_silent():
    saved = black_box._log
    black_box._log = None
    try:
        black_box.note("nothing to write to")
    finally:
        black_box._log = saved


@pytest.mark.parametrize("name", ["ingetrazo_core", "ingetrazo_render"])
def test_a_rust_panic_lands_in_the_log(log, name):
    module = pytest.importorskip(name)
    if not hasattr(module, "_panic_for_test"):
        pytest.skip("native module built before the panic hook")
    assert black_box.install_panic_log(module)
    with pytest.raises(BaseException) as raised:        # PyO3's PanicException
        module._panic_for_test(f"evidence from {name}")
    assert "evidence from" in str(raised.value)
    text = log.read_text()
    assert f"Rust panic in {name}" in text
    assert f"evidence from {name}" in text


def _with_watch(log_path, target, seconds=0.3):
    """Run ``target`` on the main thread while another thread plays the
    watch's own loop; returns whether it reported."""
    f = open(log_path, "a", encoding="utf-8")
    watch = black_box.StallWatch(f, seconds=seconds, idle_code=_idle.__code__)
    watch.beat = time.monotonic()
    reports = []

    def look():
        deadline = time.monotonic() + 3 * seconds + 0.5
        while time.monotonic() < deadline and not reports:
            if watch.check():
                reports.append(True)
            time.sleep(0.05)
    t = threading.Thread(target=look)
    t.start()
    target(3 * seconds + 0.6)
    t.join()
    f.close()
    return bool(reports)


def _idle(seconds):
    # Stands for the event loop's frame: waiting in C, no Python running.
    time.sleep(seconds)


def _busy_in_python(seconds):
    end = time.monotonic() + seconds
    n = 0
    while time.monotonic() < end:
        n += 1
    return n


def test_a_main_thread_busy_in_python_is_written_down(tmp_path):
    path = tmp_path / "crash.log"
    assert _with_watch(path, _busy_in_python)
    text = path.read_text()
    assert "main thread busy for" in text
    assert "_busy_in_python" in text          # the Python stack follows


def test_a_main_thread_waiting_in_the_event_loop_is_not(tmp_path):
    """A hidden window's timers can be held back for long (App Nap): with
    the main thread idle in the event loop there is nothing to report."""
    path = tmp_path / "crash.log"
    assert not _with_watch(path, _idle)
    assert not path.exists() or "main thread busy" not in path.read_text()


def test_one_report_per_stall_and_the_heartbeat_rearms(tmp_path):
    f = open(tmp_path / "crash.log", "a", encoding="utf-8")
    watch = black_box.StallWatch(f, seconds=0.0, idle_code=None)
    # Called from the main thread itself: its frame is busy by definition.
    assert watch.check()
    assert not watch.check()                  # same stall: once
    watch.tick()
    assert watch.check()                      # a new stall after a beat
    f.close()
