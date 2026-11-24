"""Standard native button labels follow the selected JSON catalogue."""
import pytest
from core import i18n


@pytest.mark.parametrize("lang,yes,cancel", [
    ("en", "Yes", "Cancel"), ("es", "Sí", "Cancelar"), ("pt-BR", "Sim", "Cancelar"),
    ("vi", "Có", "Hủy bỏ"), ("ja", "はい", "キャンセル"),
])
def test_standard_buttons_follow_language(lang, yes, cancel, monkeypatch):
    from views import chrome_dialogs
    box = chrome_dialogs.QMessageBox
    shown = []

    def capture(parent, title, body, buttons, **options):
        shown.extend(label for _, label in buttons)
        return options["default"]

    monkeypatch.setattr(chrome_dialogs, "ask", capture)
    before = i18n.current_language()
    try:
        i18n.set_language(lang)
        result = chrome_dialogs.question(None, "Title", "Question", box.Yes | box.Cancel, box.Yes)
        assert result == box.Yes
        assert len(shown) == 2 and set(shown) == {yes, cancel}
    finally:
        i18n.set_language(before)
