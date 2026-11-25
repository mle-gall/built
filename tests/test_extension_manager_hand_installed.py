# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Extensions installed by hand, as every one was before the manager (0.5.8).

A user who copied «planarq» into the plugins folder under 0.5.7 opened the
0.5.8 manager and was stuck: its catalog card said «A plugin with this ID
already exists» with no way to uninstall, disable or update it, and a plugin
the catalog does not know did not show at all. Now both can be uninstalled
from the manager — set aside, renamed with a dot in front, never deleted —
and a catalog one can be replaced by the catalog version.
"""
from __future__ import annotations

import pytest

from ingetrazo_core import extension_manager as manager
from core.extensions import discover_plugins
from tests.test_extension_manager import TOOL_SOURCE, _extension
from tests.extensions_chrome import Browser


def _hand_package(root, name, source=TOOL_SOURCE):
    folder = root / name
    folder.mkdir()
    (folder / "__init__.py").write_text(source)
    return folder


def _loaded(root):
    return sorted(p.stem for p in discover_plugins([root])[0])


def test_hand_installed_plugins_are_the_ones_the_manager_did_not_install(
        tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    _hand_package(tmp_path, "aberturas")
    (tmp_path / "niveles.py").write_text(TOOL_SOURCE)
    managed = _hand_package(tmp_path, "kitchen")
    (managed / "extension.json").write_text('{"id": "kitchen"}')
    (tmp_path / "__pycache__").mkdir()
    (tmp_path / "notes").mkdir()                       # no __init__.py
    _hand_package(tmp_path, ".planarq.manual-20261001-120000")
    names = [manager.hand_installed_ident(p)
             for p in manager.hand_installed_plugins()]
    assert names == ["aberturas", "niveles"]


def test_uninstalling_a_hand_copy_sets_it_aside_and_keeps_its_files(
        tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    folder = _hand_package(tmp_path, "aberturas")
    assert _loaded(tmp_path) == ["aberturas"]

    aside = manager.remove_hand_installed(folder)

    assert not folder.exists()
    assert aside.name.startswith(".aberturas.manual-")
    assert (aside / "__init__.py").read_text() == TOOL_SOURCE   # kept
    assert _loaded(tmp_path) == []                   # no longer loaded
    assert manager.hand_installed_plugins() == []


def test_a_managed_or_outside_folder_is_never_set_aside_as_hand_installed(
        tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    managed = _hand_package(tmp_path, "kitchen")
    (managed / "extension.json").write_text('{"id": "kitchen"}')
    outside = tmp_path.parent / "elsewhere.py"
    outside.write_text("x")
    for path in (managed, outside):
        with pytest.raises(manager.ExtensionManagerError):
            manager.remove_hand_installed(path)
    assert managed.exists() and outside.exists()


def test_a_hand_copy_is_replaced_by_the_catalog_version(tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    (tmp_path / "sample.py").write_text("# the 0.5.7 copy\n")
    source = TOOL_SOURCE.encode()
    monkeypatch.setattr(manager, "download", lambda *_args: source)

    moved = manager.replace_hand_installed(_extension(source))

    assert [p.name.startswith(".sample.py.manual-") for p in moved] == [True]
    assert moved[0].read_text() == "# the 0.5.7 copy\n"
    assert manager.installed_info("sample")["version"] == "1.0"
    assert _loaded(tmp_path) == ["sample"]
    manager.uninstall_extension("sample")            # managed from now on
    assert not (tmp_path / "sample").exists()


def test_a_failed_replacement_puts_the_hand_copy_back(tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    folder = _hand_package(tmp_path, "sample", "# mine\n")
    monkeypatch.setattr(manager, "download", lambda *_args: b"tampered")
    with pytest.raises(manager.ExtensionManagerError, match="Checksum"):
        manager.replace_hand_installed(_extension(b"expected"))
    assert (folder / "__init__.py").read_text() == "# mine\n"
    assert [p.name for p in tmp_path.iterdir()] == ["sample"]


def test_a_catalog_extension_installed_by_hand_offers_replace_and_uninstall(tmp_path, monkeypatch):
    path = tmp_path / "sample.py"
    path.write_text("# the 0.5.7 copy")
    ext = _extension(TOOL_SOURCE.encode())
    browser = Browser(monkeypatch, tmp_path, [ext])
    assert browser.ids == [ext.id]
    assert browser.buttons["replace"] == "Replace with the catalog version"
    assert {"remove", "folder"} <= browser.buttons.keys() and "install" not in browser.buttons
    assert browser.facts["Source"] == "Installed by hand"
    questions = browser.confirm(monkeypatch)
    monkeypatch.setattr(manager, "download", lambda *_: b"corrupt")
    browser.act("verb", "replace")
    assert "Checksum mismatch" in browser.spec["footer"]["hint"]
    assert path.read_text() == "# the 0.5.7 copy"
    monkeypatch.setattr(manager, "download", lambda *_: TOOL_SOURCE.encode())
    browser.act("verb", "replace")
    assert manager.installed_info(ext.id)["sha256"] == ext.sha256
    assert [q[0] for q in questions] == ["Install an unreviewed extension?", "Replace extension"] * 2
    assert next(tmp_path.glob(".sample.py.manual-*")).read_text() == "# the 0.5.7 copy"


def test_a_hand_plugin_the_catalog_does_not_know_can_be_uninstalled(tmp_path, monkeypatch):
    folder = _hand_package(tmp_path, "aberturas")
    browser = Browser(monkeypatch, tmp_path, [_extension(b"catalog")])
    assert browser.ids == ["aberturas"] and browser.facts["Path"] == str(folder)
    browser.confirm(monkeypatch)
    browser.act("verb", "remove")
    assert not folder.exists() and browser.ids == []
    assert next(tmp_path.glob(".aberturas.manual-*")).joinpath("__init__.py").read_text() == TOOL_SOURCE


def test_an_extension_shipped_with_the_app_installs_from_the_manager(tmp_path, monkeypatch):
    from dataclasses import replace
    source = tmp_path / "source.py"
    source.write_text(TOOL_SOURCE)
    bundled = replace(_extension(source.read_bytes(), "windowizer"), bundled=str(source))
    twin = replace(bundled, id="niveles")
    catalog = [_extension(b"catalog", "niveles")]
    browser = Browser(monkeypatch, tmp_path / "plugins", catalog, bundled=[bundled, twin])
    browser.act("tab", "Browse").act("focus", "windowizer")
    assert sorted(browser.ids) == ["niveles", "windowizer"]
    assert browser.facts["Source"] == "Included with Built"
    assert browser.facts["Review"] == "Reviewed" and "install" in browser.buttons
    browser.act("verb", "install")
    assert manager.installed_info("windowizer")["version"] == bundled.version
    assert (tmp_path / "plugins/windowizer/__init__.py").read_text() == TOOL_SOURCE


def test_an_included_extension_shows_its_own_preview_and_a_translated_line(
        tmp_path, monkeypatch):
    from core import paths
    from core.i18n import current_language
    root = tmp_path / "app"
    examples = root / "examples" / "extensions"
    examples.mkdir(parents=True)
    # encoding: the «—» is cp1252 on Windows otherwise, and the manager
    # reads extensions as UTF-8 like Python does.
    (examples / "demo.py").write_text(
        '"""Demo — parametric windows from selected faces.\n\nHow to."""\n',
        encoding="utf-8")
    (examples / "demo.png").write_bytes(b"\x89PNG\r\n\x1a\n")
    monkeypatch.setattr(paths, "app_root", lambda: root)
    demo, = manager.bundled_extensions()
    assert demo.screenshot == str(examples / "demo.png")
    assert demo.summary["en"] == "Parametric windows from selected faces."
    lang = current_language().split("-")[0]
    assert lang in demo.summary            # what the card shows, via tr()


def test_ids_supplied_to_management_actions_stay_inside_the_plugin_root(tmp_path, monkeypatch):
    plugins = tmp_path / "plugins"
    plugins.mkdir()
    outside = tmp_path / "old"
    outside.mkdir()
    (outside / "extension.json").write_text('{"id":"../old"}')
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: plugins)
    with pytest.raises(manager.ExtensionManagerError, match="ID"):
        manager.uninstall_extension("../old")
    assert (outside / "extension.json").is_file()


def test_a_second_install_cannot_change_the_folder_inside_confirmation(tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    monkeypatch.setattr(manager, "download", lambda *_: b"x = 1\n")
    (tmp_path / "local_copy.py").write_bytes(b"x = 1\n")
    ext = _extension(b"x = 1\n")

    def confirm(copies):
        assert copies == [tmp_path / "local_copy.py"]
        with pytest.raises(manager.ExtensionManagerError, match="another extension operation"):
            manager.install_extension(ext)
        return True

    manager.install_extension(ext, confirm)
    assert manager.installed_info(ext.id)["sha256"] == ext.sha256
