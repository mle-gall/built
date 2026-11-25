# SPDX-License-Identifier: GPL-3.0-or-later
"""Named, localized RAL paint from the native Library."""
import json
from ingetrazo_core.i18n import current_language, set_language
from ingetrazo_core.paths import app_root
from ingetrazo_core import bundled_materials
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tools.paint import PaintTool
from ingetrazo_render import wgpu_library as library


def test_picking_a_ral_colour_paints_a_material_with_its_name(native_host):
    was = current_language()
    try:
        set_language("es")
        library.paint_from_strip(native_host.window, "ral:RAL 7035")
        assert PaintTool.current_material.name == "RAL 7035 Gris claro"
        assert PaintTool.current_texture is None
        assert [round(v * 255) for v in PaintTool.current_color] == [203, 208, 204]
    finally:
        set_language(was)


def test_the_families_build_only_when_opened(request, monkeypatch):
    from views import wgpu_ui
    calls = []
    real = library._bundled_by_key
    monkeypatch.setattr(library, "_bundled_by_key", lambda *a: calls.append(a) or real(*a))
    native_host = request.getfixturevalue("native_host")
    wgpu_ui._components_in_model(native_host.view, native_host.window)
    assert calls == []
    library.paint_from_strip(native_host.window, "ral:RAL 7035")
    assert calls


def test_the_ral_name_follows_the_language(tmp_path):
    (tmp_path / "colors").mkdir()
    entries = [{"code": "RAL 7035", "name": "Light grey", "name_es": "Gris claro", "rgb": [203,208,204]},
               {"code": "RAL X", "name": "Grey", "rgb": [100,100,100]}]
    (tmp_path / "colors/ral.json").write_text(json.dumps({"families": [{"colors": entries}]}))
    (tmp_path / "textures").mkdir()
    (tmp_path / "textures/library.json").write_text("{}")
    def names(lang):
        return [i["name"] for i in json.loads(bundled_materials(tmp_path, lang)) if i["key"].startswith("ral:")]
    assert names("es") == ["RAL 7035 Gris claro", "RAL X Grey"]
    assert names("en") == ["RAL 7035 Light grey", "RAL X Grey"]


def test_every_colour_in_the_library_has_a_name():
    catalogue = json.loads(bundled_materials(app_root() / "resources", "en"))
    colours = [r for r in catalogue if r["category"] == "Colours (RAL)"]
    assert len(colours) == 213
    assert all(r["name"].startswith(r["short"] + " ") and r["recipe"]["name"] == r["name"] for r in colours)
