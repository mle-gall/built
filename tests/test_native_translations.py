"""Standard native button labels follow the selected JSON catalogue."""
import pytest
from core import i18n
from core.input_codes import StandardButton
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
