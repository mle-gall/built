# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The notice of a newer version: what counts as newer, when it looks, what
the window does. No test here touches the network."""
from __future__ import annotations

import time

import pytest

from core import timers
from core import update_check as uc
from ingetrazo_render import chrome_sheet
from views import release_check as ud
from ingetrazo_render import chrome_dialogs


class Store:
    """QSettings' value/setValue, without the user's settings."""

    def __init__(self, **values):
        self.values = {k.replace("__", "/"): v for k, v in values.items()}

    def value(self, key, default=None):
        return self.values.get(key, default)

    def setValue(self, key, value):
        self.values[key] = value


@pytest.fixture
def settings(monkeypatch):
    store = Store()
    monkeypatch.setattr(ud, "QSettings", lambda: store)
    monkeypatch.setattr(uc, "should_check_at_start", lambda *a, **kw: True)
    return store


def _wait(ms=120, until=None):
    """Run the window's timers *ms*; with *until*, until it holds (5 s at
    most): a thread's answer comes when it comes."""
    end = time.monotonic() + (5.0 if until is not None else ms / 1000)
    while time.monotonic() < end:
        timers.run_due()
        if until is not None and until():
            return
        time.sleep(0.01)


class _Window:
    """A window as the check sees one: its own attributes, no host (its
    timers run on core.timers' process-wide queue)."""

    def __init__(self):
        self._tool_actions, self._documents = {}, []


#: Windows stay alive to the end: a thread's answer may still be on its way.
_WINDOWS = []


def _window():
    win = _Window()
    _WINDOWS.append(win)
    return win


RELEASE = uc.Release("0.6.2", "2026-11-17", "## Fixed\n- **a thing**",
                     uc.RELEASES_PAGE + "/tag/v0.6.2")


@pytest.mark.parametrize("remote,local,newer", [
    ("0.6.1", "0.6.0", True),
    ("v0.6.1", "0.6.0", True),
    ("0.6.10", "0.6.9", True),      # as numbers, not as text
    ("0.7", "0.6.9", True),
    ("0.6.0", "0.6.0", False),
    ("0.6", "0.6.0", False),
    ("0.5.12", "0.6.0", False),
    ("garbage", "0.6.0", False),
    ("0.6.1-rc1", "0.6.0", False),  # a pre-release is not an update
])
def test_is_newer(remote, local, newer):
    assert uc.is_newer(remote, local) is newer


def test_parse_release_reads_the_answer():
    r = uc.parse_release({
        "tag_name": "v0.6.1", "published_at": "2026-11-10T13:17:01Z",
        "body": "notes", "html_url": uc.RELEASES_PAGE + "/tag/v0.6.1"})
    assert r == uc.Release("0.6.1", "2026-11-10", "notes",
                           uc.RELEASES_PAGE + "/tag/v0.6.1")


def test_download_never_goes_outside_the_project():
    r = uc.parse_release({"tag_name": "v0.6.1",
                          "html_url": "https://example.com/evil"})
    assert r.url == uc.RELEASES_PAGE + "/latest"


@pytest.mark.parametrize("payload", [{}, {"tag_name": "latest"}, ["x"]])
def test_a_strange_answer_is_an_error(payload):
    with pytest.raises(uc.UpdateCheckError):
        uc.parse_release(payload)


NOW = 1_800_000_000.0


@pytest.mark.parametrize("store,frozen,env,looks", [
    ({}, True, {}, True),
    ({}, False, {}, False),                                  # from source
    ({}, True, {"FLATPAK_ID": "x"}, False),
    ({}, True, {"SNAP": "/snap/x"}, False),
    ({"updates__check_at_start": "0"}, True, {}, False),     # Preferences
    ({"updates__last_check": NOW - 3600}, True, {}, False),  # today already
    ({"updates__last_check": NOW - 25 * 3600}, True, {}, True),
    ({"updates__last_check": NOW + 9e6}, True, {}, True),    # clock went back
    ({"updates__last_check": "bad"}, True, {}, True),
])
def test_when_it_looks_at_start(store, frozen, env, looks):
    assert uc.should_check_at_start(Store(**store), now=NOW, frozen=frozen,
                                    environ=env) is looks


def test_a_newer_version_opens_the_window_without_blocking(settings):
    win = _window()
    ud.start_check(win, True, lambda: RELEASE, "0.6.1")
    _wait(until=lambda: chrome_dialogs.history(win))
    assert not chrome_dialogs.modal(win) and not chrome_sheet.shown(win)
    chrome_dialogs.history(win)[0].action()
    sheet = chrome_sheet.current(win).build(None)
    spans = [span for block in sheet.spec["content"]["blocks"] for span in block.get("spans", [])]
    assert any(span["text"] == "a thing" and span["bold"] for span in spans)
    assert uc.KEY_LAST in settings.values


@pytest.mark.parametrize("current", ["0.6.2", "0.6.3"])
def test_quiet_and_up_to_date_says_nothing(settings, current):
    win = _window()
    ud.start_check(win, True, lambda: RELEASE, current)
    _wait(until=lambda: uc.KEY_LAST in settings.values)
    assert uc.KEY_LAST in settings.values
    assert not chrome_sheet.shown(win) and not chrome_dialogs.history(win)


def offline():
    raise uc.UpdateCheckError("offline")


def test_quiet_and_offline_says_nothing_and_tries_again(settings):
    win = _window()
    ud.start_check(win, True, offline, "0.6.1")
    _wait()
    assert not chrome_sheet.shown(win) and not chrome_dialogs.history(win)
    assert uc.KEY_LAST not in settings.values


def test_a_skipped_version_stays_quiet_but_the_menu_still_shows_it(settings):
    win = _window()
    uc.skip_version(settings, RELEASE.version)
    ud.start_check(win, True, lambda: RELEASE, "0.6.1")
    _wait(until=lambda: uc.KEY_LAST in settings.values)
    assert not chrome_dialogs.history(win) and not chrome_sheet.shown(win)
    ud.start_check(win, False, lambda: RELEASE, "0.6.1")
    _wait(until=lambda: chrome_sheet.shown(win))
    assert chrome_sheet.current(win).id == "update"


def test_the_menu_says_when_up_to_date_and_when_offline(settings):
    win = _window()
    ud.start_check(win, False, lambda: RELEASE, "0.6.2")
    _wait(until=lambda: chrome_dialogs.history(win))
    row = chrome_dialogs.history(win)[0]
    assert row.tone == "info" and "0.6.2" in row.detail
    ud.start_check(win, False, offline, "0.6.2")
    _wait(until=lambda: len(chrome_dialogs.history(win)) == 2)
    assert chrome_dialogs.history(win)[0].tone == "warning"


def test_skip_and_download_buttons(settings, monkeypatch):
    from types import SimpleNamespace
    opened = []
    monkeypatch.setattr(ud.desktop_open, "open_url", opened.append)
    win = _window()
    for action in ("later", "download", "skip"):
        ud.start_check(win, False, lambda: RELEASE, "0.6.1")
        _wait(until=lambda: chrome_sheet.shown(win))
        chrome_sheet.current(win).act(SimpleNamespace(win=win), "verb", [action])
        assert not chrome_sheet.shown(win)
        assert uc.is_skipped(settings, RELEASE.version) == (action == "skip")
    assert opened == [RELEASE.url]


def test_a_quiet_notice_waits_for_a_modal_question(settings, monkeypatch):
    win = _window()
    monkeypatch.setattr(ud, "_RETRY_MS", 30)
    chrome_dialogs.engage(win)                  # a question holds the window
    ud.start_check(win, True, lambda: RELEASE, "0.6.1")
    _wait(until=lambda: uc.KEY_LAST in settings.values)
    _wait(100)
    assert not chrome_dialogs.history(win)
    chrome_dialogs.release(win)
    _wait(until=lambda: chrome_dialogs.history(win))
    assert chrome_dialogs.history(win)[0].action


def test_start_check_respects_should_check(settings, monkeypatch):
    monkeypatch.setattr(uc, "should_check_at_start", lambda *a, **kw: False)
    called = []
    ud.start_check(_window(), True, fetch=lambda: called.append(1))
    _wait(40)
    assert not called
