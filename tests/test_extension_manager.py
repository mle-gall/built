# SPDX-License-Identifier: GPL-3.0-or-later
from __future__ import annotations

import hashlib
import io
import json
import zipfile

import pytest
from types import SimpleNamespace
from tempfile import NamedTemporaryFile, TemporaryDirectory


from built_core import extension_manager as manager
from built_core.extensions import discover_plugins
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


_sources = TemporaryDirectory()


def _extension(payload: bytes, ident="sample") -> manager.Extension:
    source = NamedTemporaryFile(suffix=".zip" if zipfile.is_zipfile(io.BytesIO(payload)) else ".py",
                                dir=_sources.name, delete=False)
    source.write(payload)
    source.close()
    return manager.Extension(ident, "1.0", hashlib.sha256(payload).hexdigest(),
                             "0.1.0", {"en": "Sample"}, {"en": "Example extension"}, source.name)


def test_install_verify_disable_and_enable_extension(tmp_path, monkeypatch):
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    extension = _extension(TOOL_SOURCE.encode())
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
    manager.install_extension(_extension(updated_source.encode()))
    assert not manager.is_extension_enabled("sample")
    assert not manager.restart_required("sample", {})
    assert discover_plugins([tmp_path]) == ([], [])

    manager.set_extension_enabled("sample", True)
    assert manager.restart_required("sample", {})
    assert [p.stem for p in discover_plugins([tmp_path])[0]] == ["sample"]
    startup = manager.extension_startup_state()
    assert not manager.restart_required("sample", startup)
    newer = (updated_source + "# newer\n").encode()
    manager.install_extension(_extension(newer))
    assert manager.restart_required("sample", startup)


def test_uninstall_removes_only_manager_owned_extension(tmp_path, monkeypatch):
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    source = TOOL_SOURCE.encode()
    manager.install_extension(_extension(source))
    startup = manager.extension_startup_state()

    manager.uninstall_extension("sample")

    assert not (tmp_path / "sample").exists()
    assert manager.installed_info("sample") is None
    assert manager.restart_required("sample", startup)
    with pytest.raises(manager.ExtensionManagerError, match="not an installed"):
        manager.uninstall_extension("sample")


def test_restart_required_detects_disable_against_startup(tmp_path, monkeypatch):
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    source = TOOL_SOURCE.encode()
    manager.install_extension(_extension(source))
    startup = manager.extension_startup_state()
    assert not manager.restart_required("sample", startup)
    manager.set_extension_enabled("sample", False)
    assert manager.restart_required("sample", startup)


def test_local_checksum_mismatch_never_creates_install(tmp_path, monkeypatch):
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    extension = _extension(b"expected")
    from pathlib import Path
    Path(extension.bundled).write_bytes(b"tampered")
    with pytest.raises(manager.ExtensionManagerError, match="Checksum mismatch"):
        manager.install_extension(extension)
    assert not (tmp_path / "sample").exists()


def test_zip_package_is_extracted_and_traversal_is_rejected(
        tmp_path, monkeypatch):
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w") as archive:
        archive.writestr("sample/__init__.py", TOOL_SOURCE)
        archive.writestr("sample/subpackage/__init__.py", "")
        archive.writestr("sample/helper.txt", "data")
    payload = output.getvalue()
    extension = _extension(payload)
    manager.install_extension(extension)
    assert (tmp_path / "sample" / "__init__.py").read_text() == TOOL_SOURCE
    assert (tmp_path / "sample" / "subpackage" / "__init__.py").is_file()
    assert (tmp_path / "sample" / "helper.txt").read_text() == "data"

    bad = io.BytesIO()
    with zipfile.ZipFile(bad, "w") as archive:
        archive.writestr("../escape.py", "no")
        archive.writestr("sample.py", TOOL_SOURCE)
    malicious = bad.getvalue()
    with pytest.raises(manager.ExtensionManagerError, match="unsafe path"):
        manager.install_extension(_extension(malicious, "other"))
    assert not (tmp_path.parent / "escape.py").exists()


def test_manual_plugin_collision_is_not_overwritten(tmp_path, monkeypatch):
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    (tmp_path / "sample.py").write_text("user code")
    extension = _extension(b"catalog code")
    with pytest.raises(manager.ExtensionManagerError, match="not managed"):
        manager.install_extension(extension)
    assert (tmp_path / "sample.py").read_text() == "user code"


def test_incompatible_extension_is_rejected_before_installation(
        tmp_path, monkeypatch):
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    extension = manager.Extension(**{
        **_extension(b"future").__dict__,
        "minimum_version": "99.0",
    })
    with pytest.raises(manager.ExtensionManagerError, match="requires Built"):
        manager.install_extension(extension)
    assert not (tmp_path / extension.id).exists()


def test_failed_extension_card_offers_recovery_actions(tmp_path, monkeypatch):
    from built_core import paths
    from built_core import desktop_open
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


def test_installed_card_shows_restart_status(tmp_path, monkeypatch):
    ext = _extension(b"source")
    folder = tmp_path / ext.id
    folder.mkdir()
    (folder / "extension.json").write_text(json.dumps({"id": ext.id, "version": ext.version, "sha256": ext.sha256}))
    browser = Browser(monkeypatch, tmp_path, [ext], window=SimpleNamespace(_extension_startup_state={}))
    assert "Restart required to apply changes" in browser.text
    assert browser.facts["Next launch"] == "Enabled" and "install" not in browser.buttons


def test_browser_builds_in_the_extensions_sheet(tmp_path, monkeypatch):
    browser = Browser(monkeypatch, tmp_path, [_extension(b"x")])
    assert browser.spec["id"] == "extensions" and browser.ids == ["sample"]


def test_zip_names_with_a_colon_are_rejected(tmp_path, monkeypatch):
    # On Windows «C:x» is drive-relative and «a.py:s» a data stream: both
    # can write outside the stage. Linux would extract them quietly.
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    for name in ("sample/C:escape.py", "sample/helper.py:stream"):
        bad = io.BytesIO()
        with zipfile.ZipFile(bad, "w") as archive:
            archive.writestr("sample/__init__.py", TOOL_SOURCE)
            archive.writestr(name, "no")
        payload = bad.getvalue()
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
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
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
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    # newline="\n": byte for byte what the catalog serves (on Windows
    # write_text puts \r\n and the copy is no longer identical).
    (tmp_path / "my_old_name.py").write_text(TOOL_SOURCE, newline="\n")
    payload = TOOL_SOURCE.encode()
    with pytest.raises(manager.ExtensionManagerError, match="my_old_name.py"):
        manager.install_extension(_extension(payload))
    assert not (tmp_path / "sample").exists()


def test_a_loose_file_with_crlf_line_endings_counts_as_a_copy(
        tmp_path, monkeypatch):
    # #764: a hand-installed copy whose line endings were converted to
    # CRLF (a Windows editor, a zip tool, Path.write_text on Windows)
    # is the same extension, even though its raw bytes differ.
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    crlf = TOOL_SOURCE.encode().replace(b"\n", b"\r\n")
    (tmp_path / "my_old_name.py").write_bytes(crlf)
    payload = TOOL_SOURCE.encode()
    with pytest.raises(manager.ExtensionManagerError, match="my_old_name.py"):
        manager.install_extension(_extension(payload))
    assert not (tmp_path / "sample").exists()


def test_a_loose_file_with_a_bom_and_crlf_counts_as_a_copy(
        tmp_path, monkeypatch):
    # #764: a UTF-8 BOM saved by a Windows editor must not hide a copy
    # either.
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    crlf = TOOL_SOURCE.encode().replace(b"\n", b"\r\n")
    (tmp_path / "my_old_name.py").write_bytes(b"\xef\xbb\xbf" + crlf)
    payload = TOOL_SOURCE.encode()
    with pytest.raises(manager.ExtensionManagerError, match="my_old_name.py"):
        manager.install_extension(_extension(payload))
    assert not (tmp_path / "sample").exists()


def test_a_package_whose_entry_has_crlf_line_endings_counts_as_a_copy(
        tmp_path, monkeypatch):
    # #764, the package form: only the entry module's endings changed.
    plugins = tmp_path / "plugins"
    plugins.mkdir()
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: plugins)
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
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: tmp_path)
    other = TOOL_SOURCE.replace("Catalog tool", "Another tool")
    (tmp_path / "my_old_name.py").write_bytes(
        other.encode().replace(b"\n", b"\r\n"))
    payload = TOOL_SOURCE.encode()
    manager.install_extension(_extension(payload))
    assert (tmp_path / "sample" / "__init__.py").is_file()
    assert (tmp_path / "my_old_name.py").is_file()


_SHA_A = "0123456789abcdef"
_SHA_B = "fedcba9876543210"


def test_install_selected_local_file(tmp_path, monkeypatch):
    root = tmp_path / "plugins"
    source = tmp_path / "sample.py"
    source.write_text(TOOL_SOURCE)
    monkeypatch.setattr("built_core.extensions.user_plugins_dir", lambda: root)
    manager.install_local_file(source)
    assert (root / "sample" / "__init__.py").read_text() == TOOL_SOURCE
    assert manager.installed_info("sample")["sha256"] == hashlib.sha256(source.read_bytes()).hexdigest()
    manager.set_extension_enabled("sample", False)
    assert not manager.is_extension_enabled("sample")
    manager.uninstall_extension("sample")
    assert source.read_text() == TOOL_SOURCE
