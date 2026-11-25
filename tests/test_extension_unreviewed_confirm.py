# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Installing an extension nobody reviewed asks first, with No as the
default: it is its author's Python code, run with the user's permissions.
The card already said «not reviewed»; one click was enough to install it."""
from __future__ import annotations

from dataclasses import replace

from built_core import extension_manager as manager
from tests.extensions_chrome import Browser
from tests.test_extension_manager import _extension

SOURCE = b"def setup(app):\n    pass\n"


def _open(monkeypatch, root, reviewed=False):
    ext = _extension(SOURCE)
    browser = Browser(monkeypatch, root, [ext], reviewed={"sample": {"sha256": ext.sha256}} if reviewed else {})
    monkeypatch.setattr(manager, "download", lambda *_: SOURCE)
    return browser.act("tab", "Browse")


def test_an_unreviewed_extension_asks_and_no_installs_nothing(tmp_path, monkeypatch):
    browser = _open(monkeypatch, tmp_path)
    asked = browser.confirm(monkeypatch, "cancel")
    browser.act("verb", "install")
    (title, text, default), = asked
    assert "unreviewed" in title and "Author" in text and default == "cancel"
    assert manager.installed_info("sample") is None


def test_an_unreviewed_extension_installs_once_the_user_says_yes(tmp_path, monkeypatch):
    browser = _open(monkeypatch, tmp_path)
    asked = browser.confirm(monkeypatch)
    browser.act("verb", "install")
    assert len(asked) == 1 and manager.installed_info("sample")["sha256"] == _extension(SOURCE).sha256


def test_a_reviewed_extension_installs_without_asking(tmp_path, monkeypatch):
    browser = _open(monkeypatch, tmp_path, reviewed=True)
    asked = browser.confirm(monkeypatch, "cancel")
    browser.act("verb", "install")
    assert not asked and manager.installed_info("sample") is not None


def test_a_bundled_extension_installs_without_asking(tmp_path, monkeypatch):
    source = tmp_path / "local.py"
    source.write_bytes(SOURCE)
    ext = replace(_extension(SOURCE), bundled=str(source))
    browser = Browser(monkeypatch, tmp_path / "plugins", bundled=[ext]).act("tab", "Browse")
    asked = browser.confirm(monkeypatch, "cancel")
    browser.act("verb", "install")
    assert not asked and manager.installed_info("sample") is not None
