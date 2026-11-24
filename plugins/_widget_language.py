"""Language support for optional Qt extension widgets."""
import sys

from PySide6.QtCore import QLibraryInfo, QTranslator
from PySide6.QtWidgets import QApplication


class _ButtonsOnlyTranslator(QTranslator):
    """Qt's own catalog, limited to the standard button texts.

    ``qtbase_<lang>.qm`` also names the keys — «Control+Mayúsculas+Re Pág»
    for Ctrl+Shift+PgUp in menus, tooltips and the shortcut editor — and
    the shortcuts stay in English on purpose. The button texts live in
    these contexts; everything else is left untranslated. QColorDialog
    joins them (#1178): the colour picker's own labels («Basic colors»,
    «Custom colors», the channel names) are plain words, no key names,
    so they translate with the rest of the interface."""

    _CONTEXTS = frozenset({"QPlatformTheme", "QMessageBox",
                           "QDialogButtonBox", "QColorDialog"})

    _BUILTIN = {
        "vi": {
            "&OK": "&Đồng ý",
            "OK": "Đồng ý",
            "&Cancel": "Hủy",
            "Cancel": "Hủy",
            "&Yes": "&Có",
            "Yes": "Có",
            "&No": "&Không",
            "No": "Không",
            "&Save": "&Lưu",
            "Save": "Lưu",
            "&Discard": "&Hủy thay đổi",
            "Discard": "Hủy thay đổi",
            "&Close": "&Đóng",
            "Close": "Đóng",
            "&Apply": "&Áp dụng",
            "Apply": "Áp dụng",
            "&Open": "&Mở",
            "Open": "Mở",
            "&Help": "&Trợ giúp",
            "Help": "Trợ giúp",
            "Don't Save": "Không lưu",
            "&Don't Save": "&Không lưu",
            "Reset": "Đặt lại",
            "Restore Defaults": "Khôi phục mặc định",
        }
    }

    def __init__(self) -> None:
        super().__init__()
        self._qt = QTranslator()
        self._lang = ""

    def load_qt(self, lang: str) -> bool:
        self._lang = lang
        if lang in self._BUILTIN:
            return True
        folder = QLibraryInfo.path(QLibraryInfo.LibraryPath.TranslationsPath)
        return self._qt.load(f"qtbase_{lang.replace('-', '_')}", folder)

    def isEmpty(self) -> bool:
        if self._lang in self._BUILTIN:
            return False
        return self._qt.isEmpty()

    def translate(self, context, source, disambiguation=None, n=-1):
        if context not in self._CONTEXTS:
            return None                   # not ours: Qt keeps its text
        if self._lang in self._BUILTIN:
            trans = self._BUILTIN[self._lang].get(source)
            if trans:
                return trans
        return self._qt.translate(context, source, disambiguation, n)


#: Kept alive for the whole session: Qt drops a translator that is freed.
_qt_translator: QTranslator | None = None


def _install_qt_translator(lang: str) -> None:
    """Let Qt name its standard buttons in ``lang`` too.

    Our catalog only covers ``tr()`` strings; the standard buttons of
    QMessageBox and QDialogButtonBox (OK, Cancel, Yes, No…) come from
    Qt's own ``qtbase_<lang>.qm``, which PySide6 ships. Without it they
    stayed in English under every language. A missing file just leaves
    them in English, as before."""
    global _qt_translator
    if lang == "en":
        return
    translator = _ButtonsOnlyTranslator()
    if translator.load_qt(lang):
        QApplication.installTranslator(translator)
        _qt_translator = translator

def apply_widget_language(app, lang):
    _install_qt_translator(lang)
    from views.rtl import apply_language_direction
    apply_language_direction(app, lang)
    apply_script_font(app, lang)


#: Language → fonts that carry its script, best first. Leelawadee UI
#: ships with Windows 8 and later (Lao, Thai and Latin); Lao UI with
#: Windows 7; Noto Sans Lao when the user installed it.
SCRIPT_FONTS: dict[str, tuple[str, ...]] = {
    "lo": ("Noto Sans Lao", "Leelawadee UI", "Lao UI"),
}

#: Platforms whose own font fallback breaks these scripts apart.
_PLATFORMS = ("win32",)


def script_font_for(lang: str | None, installed, platform: str | None = None
                    ) -> str | None:
    """The first font of :data:`SCRIPT_FONTS` for ``lang`` that is in
    ``installed`` on ``platform`` (default: this one), or ``None``."""
    platform = sys.platform if platform is None else platform
    if platform not in _PLATFORMS:
        return None
    base = str(lang or "").split("-")[0].split("_")[0].lower()
    have = set(installed)
    for name in SCRIPT_FONTS.get(base, ()):
        if name in have:
            return name
    return None


def apply_script_font(app, lang: str | None, platform: str | None = None
                      ) -> bool:
    """At start-up, after the language is chosen: put the script's font
    first in the application font. Returns whether it did; nothing
    changes when it does not."""
    from PySide6.QtGui import QFontDatabase
    name = script_font_for(lang, QFontDatabase.families(), platform)
    if name is None:
        return False
    font = app.font()
    families = [name] + [f for f in (font.families() or [font.family()])
                         if f and f != name]
    font.setFamilies(families)
    app.setFont(font)
    return True
