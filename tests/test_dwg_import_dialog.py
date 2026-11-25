# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #401: the DWG import runs the converter without freezing the
window, and says in the user's language why it failed, timed out or stopped."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from pathlib import Path  # noqa: E402

import pytest  # noqa: E402
from tests.shell_isolation import restore_shell_patches  # noqa: E402,F401

from formats import dwg_bridge  # noqa: E402

@pytest.fixture
def window(tmp_path):
    from ingetrazo_render import Host
    host = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
    yield host.window
    host.close()


@pytest.fixture
def dialogs(monkeypatch):
    shown = []
    monkeypatch.setattr("ingetrazo_render.chrome_dialogs.critical",
                        lambda parent, title, text: shown.append((title, text)))
    monkeypatch.setattr(dwg_bridge, "have_dwg_support", lambda: True)
    return shown


def _failing(kind, message):
    def convert(self, dwg_path):
        raise dwg_bridge.DwgBridgeError(message, kind)
    return convert


def test_a_failed_conversion_says_so_with_the_converter_output(
        window, dialogs, monkeypatch):
    monkeypatch.setattr(dwg_bridge.DwgConverter, "convert",
                        _failing("failed", "conversion produced no output: boom"))
    assert window._import_dwg_path(Path("x.dwg")) is False
    (title, text), = dialogs
    assert "boom" in text and "DXF" in text


def test_a_timeout_has_its_own_message(window, dialogs, monkeypatch):
    monkeypatch.setattr(dwg_bridge.DwgConverter, "convert",
                        _failing("timeout", "slow"))
    assert window._import_dwg_path(Path("x.dwg")) is False
    (_, text), = dialogs
    assert "too long" in text


def test_cancel_is_quiet_and_reported_in_the_status_bar(
        window, dialogs, monkeypatch):
    monkeypatch.setattr(dwg_bridge.DwgConverter, "convert",
                        _failing("cancelled", "stop"))
    assert window._import_dwg_path(Path("x.dwg")) is False
    assert dialogs == []
    assert "cancelled" in window.statusBar().currentMessage()


def test_the_import_job_offers_cancel_and_cancel_stops_the_converter(
        window, dialogs, monkeypatch):
    from ingetrazo_render import chrome_dialogs
    monkeypatch.setattr(chrome_dialogs, "live", lambda _: window.view)
    seen = {}

    def convert(self, dwg_path):
        from views import wgpu_ui
        from ingetrazo_render import chrome_dialogs
        job, = chrome_dialogs.jobs(window)
        seen["cancellable"] = job.cancellable
        wgpu_ui._run(window, [f"job:cancel:{job.id}"])
        raise dwg_bridge.DwgBridgeError("stopped", "cancelled")

    cancelled = []
    monkeypatch.setattr(dwg_bridge.DwgConverter, "convert", convert)
    monkeypatch.setattr(dwg_bridge.DwgConverter, "cancel",
                        lambda self: cancelled.append(True))
    window._import_dwg_path(Path("x.dwg"))
    assert seen["cancellable"]
    assert cancelled
