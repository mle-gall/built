"""Drive the Library's registry tab through its normal sheet commands."""
import json
from types import SimpleNamespace
from unittest.mock import Mock

import pytest

from core import bim_registry
from core.history import History
from core.scene import Scene
from built_render import chrome_sheet
from built_render import wgpu_library


@pytest.fixture
def library(monkeypatch):
    scene = Scene()
    view = SimpleNamespace(update=lambda: None)
    win = SimpleNamespace(viewport=SimpleNamespace(scene=scene, history=History(scene)))
    ctx = chrome_sheet.Ctx(win, view)
    lib = wgpu_library.LibrarySheet(view)
    monkeypatch.setattr(lib, "_items", lambda ctx: ([], {}))
    monkeypatch.setattr(chrome_sheet, "close", Mock())
    lib.act(ctx, "tab", ["registry"])
    sheet = lib.build(ctx)
    chrome_sheet.check_wire(json.dumps(sheet.wire()))
    return lib, ctx, sheet.forms()[0]


def test_namespace_import_and_undo(library, tmp_path, monkeypatch):
    lib, ctx, form = library
    vp = ctx.win.viewport
    original = bim_registry.get(vp.scene)
    form.receive("registry_key", "survey")
    form.receive("registry_name", "Survey")
    lib.act(ctx, "verb", ["registry_add"])
    draft = json.loads(form.field("registry_json")[3])
    assert draft["classification_systems"]["survey"]["name"] == "Survey"
    for key, text in (("registry_key", "survey"), ("registry_name", "Duplicate")):
        form.receive(key, text)
    lib.act(ctx, "verb", ["registry_add"])
    assert "already exists" in form.error and json.loads(form.field("registry_json")[3]) == draft
    path = tmp_path / "codes.csv"
    from built_render.shell_files import ShellFileDialog
    monkeypatch.setattr(ShellFileDialog, "getOpenFileName", lambda *args: (str(path), "CSV"))
    path.write_text("code,title\nA,First\nA,Duplicate")
    lib.act(ctx, "verb", ["registry_csv"])
    assert "duplicate" in form.error and json.loads(form.field("registry_json")[3]) == draft
    path.write_text("code,title,parent\nA,First,\nB,Second,A", encoding="utf-8-sig")
    lib.act(ctx, "verb", ["registry_csv"])
    assert not form.error and bim_registry.get(vp.scene) == original
    lib.act(ctx, "verb", ["registry_apply"])
    chrome_sheet.close.assert_called_once_with(ctx.win)
    assert bim_registry.get(vp.scene)["classification_systems"]["survey"]["codes"]["B"]["parent"] == "A"
    assert vp.history.undo() and bim_registry.get(vp.scene) == original


def test_invalid_or_stale_drafts_do_not_overwrite(library):
    lib, ctx, form = library
    vp = ctx.win.viewport
    original = form.field("registry_json")[3]
    for invalid in ("{", "[]", '{"classification_systems":[]}'):
        form.receive("registry_json", invalid)
        lib.act(ctx, "verb", ["registry_apply"])
        assert form.error and form.field("registry_json")[3] == invalid
        assert not vp.history.undo_stack
    form.receive("registry_json", original)
    vp.scene.plugin_data["bim_registry"] = {"project": {"name": "Newer"}}
    lib.act(ctx, "verb", ["registry_apply"])
    assert "changed" in form.error and not vp.history.undo_stack
    lib.closed(ctx)
    assert lib.registry is None
    reopened = lib.build(ctx).forms()[0]
    assert json.loads(reopened.field("registry_json")[3]) == bim_registry.get(vp.scene)
    ctx.win.viewport.scene = Scene()
    assert not reopened.act(ctx.win, "registry_apply") and "model changed" in reopened.error


def test_read_only_model_preserves_the_draft(library):
    lib, ctx, form = library
    vp = ctx.win.viewport
    original = bim_registry.get(vp.scene)
    form.receive("registry_name", "Survey")
    form.receive("registry_key", "survey")
    lib.act(ctx, "verb", ["registry_add"])
    vp.scene.worksession_read_only = True
    lib.act(ctx, "verb", ["registry_apply"])
    assert form.error and "survey" in form.field("registry_json")[3]
    assert bim_registry.get(vp.scene) == original and not vp.history.undo_stack
    chrome_sheet.close.assert_not_called()
