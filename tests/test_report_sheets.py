"""Exercise the Rust sheet specs via the same host bindings as both apps."""
from tests.shell_isolation import restore_shell_patches  # noqa: F401
import csv
import io
import json
from types import SimpleNamespace

import pytest

from built_render import chrome_sheet
from built_render import AboutSheet as About, ExtensionsSheet as Extensions, CutListSheet as CutList


@pytest.fixture
def pasteboard(monkeypatch):
    from built_render import shell_services as native

    items = {}
    def write(entries):
        items.clear()
        items.update(entries)
        return True

    monkeypatch.setattr(chrome_sheet, "in_shell", lambda: True)
    monkeypatch.setattr(native, "pasteboard_write", write)
    return items


def build(provider, ctx):
    pytest.importorskip("built_render")
    spec = provider.build(ctx).wire()
    chrome_sheet.check_wire(json.dumps(spec))
    return spec


def test_about_and_clipboard(monkeypatch, native_host, pasteboard):
    ctx = SimpleNamespace(win=native_host.window)
    sheet = About()
    spec = build(sheet, ctx)
    assert spec.get("detail") is None
    for label in ("GPL-3.0-or-later", "Marco Sumari Tellez", "Civil Engineer", "Pedro Caeiro"):
        assert label in str(spec["content"])
    sheet.act(ctx, "verb", ["copy"])
    for field in ("Built ", "Engine:", "Renderer:", "Commit:", "Language:"):
        assert field in pasteboard["public.utf8-plain-text"].decode()
    copied = []
    import webbrowser
    monkeypatch.setattr(webbrowser, "open", copied.append)
    sheet.act(ctx, "verb", ["upstream"])
    assert copied[-1] == "https://github.com/ingelibre/ingetrazo"


def test_extensions_install_update_toggle_safe_reset_remove(tmp_path, monkeypatch):
    from core import extensions
    from built_core import extension_manager as em
    root = tmp_path / "plugins"
    root.mkdir()
    monkeypatch.setattr("core.extensions.user_plugins_dir", lambda: root)
    monkeypatch.setattr(extensions, "plugin_dirs", lambda: [tmp_path, root])
    monkeypatch.setattr(em, "load_cached_catalog", lambda: None)
    monkeypatch.setattr(em, "refresh_catalog", lambda: em.CatalogSnapshot([], {}, "", True))
    example = em.bundled_extensions()[0]
    monkeypatch.setattr(em, "bundled_extensions", lambda: [example])
    sheet = Extensions()
    ctx = SimpleNamespace(win=SimpleNamespace(_running_extensions=set()))
    assert build(sheet, ctx)["tabs"][0]["on"]
    sheet.act(ctx, "tab", ["Browse"])
    build(sheet, ctx)
    sheet.act(ctx, "verb", ["install"])
    assert em.installed_info(example.id)
    sheet.act(ctx, "tab", ["Installed"])
    sheet.act(ctx, "focus", [example.id])
    spec = build(sheet, ctx)
    assert spec["detail"]["title"] == em.display_name(example, "en")
    for enabled in (False, True):
        sheet.act(ctx, "verb", ["toggle"])
        assert em.is_extension_enabled(example.id) is enabled
        build(sheet, ctx)
    marker = root / example.id / "extension.json"
    metadata = json.loads(marker.read_text())
    marker.write_text(json.dumps(dict(metadata, sha256="stale")))
    sheet.act(ctx, "tab", ["Updates"])
    assert build(sheet, ctx)["focus"] == example.id
    sheet.act(ctx, "verb", ["install"])
    assert em.installed_info(example.id)["sha256"] == example.sha256
    sheet.act(ctx, "verb", ["safe"])
    assert em.safe_mode()
    assert extensions.discover_plugins([root]) == ([], [])
    monkeypatch.setattr(sheet, "confirm", lambda *args: True)
    sheet.act(ctx, "verb", ["reset"])
    assert not em.safe_mode()
    sheet.act(ctx, "tab", ["Installed"])
    build(sheet, ctx)
    sheet.act(ctx, "verb", ["remove"])
    assert em.installed_info(example.id) is None


def test_cut_list_group_copy_export(tmp_path, monkeypatch, pasteboard):
    from core.scene import Scene
    from tests.test_parts import _box, _part
    from core.group import Group
    from built_render.shell_files import ShellFileDialog
    assembly = Group(name="Assembly")
    assembly.adopt([_part(_box(.2, .8, .4, color=color), name)
                    for color, name in [((1, 0, 0), 'Red, "panel"'), ((0, 1, 0), "Green")]])
    scene = Scene()
    scene.groups.append(assembly)
    scene.select([assembly])
    ctx = SimpleNamespace(win=SimpleNamespace(viewport=SimpleNamespace(scene=scene)))
    sheet = CutList()
    build(sheet, ctx)
    assert len(sheet.table) == 3
    sheet.act(ctx, "verb", ["group"])
    spec = build(sheet, ctx)
    assert spec["content"]["blocks"][0]["cells"] == sheet.table
    assert len(sheet.table) == 2 and sheet.table[1][0] == "2"
    assert len(sheet.table[1]) == 6
    sheet.act(ctx, "verb", ["copy"])
    assert list(csv.reader(io.StringIO(pasteboard["public.utf8-plain-text"].decode(), newline=""), dialect="excel-tab")) == sheet.table
    path = tmp_path / "parts.csv"
    monkeypatch.setattr(ShellFileDialog, "getSaveFileName", lambda *args: (str(path), "CSV"))
    sheet.act(ctx, "verb", ["csv"])
    with path.open(newline="") as f:
        assert list(csv.reader(f)) == sheet.table


@pytest.mark.parametrize("sid", ["about", "extensions", "cutlist"])
def test_offscreen_window_commands_open_report_sheets(native_host, sid):
    win = native_host.window
    win.run_command(sid)
    assert chrome_sheet.current(win).id == sid
