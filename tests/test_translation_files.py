# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Every translation file loads and keeps the placeholders of its source.

Contributed catalogs arrive hand-edited: a missing comma makes the whole
file unreadable (the language silently falls back to English), and a
translated placeholder — ``{名称}`` for ``{name}`` — makes ``tr`` fall back
to English for that message. Both happened with the first Chinese file.
"""
import json
import string

import pytest

from built_core.i18n import LANGUAGE_NAMES, available_languages
from built_core.paths import app_root

_I18N_DIR = app_root() / "i18n"
_FILES = sorted(_I18N_DIR.glob("*.json"))


def _fields(text):
    return sorted({f for _, f, _, _ in string.Formatter().parse(text) if f})


@pytest.mark.parametrize("path", _FILES, ids=lambda p: p.stem)
def test_file_is_valid_json_map(path):
    data = json.loads(path.read_text(encoding="utf-8"))
    assert isinstance(data, dict)
    assert all(isinstance(k, str) and isinstance(v, str) for k, v in data.items())


@pytest.mark.parametrize("path", _FILES, ids=lambda p: p.stem)
def test_placeholders_survive_translation(path):
    data = json.loads(path.read_text(encoding="utf-8"))
    bad = [k for k, v in data.items() if _fields(k) != _fields(v)]
    assert not bad, f"{len(bad)} entries change their {{placeholders}}: {bad[:3]}"


def test_every_language_has_a_menu_name():
    assert set(available_languages()) <= set(LANGUAGE_NAMES)


@pytest.mark.parametrize("path", [p for p in _FILES
                                  if p.stem not in ("en", "es")],
                         ids=lambda p: p.stem)
def test_every_language_has_every_message(path):
    """A message added in Spanish and English but in no other catalog reads
    in English there with no warning: 21 strings of Join Segments and Zoom
    Window sat untranslated in French and Polish until a review of the
    language PRs happened on them (17-10)."""
    source = json.loads((_I18N_DIR / "es.json").read_text(encoding="utf-8"))
    data = json.loads(path.read_text(encoding="utf-8"))
    missing = [k for k in source if k not in data]
    assert not missing, f"{len(missing)} messages missing: {missing[:3]}"


def test_traditional_chinese_is_offered_under_its_own_name():
    """The file name is the language code, with a hyphen like ``zh-CN``: with
    an underscore the system's language would not find it (#1010)."""
    assert "zh-TW" in available_languages()
    assert LANGUAGE_NAMES["zh-TW"] == "正體中文"


def test_bulgarian_is_offered_under_its_own_name():
    """Bulgarian (#1185) only needs its file and its name."""
    assert "bg" in available_languages()
    assert LANGUAGE_NAMES["bg"] == "Български"


def test_traditional_chinese_keeps_the_composer_tab_generic():
    """Its source used a product's name for the sheet layout tab."""
    data = json.loads((_I18N_DIR / "zh-TW.json").read_text(encoding="utf-8"))
    assert data["Layout"] == "版面配置"


def test_lao_is_offered_again():
    """Lao (#850) is offered again since its marks compose on Windows
    (Windows round 0.6.0, views/script_fonts.py)."""
    assert (_I18N_DIR / "lo.json").is_file()
    assert {"lo", "th", "ja", "ko", "vi"} <= set(available_languages())


#: Scripts written without spaces between words: a suffix there attaches
#: straight to the number or word before it.
_NO_SPACE_SCRIPTS = {"ja", "ko", "zh-CN", "zh-TW"}


@pytest.mark.parametrize("path", [p for p in _FILES
                                  if p.stem not in _NO_SPACE_SCRIPTS],
                         ids=lambda p: p.stem)
def test_leading_and_trailing_spaces_are_kept(path):
    """A message that starts or ends with a space is glued to something
    else (« decimals» after the number of a spin box): its translation
    keeps the space, or the Windows round 0.6.0 reads «2الكسور العشرية»."""
    data = json.loads(path.read_text(encoding="utf-8"))
    lost = [k for k, v in data.items() if v and (
        (k.startswith(" ") and not v[:1].isspace())
        or (k.endswith(" ") and not v[-1:].isspace()))]
    assert not lost, f"{len(lost)} lost their space: {lost[:3]}"


def test_contributor_lines_have_translations():
    """About's roll of contributors: each description goes through
    ``tr()``, but a line whose description never entered the catalogs
    was invisible to ``test_every_language_has_every_message`` — that
    test only compares catalogs against es.json, and the line was
    missing from es.json too. 26 lines sat untranslated in every
    language that way until the 0.6.1 credits update (#1271). Pin the
    source list itself: a new CONTRIBUTORS line without its es.json
    entry fails here, and the completeness test above then carries
    the entry to every other catalog."""
    from built_render import AboutReview
    source = json.loads((_I18N_DIR / "es.json").read_text(encoding="utf-8"))
    missing = [role for _name, role, _link, _starred in AboutReview.contributors()
               if role not in source]
    assert not missing, (
        f"{len(missing)} contributor lines have no translation entry: "
        f"{missing[:3]}")


def test_inventory_tracks_new_ui_strings_and_drops_retired_catalogue_keys(tmp_path, monkeypatch):
    from scripts import translation_coverage as audit

    monkeypatch.setattr(audit, "ROOT", tmp_path)
    catalogues = tmp_path / "i18n"
    catalogues.mkdir()
    (catalogues / "fr.json").write_text('{"Old": "Ancien", "Menu": "Menu"}')
    plugins = tmp_path / "plugins"
    plugins.mkdir()
    (plugins / "demo.py").write_text('tr("New " "plugin label")\nmenu = "Menu"\n')
    src = tmp_path / "rust/built_render/src"
    src.mkdir(parents=True)
    (src / "example.rs").write_text(
        'tr(py, "New Rust label"); tr.call1(("Hint",)); ui.label("New control");')
    (src / "preference_rows.json").write_text('{}')
    assert audit.inventory() == ["Hint", "Menu", "New Rust label", "New control", "New plugin label"]


def test_reviewed_panels_translate_literal_widget_labels():
    import ast
    import re
    from pathlib import Path

    root = Path(__file__).resolve().parents[1]
    labels = {"field": 1, "section": 1, "item": 2}
    for filename in ("_blender_panel.py", "python_console.py"):
        tree = ast.parse((root / "plugins" / filename).read_text())
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Name):
                continue
            index = labels.get(node.func.id)
            if index is not None and len(node.args) > index:
                arg = node.args[index]
                assert not (isinstance(arg, ast.Constant) and isinstance(arg.value, str)
                            and arg.value.strip()), (filename, node.lineno)

    bare = re.compile(r'(?:ui\.(?:label|strong|button)|on_hover_text|RichText::new)\(\s*"[A-Za-z]')
    for name in ("activity.rs", "sheets.rs"):
        source = (root / "rust/built_render/src/ui" / name).read_text().split("#[cfg(test)]")[0]
        assert not bare.search(source), name
