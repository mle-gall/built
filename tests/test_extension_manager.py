# SPDX-License-Identifier: GPL-3.0-or-later
from __future__ import annotations

import hashlib
import io
import json
import zipfile

import pytest
from dataclasses import replace
from types import SimpleNamespace


from ingetrazo_core import extension_manager as manager
from core.extensions import discover_plugins
from tests.extensions_chrome import Browser


TOOL_SOURCE = """
from tools.base import Tool

class CatalogTool(Tool):
    name = "Catalog tool"
    def on_activate(self, viewport):
        pass
    def on_deactivate(self, viewport):
        pass
"""


def _extension(payload: bytes, ident="sample") -> manager.Extension:
    return manager.Extension(
        id=ident,
        version="1.0",
        author="Author",
        license="MIT",
        repository="https://github.com/example/sample",
        download="https://example.org/sample.py",
        sha256=hashlib.sha256(payload).hexdigest(),
        minimum_version="0.1.0",
        name={"en": "Sample"},
        summary={"en": "Example extension"},
        tags=(),
    )


def test_catalog_validation_and_review_uses_the_reviewed_hash(
        tmp_path, monkeypatch):
    source = b"extension source"
    catalog = {
        "format": 1,
        "extensions": [{
            "id": "sample",
            "version": "1.0",
            "author": "Author",
            "license": "MIT",
            "repository": "https://github.com/example/sample",
            "download": "https://example.org/sample.py",
            "sha256": hashlib.sha256(source).hexdigest(),
            "ingetrazo": "0.5.0",
            "name": {"en": "Sample"},
            "summary": {"en": "Description"},
            "tags": [],
            "reviewed": True,
        }],
    }
    monkeypatch.setattr(manager, "catalog_cache_path",
                        lambda: tmp_path / "catalog-cache.json")
    monkeypatch.setattr(
        manager, "download",
        lambda url, _limit: (json.dumps(catalog).encode() if "catalog" in url
                             else b'[sample]\nsha256 = "'
                             + hashlib.sha256(source).hexdigest().encode()
                             + b'"\n'))
    snapshot = manager.refresh_catalog()
    extensions, reviewed = snapshot.extensions, snapshot.reviewed
    assert manager.is_reviewed(extensions[0], reviewed)
    reviewed["sample"]["sha256"] = "0" * 64
    assert not manager.is_reviewed(extensions[0], reviewed)


def test_catalog_is_cached_and_used_when_refresh_is_offline(
        tmp_path, monkeypatch):
    source = b"extension source"
    catalog = {
        "format": 1,
        "extensions": [{
            "id": "sample",
            "version": "1.0",
            "author": "Author",
            "license": "MIT",
            "repository": "https://github.com/example/sample",
            "download": "https://example.org/sample.py",
            "sha256": hashlib.sha256(source).hexdigest(),
            "ingetrazo": "0.5.0",
            "name": {"en": "Sample"},
            "summary": {"en": "Description"},
            "tags": [],
        }],
    }
    reviewed = b'[sample]\nsha256 = "' + hashlib.sha256(source).hexdigest().encode() + b'"\n'
    monkeypatch.setattr(manager, "catalog_cache_path",
                        lambda: tmp_path / "catalog-cache.json")
    monkeypatch.setattr(
        manager, "download",
        lambda url, _limit: (json.dumps(catalog).encode()
                             if "catalog" in url else reviewed))
    live = manager.refresh_catalog()
    assert not live.from_cache
    assert manager.load_cached_catalog().extensions[0].id == "sample"

    def offline(*_args):
        raise manager.ExtensionManagerError("offline")
    monkeypatch.setattr(manager, "download", offline)
    cached = manager.refresh_catalog()
    assert cached.from_cache
    assert cached.fetched_at == live.fetched_at
    assert cached.extensions[0].id == "sample"


def test_catalog_refresh_without_cache_surfaces_failure(tmp_path, monkeypatch):
    monkeypatch.setattr(manager, "catalog_cache_path",
                        lambda: tmp_path / "missing-cache.json")
    monkeypatch.setattr(manager, "download", lambda *_args: (_ for _ in ()).throw(
        manager.ExtensionManagerError("offline")))
    with pytest.raises(manager.ExtensionManagerError, match="offline"):
        manager.refresh_catalog()


def test_install_verify_disable_and_enable_extension(tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    extension = _extension(TOOL_SOURCE.encode())
    monkeypatch.setattr(manager, "download", lambda *_args: TOOL_SOURCE.encode())
    manager.install_extension(extension)
    target = tmp_path / "sample"
    assert (target / "__init__.py").read_text() == TOOL_SOURCE
    assert manager.installed_info("sample")["version"] == "1.0"
    assert manager.is_extension_enabled("sample")
    assert manager.restart_required("sample", {})
    assert [p.stem for p in discover_plugins([tmp_path])[0]] == ["sample"]

    manager.set_extension_enabled("sample", False)
    assert not manager.is_extension_enabled("sample")
    assert not manager.restart_required("sample", {})
    assert discover_plugins([tmp_path]) == ([], [])

    updated_source = TOOL_SOURCE + "\n# Updated package\n"
    monkeypatch.setattr(manager, "download", lambda *_args: updated_source.encode())
    manager.install_extension(_extension(updated_source.encode()))
    assert not manager.is_extension_enabled("sample")
    assert not manager.restart_required("sample", {})
    assert discover_plugins([tmp_path]) == ([], [])

    manager.set_extension_enabled("sample", True)
    assert manager.restart_required("sample", {})
    assert [p.stem for p in discover_plugins([tmp_path])[0]] == ["sample"]
    startup = manager.extension_startup_state()
    assert not manager.restart_required("sample", startup)
    monkeypatch.setattr(manager, "download",
                        lambda *_args: (updated_source + "# newer\n").encode())
    newer = (updated_source + "# newer\n").encode()
    manager.install_extension(_extension(newer))
    assert manager.restart_required("sample", startup)


def test_uninstall_removes_only_manager_owned_extension(tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    source = TOOL_SOURCE.encode()
    monkeypatch.setattr(manager, "download", lambda *_args: source)
    manager.install_extension(_extension(source))
    startup = manager.extension_startup_state()

    manager.uninstall_extension("sample")

    assert not (tmp_path / "sample").exists()
    assert manager.installed_info("sample") is None
    assert manager.restart_required("sample", startup)
    with pytest.raises(manager.ExtensionManagerError, match="not an installed"):
        manager.uninstall_extension("sample")


def test_restart_required_detects_disable_against_startup(tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    source = TOOL_SOURCE.encode()
    monkeypatch.setattr(manager, "download", lambda *_args: source)
    manager.install_extension(_extension(source))
    startup = manager.extension_startup_state()
    assert not manager.restart_required("sample", startup)
    manager.set_extension_enabled("sample", False)
    assert manager.restart_required("sample", startup)


def test_download_checksum_mismatch_never_creates_install(tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    extension = _extension(b"expected")
    monkeypatch.setattr(manager, "download", lambda *_args: b"tampered")
    with pytest.raises(manager.ExtensionManagerError, match="Checksum mismatch"):
        manager.install_extension(extension)
    assert not (tmp_path / "sample").exists()


def test_zip_package_is_extracted_and_traversal_is_rejected(
        tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w") as archive:
        archive.writestr("sample/__init__.py", TOOL_SOURCE)
        archive.writestr("sample/subpackage/__init__.py", "")
        archive.writestr("sample/helper.txt", "data")
    payload = output.getvalue()
    extension = _extension(payload)
    monkeypatch.setattr(manager, "download", lambda *_args: payload)
    manager.install_extension(extension)
    assert (tmp_path / "sample" / "__init__.py").read_text() == TOOL_SOURCE
    assert (tmp_path / "sample" / "subpackage" / "__init__.py").is_file()
    assert (tmp_path / "sample" / "helper.txt").read_text() == "data"

    bad = io.BytesIO()
    with zipfile.ZipFile(bad, "w") as archive:
        archive.writestr("../escape.py", "no")
        archive.writestr("sample.py", TOOL_SOURCE)
    malicious = bad.getvalue()
    monkeypatch.setattr(manager, "download", lambda *_args: malicious)
    with pytest.raises(manager.ExtensionManagerError, match="unsafe path"):
        manager.install_extension(_extension(malicious, "other"))
    assert not (tmp_path.parent / "escape.py").exists()


def test_manual_plugin_collision_is_not_overwritten(tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    (tmp_path / "sample.py").write_text("user code")
    extension = _extension(b"catalog code")
    monkeypatch.setattr(manager, "download", lambda *_args: b"catalog code")
    with pytest.raises(manager.ExtensionManagerError, match="not managed"):
        manager.install_extension(extension)
    assert (tmp_path / "sample.py").read_text() == "user code"


def test_minimum_version_comparison():
    extension = _extension(b"")
    assert manager.compatible_with_current(extension)
    assert manager.compatible_with_current(
        manager.Extension(**{**extension.__dict__,
                             "minimum_version": "99.0"})) is False


def test_incompatible_extension_is_rejected_before_download(
        tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    extension = manager.Extension(**{
        **_extension(b"future").__dict__,
        "minimum_version": "99.0",
    })
    called = False

    def unexpected_download(*_args):
        nonlocal called
        called = True
        return b"future"

    monkeypatch.setattr(manager, "download", unexpected_download)
    with pytest.raises(manager.ExtensionManagerError, match="requires Built"):
        manager.install_extension(extension)
    assert not called
    assert not (tmp_path / extension.id).exists()


def test_tested_version_age_is_a_nonblocking_warning():
    extension = _extension(b"")
    assert manager.tested_version_is_old(
        manager.Extension(**{**extension.__dict__,
                             "minimum_version": "0.0.0"}))


def test_extension_sort_modes_and_recent_date_availability(tmp_path, monkeypatch):
    alpha = replace(_extension(b"a", "alpha"), name={"en": "Alpha"}, updated_at="2026-01-01")
    beta = replace(_extension(b"b", "beta"), name={"en": "Beta"}, updated_at="2026-03-01")
    target = tmp_path / alpha.id
    target.mkdir()
    (target / "extension.json").write_text(json.dumps({"id": alpha.id, "sha256": "old"}))
    browser = Browser(monkeypatch, tmp_path, [alpha, beta], reviewed={beta.id: {"sha256": beta.sha256}})
    browser.act("tab", "Browse")
    for sort, ids in [("name", ["alpha", "beta"]), ("reviewed", ["beta", "alpha"]),
                      ("updates", ["alpha", "beta"]), ("recent", ["beta", "alpha"])]:
        assert browser.act("headmenu", "sort", sort).ids == ids
    no_dates = Browser(monkeypatch, tmp_path, [replace(alpha, updated_at=None)]).act("tab", "Browse")
    sort = next(m for m in no_dates.spec["menus"] if m["id"] == "sort")
    assert not next(item for item in sort["options"] if item["id"] == "recent")["enabled"]


def test_failed_extension_card_offers_recovery_actions(tmp_path, monkeypatch):
    from core import desktop_open, paths
    ext = _extension(TOOL_SOURCE.encode())
    folder = tmp_path / ext.id
    folder.mkdir()
    (folder / "extension.json").write_text(json.dumps({"id": ext.id, "sha256": ext.sha256}))
    win = SimpleNamespace(_extension_load_errors={ext.id: {"error": "ImportError: missing dependency",
                                                         "path": str(folder / "__init__.py")}})
    browser = Browser(monkeypatch, tmp_path, [ext], window=win)
    assert browser.facts["This session"] == "Failed to load at startup"
    assert browser.facts["Path"] == str(folder / "__init__.py")
    assert "ImportError: missing dependency" in browser.text and browser.buttons["toggle"] == "Disable"
    opened = []
    monkeypatch.setattr(desktop_open, "open_url", opened.append)
    browser.act("verb", "folder").act("verb", "logs")
    assert opened == [folder, paths.user_log_dir()]


def test_manager_shows_update_and_trust_details_without_downloading(tmp_path, monkeypatch):
    ext = replace(_extension(b"new"), tags=("drawing",))
    folder = tmp_path / ext.id
    folder.mkdir()
    (folder / "extension.json").write_text(json.dumps({"id": ext.id, "version": "0.9", "sha256": "old"}))
    browser = Browser(monkeypatch, tmp_path, [ext])
    assert "Update available: v0.9 → v1.0" in browser.text
    assert browser.facts["License"] == "MIT" and browser.facts["Tested with Built"] == ext.minimum_version
    assert browser.facts["Author"] == ext.author and browser.facts["Review"] == "Not reviewed by a Built maintainer"
    assert browser.buttons["install"] == "Update" and browser.buttons["repository"] == "Repository ↗"


def test_incompatible_extension_is_not_offered_for_install_or_enable(tmp_path, monkeypatch):
    ext = replace(_extension(b"future"), minimum_version="99.0")
    browser = Browser(monkeypatch, tmp_path, [ext]).act("tab", "Browse")
    assert "Requires Built 99.0 or later." in browser.text
    assert "install" not in browser.buttons and "toggle" not in browser.buttons
    browser.act("verb", "install")
    assert manager.installed_info(ext.id) is None


def test_unexpected_catalog_refresh_error_keeps_cached_catalog_visible(tmp_path, monkeypatch):
    from threading import Event
    failed = Event()

    def fail():
        failed.set()
        raise RuntimeError("unexpected parser failure")
    browser = Browser(monkeypatch, tmp_path, [_extension(b"x")])
    monkeypatch.setattr(manager, "refresh_catalog", fail)
    browser.act("tab", "Browse")
    assert failed.wait(5)
    if browser.provider.refreshing:
        with pytest.raises(RuntimeError):
            browser.provider.refreshing.result(timeout=5)
    browser.read()
    assert browser.ids == ["sample"]
    assert "unexpected parser failure" in browser.spec["footer"]["hint"]
    assert browser.provider.refreshing is None


def test_extension_manager_search_tags_and_reviewed_filter(tmp_path, monkeypatch):
    first = replace(_extension(b"one", "loop_cut"), tags=("drawing", "productivity"))
    second = replace(_extension(b"two", "terrain_tool"), name={"en": "Terrain Tool"},
                     summary={"en": "Terrain editor"}, tags=("terrain",))
    browser = Browser(monkeypatch, tmp_path, [first, second], reviewed={first.id: {"sha256": first.sha256}})
    browser.act("tab", "Browse")
    assert browser.spec["content"]["sub"] == "2 of 2 extensions"
    assert browser.act("search", "editor").ids == [second.id]
    assert browser.act("search", "").act("headmenu", "tag", "terrain").ids == [second.id]
    assert browser.act("headmenu", "tag", "").act("headmenu", "trust", "reviewed").ids == [first.id]


def test_installed_card_shows_restart_status(tmp_path, monkeypatch):
    ext = _extension(b"source")
    folder = tmp_path / ext.id
    folder.mkdir()
    (folder / "extension.json").write_text(json.dumps({"id": ext.id, "version": ext.version, "sha256": ext.sha256}))
    browser = Browser(monkeypatch, tmp_path, [ext], window=SimpleNamespace(_extension_startup_state={}))
    assert "Restart required to apply changes" in browser.text
    assert browser.facts["Next launch"] == "Enabled" and "install" not in browser.buttons


def test_catalog_text_is_shown_as_plain_text(tmp_path, monkeypatch):
    summary = "<b style='color:green'>REVIEWED</b> trust me"
    ext = replace(_extension(b"html"), summary={"en": summary}, repository="file:///etc/passwd")
    browser = Browser(monkeypatch, tmp_path, [ext]).act("tab", "Browse")
    assert {"kind": "text", "text": summary} in browser.blocks
    assert browser.facts["Review"] == "Not reviewed by a Built maintainer"
    assert "repository" not in browser.buttons and browser.provider.model.command("repository", str) is None


def test_browser_builds_in_the_extensions_sheet(tmp_path, monkeypatch):
    browser = Browser(monkeypatch, tmp_path, [_extension(b"x")]).act("tab", "Browse")
    assert browser.spec["id"] == "extensions" and browser.ids == ["sample"]


def test_zip_names_with_a_colon_are_rejected(tmp_path, monkeypatch):
    # On Windows «C:x» is drive-relative and «a.py:s» a data stream: both
    # can write outside the stage. Linux would extract them quietly.
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    for name in ("sample/C:escape.py", "sample/helper.py:stream"):
        bad = io.BytesIO()
        with zipfile.ZipFile(bad, "w") as archive:
            archive.writestr("sample/__init__.py", TOOL_SOURCE)
            archive.writestr(name, "no")
        payload = bad.getvalue()
        monkeypatch.setattr(manager, "download", lambda *_a, p=payload: p)
        with pytest.raises(manager.ExtensionManagerError, match="unsafe path"):
            manager.install_extension(_extension(payload))
        assert not (tmp_path / "sample").exists()


def _zip(files: dict) -> bytes:
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w") as archive:
        for name, text in files.items():
            archive.writestr(name, text)
    return out.getvalue()


def test_a_hand_installed_copy_under_another_name_is_set_aside(
        tmp_path, monkeypatch):
    # OpenTrace used to be unpacked by hand as «arquitetura_parametrica»;
    # installed from the catalog as «opentrace_bim» it loaded twice.
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    old = tmp_path / "arquitetura_parametrica"
    (old / "nested").mkdir(parents=True)
    for name in ("beam.py", "column.py", "slab.py", "nested/roof.py"):
        (old / name).write_text("# old version\n")
    (old / "__init__.py").write_text("# old entry\n")
    other = tmp_path / "unrelated"
    other.mkdir()
    (other / "__init__.py").write_text(TOOL_SOURCE)
    (other / "a.py").write_text("x = 1\n")
    payload = _zip({f"opentrace_bim/{n}": "# new\n" for n in (
        "__init__.py", "beam.py", "column.py", "slab.py", "nested/roof.py",
        "walls.py")})
    monkeypatch.setattr(manager, "download", lambda *_a: payload)
    extension = _extension(payload, "opentrace_bim")

    asked = []
    with pytest.raises(manager.ExtensionManagerError, match="by hand"):
        manager.install_extension(
            extension, confirm_manual_copies=lambda p: asked.append(p) or False)
    assert [p.name for p in asked[0]] == ["arquitetura_parametrica"]
    assert old.is_dir() and not (tmp_path / "opentrace_bim").exists()

    manager.install_extension(extension, confirm_manual_copies=lambda p: True)
    assert (tmp_path / "opentrace_bim" / "__init__.py").is_file()
    assert not old.exists() and other.is_dir()           # only the copy goes
    aside = [p for p in tmp_path.iterdir()
             if p.name.startswith(".arquitetura_parametrica.manual-")]
    assert len(aside) == 1 and (aside[0] / "beam.py").is_file()
    stems = [plug.stem for plug in discover_plugins([tmp_path])[0]]
    assert "arquitetura_parametrica" not in stems


def test_an_identical_loose_file_counts_as_a_copy(tmp_path, monkeypatch):
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    # newline="\n": byte for byte what the catalog serves (on Windows
    # write_text puts \r\n and the copy is no longer identical).
    (tmp_path / "my_old_name.py").write_text(TOOL_SOURCE, newline="\n")
    payload = TOOL_SOURCE.encode()
    monkeypatch.setattr(manager, "download", lambda *_a: payload)
    with pytest.raises(manager.ExtensionManagerError, match="my_old_name.py"):
        manager.install_extension(_extension(payload))
    assert not (tmp_path / "sample").exists()


def test_a_loose_file_with_crlf_line_endings_counts_as_a_copy(
        tmp_path, monkeypatch):
    # #764: a hand-installed copy whose line endings were converted to
    # CRLF (a Windows editor, a zip tool, Path.write_text on Windows)
    # is the same extension, even though its raw bytes differ.
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    crlf = TOOL_SOURCE.encode().replace(b"\n", b"\r\n")
    (tmp_path / "my_old_name.py").write_bytes(crlf)
    payload = TOOL_SOURCE.encode()
    monkeypatch.setattr(manager, "download", lambda *_a: payload)
    with pytest.raises(manager.ExtensionManagerError, match="my_old_name.py"):
        manager.install_extension(_extension(payload))
    assert not (tmp_path / "sample").exists()


def test_a_loose_file_with_a_bom_and_crlf_counts_as_a_copy(
        tmp_path, monkeypatch):
    # #764: a UTF-8 BOM saved by a Windows editor must not hide a copy
    # either.
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    crlf = TOOL_SOURCE.encode().replace(b"\n", b"\r\n")
    (tmp_path / "my_old_name.py").write_bytes(b"\xef\xbb\xbf" + crlf)
    payload = TOOL_SOURCE.encode()
    monkeypatch.setattr(manager, "download", lambda *_a: payload)
    with pytest.raises(manager.ExtensionManagerError, match="my_old_name.py"):
        manager.install_extension(_extension(payload))
    assert not (tmp_path / "sample").exists()


def test_a_package_whose_entry_has_crlf_line_endings_counts_as_a_copy(
        tmp_path, monkeypatch):
    # #764, the package form: only the entry module's endings changed.
    plugins = tmp_path / "plugins"
    plugins.mkdir()
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: plugins)
    hand = plugins / "hand_copy"
    hand.mkdir()
    (hand / "__init__.py").write_bytes(
        TOOL_SOURCE.encode().replace(b"\n", b"\r\n"))
    package = tmp_path / "catalog_copy"
    package.mkdir()
    (package / "__init__.py").write_bytes(TOOL_SOURCE.encode())
    assert manager.manual_copies("sample", package) == [hand]


def test_a_different_loose_file_with_crlf_is_not_a_copy(
        tmp_path, monkeypatch):
    # Normalizing endings must not collapse different code into a copy:
    # a file that differs beyond its line endings stays unrelated.
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: tmp_path)
    other = TOOL_SOURCE.replace("Catalog tool", "Another tool")
    (tmp_path / "my_old_name.py").write_bytes(
        other.encode().replace(b"\n", b"\r\n"))
    payload = TOOL_SOURCE.encode()
    monkeypatch.setattr(manager, "download", lambda *_a: payload)
    manager.install_extension(_extension(payload))
    assert (tmp_path / "sample" / "__init__.py").is_file()
    assert (tmp_path / "my_old_name.py").is_file()


_SHA_A = "0123456789abcdef"
_SHA_B = "fedcba9876543210"


def test_a_bad_thumb_or_print_is_ignored_and_the_extension_kept():
    base = {
        "id": "sample", "version": "1.0", "author": "A", "license": "MIT",
        "repository": "https://github.com/example/sample",
        "download": "https://example.org/sample.py", "sha256": "a" * 64,
        "ingetrazo": "0.1.0", "name": {"en": "S"}, "summary": {"en": "S"},
        "screenshot": "sample.png"}
    good = manager.parse_extension(
        {**base, "thumb": "sample.webp", "screenshot_sha": _SHA_A})
    assert (good.thumb, good.screenshot_sha) == ("sample.webp", _SHA_A)
    for thumb, sha in (("../sample.webp", "0123456789ABCDEF"),
                       ("a/b.webp", "0123"), ("a\\b.webp", 12345),
                       ("sample.exe", "g" * 16), (7, None), ("", "")):
        parsed = manager.parse_extension(
            {**base, "thumb": thumb, "screenshot_sha": sha})
        assert parsed.id == "sample" and parsed.screenshot == "sample.png"
        assert parsed.thumb is None and parsed.screenshot_sha is None
    old = manager.parse_extension(base)              # an older catalog
    assert old.thumb is None and old.screenshot_sha is None
