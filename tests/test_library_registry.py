"""Drive the model settings registry page through its normal sheet commands."""
import json
from types import SimpleNamespace
from unittest.mock import Mock

import pytest

from core import bim_registry
from built_core.history import History
from built_core.scene import Scene
from built_render import chrome_sheet
from built_render import settings_sheet


@pytest.fixture
def library(monkeypatch):
    scene = Scene()
    view = SimpleNamespace(update=lambda: None)
    win = SimpleNamespace(viewport=SimpleNamespace(scene=scene, history=History(scene)))
    ctx = chrome_sheet.Ctx(win, view)
    lib = settings_sheet.SettingsSheet("registry")
    monkeypatch.setattr(chrome_sheet, "close", Mock())
    sheet = lib.build(ctx)
    wire = sheet.wire()
    chrome_sheet.check_wire(json.dumps(wire))
    blocks = wire["content"]["blocks"]
    assert "Classification systems" in [b.get("text") for b in blocks]
    buttons = next(b["buttons"] for b in blocks if b["kind"] == "buttons")
    assert [b["id"] for b in buttons] == ["registry_advanced", "registry_add", "registry_csv", "registry_apply"]
    assert "registry_json" not in [row[0] for row in sheet.forms()[0].wire()[4]]
    lib.act(ctx, "verb", ["registry_advanced"])
    assert "registry_json" in [row[0] for row in sheet.forms()[0].wire()[4]]
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


def test_bundled_wood_variants_keep_their_identity_and_have_distinct_labels():
    import json
    from pathlib import Path
    from built_core import bundled_materials

    resources = Path(__file__).resolve().parents[1] / "resources"
    wood = [item for item in json.loads(bundled_materials(resources, "en"))
            if item["name"] in ("Oak", "Pine")]
    assert len(wood) == len({item["label"] for item in wood}) == 5
    assert all(item["recipe"]["name"] == item["name"] for item in wood)
