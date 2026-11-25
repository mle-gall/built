"""Catalogue and script-font selection for the native chrome."""
import os
from pathlib import Path
import sys

from core import i18n


_LAO_FACES = {
    "Noto Sans Lao": ("NotoSansLao-Regular.ttf", "NotoSansLao-VariableFont_wdth,wght.ttf"),
    "Leelawadee UI": ("LeelawUI.ttf",),
    "Lao UI": ("LaoUI.ttf",),
}


def script_font_for(lang, installed, platform=None):
    base = (lang or "").lower().replace("_", "-").split("-")[0]
    if (platform or sys.platform) == "win32" and base == "lo":
        return next((name for name in _LAO_FACES if name in installed), None)
    return None


def apply_widget_language(app, lang):
    i18n.set_language(lang)


def chrome_font(lang=None, platform=None, font_dirs=None):
    """Choose the Windows Lao face when available, otherwise use the app font."""
    platform = platform or sys.platform
    lang = i18n.current_language() if lang is None else lang
    default = Path(__file__).resolve().parents[1] / "resources" / "fonts" / "inter"
    if not default.is_dir():
        default = Path("/System/Library/Fonts/SFNS.ttf")
    if not script_font_for(lang, _LAO_FACES, platform):
        return str(default)
    if font_dirs is None:
        font_dirs = [Path(os.environ.get("WINDIR", "C:/Windows")) / "Fonts"]
        if local := os.environ.get("LOCALAPPDATA"):
            font_dirs.insert(0, Path(local) / "Microsoft" / "Windows" / "Fonts")
    installed = {}
    for family, files in _LAO_FACES.items():
        for directory in font_dirs:
            path = next((directory / filename for filename in files if (directory / filename).is_file()), None)
            if path:
                installed[family] = path
                break
    chosen = script_font_for(lang, installed, platform)
    return str(installed.get(chosen, default))


FONT = chrome_font()
