"""Host binding for the renderer's font picker and retained text geometry."""
import json

from PySide6.QtCore import QSettings
from views import chrome_dialogs, wgpu_ui


def show(win, title, text, family, bold, italic, size, unit, done, *, depth=None, editable=False, bounds=(4, 96)):
    view = chrome_dialogs.view_of(win)
    if not wgpu_ui.ENABLED or not getattr(view, "_ui_attached", False):
        return False
    settings = QSettings()
    draft = dict(title=title, text=text, font=family, bold=bold, italic=italic,
                 height=size, unit=unit, editable=editable, thickness=depth, minimum=bounds[0], maximum=bounds[1])
    for key in ("recent", "favourites"):
        draft[key] = settings.value("fonts/" + key, [], type=list)
    view.renderer.ui_font_picker(json.dumps(draft))
    view._font_done = done
    view.update()
    return True


def answer(win, payload, source_view=None):
    current = chrome_dialogs.view_of(win)
    view = source_view if source_view is not None else current
    done = vars(view).pop("_font_done", None) if view else None
    if done and payload and view is current:
        choice = json.loads(payload)
        settings = QSettings()
        for key in ("recent", "favourites"):
            settings.setValue("fonts/" + key, choice[key])
        done(choice)


def text3d(win, params, done):
    from core import units
    from core.i18n import tr
    from core.text3d import text_params
    draft = {**text_params(tr("Built")), **(params or {})}
    unit = units.model_units_of(win.viewport.scene)["length"]

    def apply(p):
        done(text_params(p["text"].strip(), p["font"], p["bold"], p["italic"],
                         units.metres_from_number(p["height"], unit),
                         units.metres_from_number(p["thickness"], unit)))

    if not show(win, tr("3D Text"), draft["text"], draft["font"], draft["bold"], draft["italic"],
                units.length_number(draft["height"], unit), units.length_suffix(unit), apply,
                depth=units.length_number(draft["thickness"], unit), editable=True,
                bounds=(units.length_number(0.001, unit), units.length_number(1000., unit))):
        if hasattr(win, "_host"):
            return
        choice = win._text3d_dialog(params)
        if choice:
            done(choice)


def annotation(win):
    from views.settings_sheet import Env, _scene_changed
    env = Env(win)
    style = env.scene.dimension_style

    def apply(p):
        style.update(font_family=p["font"], font_bold=p["bold"], font_italic=p["italic"],
                     font_size=max(6, min(28, round(p["height"]))))
        env.scene.version += 1
        _scene_changed(env)

    show(win, "Dimension and label font", "2.40 m", style.get("font_family", ""),
         style.get("font_bold", True), style.get("font_italic", False),
         style.get("font_size", 9), " pt", apply, bounds=(6, 28))
