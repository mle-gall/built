"""Standard native button labels follow the selected JSON catalogue."""
import pytest
from ingetrazo_core import i18n
from ingetrazo_core import StandardButton
from ingetrazo_render import chrome_dialogs


@pytest.mark.parametrize("lang,yes,cancel", [
    ("en", "Yes", "Cancel"), ("es", "Sí", "Cancelar"), ("pt-BR", "Sim", "Cancelar"),
    ("vi", "Có", "Hủy bỏ"), ("ja", "はい", "キャンセル"),
])
def test_standard_buttons_follow_language(lang, yes, cancel, monkeypatch):
    shown = []

    def capture(parent, title, body, buttons, **options):
        shown.extend(label for _, label in buttons)
        return options["default"]

    monkeypatch.setattr(chrome_dialogs, "ask", capture)
    before = i18n.current_language()
    try:
        i18n.set_language(lang)
        result = chrome_dialogs.question(None, "Title", "Question",
                                         StandardButton.Yes | StandardButton.Cancel, StandardButton.Yes)
        assert result == StandardButton.Yes
        assert len(shown) == 2 and set(shown) == {yes, cancel}
    finally:
        i18n.set_language(before)


def test_translation_api_in_a_fresh_plugin_interpreter():
    import subprocess
    import sys

    from ingetrazo_core.paths import app_root

    subprocess.run([sys.executable, "-c", """
from core.i18n import (LANGUAGE_NAMES, RTL_LANGUAGES, HIDDEN_LANGUAGES, SYSTEM,
    available_languages, current_language, is_rtl, language_for_locales,
    resolve_language, set_language, source_of, tr)
from ingetrazo_core import i18n
assert set_language is i18n.set_language and tr is i18n.tr
set_language("es")
assert i18n.current_language() == "es" and tr("Yes") == "Sí"
"""], cwd=app_root(), check=True, timeout=30)


def test_catalogue_switch_keeps_shortcut_sources_and_format_fallback(tmp_path, monkeypatch):
    import json
    import sys

    catalogs = tmp_path / "i18n"
    catalogs.mkdir()
    (catalogs / "demo.json").write_text(json.dumps({
        "First": "Shared", "Last": "Shared", "{n} faces": "{wrong} caras",
        "Value: {x:.2f}": "Resultado: {x:.2f}",
    }), encoding="utf-8")
    (catalogs / "broken.json").write_text("{", encoding="utf-8")
    before = i18n.current_language()
    try:
        with monkeypatch.context() as m:
            m.setattr(sys, "_MEIPASS", str(tmp_path), raising=False)
            i18n.set_language("demo")
            assert i18n.source_of("Shared") == "Last"
            assert [i18n.tr("{n} faces", n=n) for n in (0, 1, 2)] == [
                "0 faces", "1 faces", "2 faces"]
            assert i18n.tr("Value: {x:.2f}", x=3) == "Resultado: 3.00"
            assert i18n.tr("Literal {") == "Literal {"
            with pytest.raises(KeyError):
                i18n.tr("{n} faces", other=1)
            for code in ("broken", "missing", "en"):
                i18n.set_language(code)
                assert i18n.tr("First") == "First"
                assert i18n.source_of("Shared") == "Shared"
                assert i18n.current_language() == code
    finally:
        i18n.set_language(before)
