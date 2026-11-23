# SPDX-License-Identifier: GPL-3.0-or-later
"""Settings as a large sheet (``views/chrome_sheet.py``): option B of the
2026-11-23 review, the Preferences of the Built window.

Sections on the left, in two groups: **Built · this Mac** (General,
Navigation, Display, 3D mouse, Import, Keyboard, AI assistant, Engine),
kept in QSettings, and **This model** (Units, Dimension style, Location &
north), kept in the document and saved with it. A search lists the
matching rows of every section, each under its section's name, so nothing
hides behind one.

Every change applies at once (no OK, no Cancel), through the same
host functions as the Qt Preferences dialog (``views/settings_apply.py``); a change that only takes effect at the next start
says so at the top of the sheet.

"""
from __future__ import annotations

from core.i18n import tr
from views import chrome_sheet
from views import chrome_forms as cf

#: The sections, in the side list's order: (id, label, scope).
MAC, MODEL = "mac", "model"
SECTIONS = (
    ("general", "General", MAC),
    ("navigation", "Navigation", MAC),
    ("display", "Display", MAC),
    ("ndof", "3D mouse", MAC),
    ("import", "Import", MAC),
    ("keyboard", "Keyboard", MAC),
    ("ai", "AI assistant", MAC),
    ("engine", "Engine", MAC),
    ("units", "Units", MODEL),
    ("dimensions", "Dimension style", MODEL),
    ("location", "Location & north", MODEL),
)
_LABELS = {sid: label for sid, label, _scope in SECTIONS}
_SUBS = {
    "general": "Applies at once.",
    "navigation": "Applies at once. Shift pans and ⌘ zooms whatever is chosen here.",
    "display": "Applies at once, to every open project.",
    "ndof": "Applies at once. Editing an axis makes the mapping Custom.",
    "import": "The answers the import dialogs start with; each still asks.",
    "keyboard": "Pick a command and change its keys. Changes apply at once.",
    "ai": "The AI Assistant reads these when it next sends a message.",
    "engine": "How Built computes and draws.",
    "units": "Saved with this model. A number typed without a unit is in this unit.",
    "dimensions": "Saved with this model: how its dimensions read.",
    "location": "Saved with this model: where it stands and where north lies.",
}
#: Other names a caller may give a page.
_ALIASES = {"3dmouse": "ndof", "mouse3d": "ndof", "shortcuts": "keyboard", "keys": "keyboard",
            "dimension_style": "dimensions", "dimstyle": "dimensions", "north": "location",
            "ia": "ai", "assistant": "ai"}

#: Changes waiting for the next start: {row id: (label, value before)}.
#: Module-wide, so closing and reopening the sheet still says it.
_PENDING: dict = {}
_NO = object()


# ---- What the rows read and write ----------------------------------------------
class Env:
    """What a row's getter and setter see: the window, its viewport and
    scene, and one QSettings for the whole build or change."""

    def __init__(self, win):
        from PySide6.QtCore import QSettings
        self.win = win
        self.st = QSettings()
        self.vp = getattr(win, "viewport", None)
        self.scene = getattr(self.vp, "scene", None)


class Row:
    """One setting: how to read it, how to apply it, how to draw it.

    ``get(env)`` → the value; ``put(env, value)`` applies it at once.
    ``options``/``enabled``/``visible`` may be callables of ``env``.
    ``restart``: what the restart note names when the value changed;
    ``default``: the value Restore defaults puts back (absent: left)."""

    def __init__(self, rid, kind, label, get=None, put=None, *, options=(), lo=0.0, hi=0.0,
                 decimals=0, suffix="", tip="", restart="", default=_NO, enabled=True,
                 visible=True, row_label="", words="", pick=None, filters="", password=False,
                 fmt=None, press=None, needs_restart=None, start=None):
        self.id, self.kind, self.label = rid, kind, label
        self.get, self.put, self.press = get, put, press
        self.options, self.lo, self.hi, self.decimals = options, lo, hi, decimals
        self.suffix, self.tip, self.restart, self.default = suffix, tip, restart, default
        self.enabled, self.visible, self.row_label, self.words = enabled, visible, row_label, words
        self.pick, self.filters, self.password, self.fmt = pick, filters, password, fmt
        self.needs_restart = needs_restart
        self.start = start              # what runs now, when not the value

    def opts(self, env):
        o = self.options(env) if callable(self.options) else self.options
        return [(x, x) if isinstance(x, str) else (x[0], x[1]) for x in o]

    def on(self, attr, env) -> bool:
        v = getattr(self, attr)
        return bool(v(env)) if callable(v) else bool(v)

    def value(self, env):
        return self.get(env) if self.get is not None else None

    def field(self, env) -> cf.Field:
        kind = self.kind
        value = self.value(env)
        if kind == "text" and self.label and value:
            value = f"{self.label}: {value}"     # a text row draws its text only
        return cf.Field(self.id, kind, self.label, value, options=self.opts(env)
                        if kind == "choice" else (), lo=self.lo, hi=self.hi, decimals=self.decimals,
                        suffix=self.suffix, tip=self.tip, enabled=self.on("enabled", env),
                        row_label=self.row_label, password=self.password, pick=self.pick,
                        filters=self.filters, fmt=self.fmt,
                        note=self.value(env) if kind == "button" and self.get else "")

    def haystack(self, env, section, group) -> str:
        parts = [section, group, self.label, self.row_label, self.tip, self.words]
        if self.kind == "choice":
            parts += [label for _k, label in self.opts(env)]
        return " ".join(str(p) for p in parts if p).lower()


def _b(raw, default="1") -> bool:
    return str(raw if raw is not None else default).lower() not in ("0", "false", "")


def _int(raw, default: int) -> int:
    try:
        return int(float(raw))
    except (TypeError, ValueError):
        return default


def _key(key, default=None, kind=str):
    """A row getter for a plain QSettings key."""
    def get(env):
        raw = env.st.value(key, default)
        if kind is bool:
            return _b(raw, "1" if default in (True, "1") else "0")
        if kind is int:
            return _int(raw, default)
        return str(raw if raw is not None else default)
    return get


def _set_key(key, kind=str, then=None):
    def put(env, value):
        env.st.setValue(key, ("1" if value else "0") if kind is bool else value)
        env.st.sync()
        if then is not None:
            then(env)
    return put


def _gkey(name, default, kind=str):
    """A getter for a ``core.general_settings`` key."""
    def get(env):
        from core import general_settings as gs
        raw = gs.value(name, default, env.st)
        if kind is bool:
            return _b(raw, default)
        if kind is int:
            return _int(raw, default)
        return str(raw if raw is not None else default)
    return get


def _set_gkey(name, kind=str):
    def put(env, value):
        from core import general_settings as gs
        gs.set_value(name, ("1" if value else "0") if kind is bool else value, env.st)
        env.st.sync()
    return put


def _p():
    from views import settings_apply
    return settings_apply


# ---- Built · this Mac ------------------------------------------------------------
def _general(env) -> list:
    from core.i18n import LANGUAGE_NAMES, SYSTEM, available_languages
    from views.icons import TOOLBAR_ICON_SIZES, default_toolbar_icon_px, toolbar_icon_px
    from views.theme import (ACCENTS, CAMPBELL, DARK, LIGHT, ORANGE, SAND, SLATE, SOLARIZED, STUDIO,
                             SYSTEM as THEME_SYSTEM, saved_accent, saved_theme)
    p = _p()

    def lang_restart(value, running):
        from core.i18n import resolve_language
        return resolve_language(value) != running

    def running_language(env):
        from core.i18n import current_language
        return current_language()

    look = [
        Row("language", "choice", tr("Language"),
            _key("language", SYSTEM),
            lambda env, v: p.apply_language(v, env.st),
            options=[(SYSTEM, tr("Same as the system"))]
            + [(c, LANGUAGE_NAMES.get(c, c)) for c in available_languages()],
            restart=tr("Language"), needs_restart=lang_restart, start=running_language,
            tip=tr("Takes effect at the next start.")),
        Row("theme", "choice", tr("Theme"), lambda env: saved_theme(),
            lambda env, v: p.apply_theme_choice(v),
            options=[(DARK, tr("Dark")), (SLATE, tr("Slate")), (STUDIO, tr("Studio")),
                     (SOLARIZED, tr("Solarized")), (CAMPBELL, tr("Campbell")), (LIGHT, tr("Light")),
                     (SAND, tr("Sand")), (THEME_SYSTEM, tr("Same as the system"))],
            default=DARK,
            tip=tr("Colours of menus, panels and toolbars. «Same as the system» switches along with "
                   "the desktop's light or dark mode. The 3D view keeps its own style.")),
        Row("accent", "choice", tr("Accent colour"), lambda env: saved_accent(),
            lambda env, v: p.apply_accent_choice(v),
            options=[(k, tr(label)) for k, label, _rgb in ACCENTS], default=ORANGE,
            tip=tr("Colour of the details of the icons and of the toolbar buttons under the mouse.")),
    ]
    from views import mac_glass
    if mac_glass.supported():
        look.append(Row("glass", "check", tr("Liquid Glass window chrome"),
                        lambda env: mac_glass.enabled(),
                        lambda env, v: p.apply_window_glass(v),
                        restart=tr("Window chrome"), default=True,
                        tip=tr("Toolbars, panels and menus on macOS's translucent glass. "
                               "Takes effect at the next start.")))
    look += [
        Row("icon_px", "choice", tr("Toolbar icons"), lambda env: toolbar_icon_px(),
            lambda env, v: p.apply_toolbar_icon_px(env.win, int(v)),
            options=[(px, f"{tr(label)} ({px} px)") for px, label in TOOLBAR_ICON_SIZES],
            default=default_toolbar_icon_px()),
        Row("ext_menus", "choice", tr("Extension menus"), _gkey("ext_menus", "auto"),
            _set_gkey("ext_menus"),
            options=[("auto", tr("Group them when there are many")),
                     ("always", tr("Always in submenus")), ("flat", tr("Each entry on its own"))],
            default="auto",
            tip=tr("How the extensions' entries show in the right-click menu (at once) and in the "
                   "Extensions menu (at the next start).")),
        Row("hover_picture", "check", tr("Show a picture of the project on hover over its tab"),
            _key("tabs/hover_picture", "1", bool), _set_key("tabs/hover_picture", bool), default=True),
    ]

    def autosave_put(env, on=None, minutes=None, backup=None):
        g = _gkey
        on = g("autosave", "1", bool)(env) if on is None else on
        minutes = g("autosave_min", 5, int)(env) if minutes is None else minutes
        p.apply_autosave(env.win, on, minutes, backup, env.st)
        env.st.sync()

    files = [
        Row("autosave", "check", tr("Auto-save"), _gkey("autosave", "1", bool),
            lambda env, v: autosave_put(env, on=v), default=True),
        Row("autosave_min", "number", tr("Every"), _gkey("autosave_min", 5, int),
            lambda env, v: autosave_put(env, minutes=v), lo=1, hi=60, suffix=tr("min"), default=5,
            enabled=lambda env: _gkey("autosave", "1", bool)(env)),
        Row("backup", "check", tr("Keep a backup of the previous save (.bak)"),
            _gkey("backup", "1", bool), lambda env, v: autosave_put(env, backup=v), default=True),
        Row("undo_steps", "number", tr("Undo steps"), _gkey("undo_steps", 200, int),
            lambda env, v: (p.apply_undo_steps(env.win, v, env.st), env.st.sync()),
            lo=0, hi=5000, default=200,
            tip=tr("Steps kept for Undo. Each one holds a snapshot of the model, so fewer steps mean "
                   "less memory on a big model; 0 = unlimited.")),
    ]
    from core.update_check import KEY_AUTO, store_managed
    if not store_managed():
        files.append(Row("check_updates", "check", tr("Check for new versions at startup"),
                         _key(KEY_AUTO, "1", bool), _set_key(KEY_AUTO, bool), default=True,
                         tip=tr("Once a day, Built asks GitHub for the latest version number. "
                                "Nothing about you or your files is sent.")))
    return [("look", tr("Look"), look), ("files", tr("Files and undo"), files),
            ("newdoc", tr("New documents"), _figure_rows(env))]


def _figure_rows(env) -> list:
    from core import scale_figure as sf

    def custom(env):
        return _figure_pick(env) == sf.CUSTOM

    def put_pick(env, v):
        if v == sf.CUSTOM:
            image = str(env.st.value(sf.KEY_IMAGE, "") or "")
            from pathlib import Path
            if not (image and Path(image).is_file()):
                SettingsSheet.figure_custom = True     # a picture first
                return
        SettingsSheet.figure_custom = False
        env.st.setValue(sf.KEY_PICK, v)
        env.st.sync()

    def put_image(env, path):
        from pathlib import Path
        from PySide6.QtGui import QImage
        if not path:
            return
        img = QImage(path)
        if img.isNull() or img.height() == 0:
            raise ValueError(tr("That file is not a picture Built can read."))
        try:
            stored = str(sf.store_picture(path))
        except (OSError, ValueError):
            raise ValueError(tr("The picture could not be copied; the previous figure is kept."))
        env.st.setValue(sf.KEY_IMAGE, stored)
        env.st.setValue(sf.KEY_PICK, sf.CUSTOM)
        if not str(env.st.value(sf.KEY_NAME, "") or "").strip():
            env.st.setValue(sf.KEY_NAME, Path(path).stem.replace("_", " "))
        env.st.setValue(sf.KEY_HEIGHT, round(float(env.st.value(sf.KEY_HEIGHT, 1.70) or 1.70), 3))
        env.st.sync()
        SettingsSheet.figure_custom = False
        if not img.hasAlphaChannel():
            SettingsSheet.note = (tr("This picture has no transparent background, so it will show as "
                                     "a rectangle. A PNG cut out around the person looks best."), "warn")

    def height(env):
        try:
            return float(env.st.value(sf.KEY_HEIGHT, 1.70))
        except (TypeError, ValueError):
            return 1.70

    on = _key("new_document/scale_figure", "1", bool)
    return [
        Row("scale_figure", "check", tr("Put the scale figure in new documents"), on,
            _set_key("new_document/scale_figure", bool), default=True),
        Row("figure", "choice", tr("Scale figure"), _figure_pick, put_pick,
            options=lambda env: [(p["key"], f"{tr(p['name'])} ({float(p['height']):.2f} m)")
                                 for p in sf.people()] + [(sf.CUSTOM, tr("My own picture…"))],
            default=sf.DEFAULT_PICK, enabled=on),
        Row("figure_image", "path", tr("Picture"),
            lambda env: str(env.st.value(sf.KEY_IMAGE, "") or ""), put_image, pick="open",
            filters=tr("Images") + " (*.png *.webp);;" + tr("All files") + " (*)",
            visible=custom, enabled=on, tip=tr("A PNG on a transparent background")),
        Row("figure_height", "number", tr("Height"), height,
            lambda env, v: (env.st.setValue(sf.KEY_HEIGHT, round(float(v), 3)), env.st.sync()),
            lo=sf.MIN_HEIGHT, hi=sf.MAX_HEIGHT, decimals=2, suffix="m", visible=custom, enabled=on,
            tip=tr("The height this person stands")),
        Row("figure_name", "edit", tr("Name"), lambda env: str(env.st.value(sf.KEY_NAME, "") or ""),
            lambda env, v: (env.st.setValue(sf.KEY_NAME, v.strip()), env.st.sync()),
            visible=custom, enabled=on),
    ]


def _figure_pick(env):
    from core import scale_figure as sf
    if SettingsSheet.figure_custom:
        return sf.CUSTOM
    return str(env.st.value(sf.KEY_PICK, sf.DEFAULT_PICK) or sf.DEFAULT_PICK)


_SLIDES = (("orbit", "Orbits the view"), ("pan", "Pans the view"), ("zoom", "Zooms the view"))


def _navigation(env) -> list:
    from tools.walkthrough import DEFAULT_LOOK_SENSITIVITY, look_sensitivity
    from views import trackpad_nav
    p = _p()

    def slides(env, trackpad=None, touch=None, inertia=None):
        p.apply_slides(env.win,
                       trackpad_nav.load_slide_mode() if trackpad is None else trackpad,
                       trackpad_nav.load_touch_slide() if touch is None else touch,
                       trackpad_nav.load_inertia() if inertia is None else inertia, env.st)
        env.st.sync()
    slide_opts = [(k, tr(label)) for k, label in _SLIDES]
    mouse = [
        Row("invert_wheel", "check", tr("Invert wheel zoom"), _key("nav/invert_wheel", "0", bool),
            lambda env, v: (p.apply_invert_wheel(env.win, v, env.st), env.st.sync()), default=False),
        Row("invert_orbit", "check", tr("Invert vertical orbit"), _key("nav/invert_orbit_y", "0", bool),
            lambda env, v: (p.apply_invert_orbit(env.win, v, env.st), env.st.sync()), default=False),
        Row("look_sens", "number", tr("Mouse-look sensitivity"), lambda env: look_sensitivity(),
            _set_key("walk/look_sensitivity"), lo=1, hi=100, default=DEFAULT_LOOK_SENSITIVITY,
            tip=tr("How fast a drag turns the head in First Person"), words="first person walk"),
    ]
    touch = [
        Row("trackpad_slide", "choice", tr("Two-finger slide on the trackpad"),
            lambda env: trackpad_nav.load_slide_mode(), lambda env, v: slides(env, trackpad=v),
            options=slide_opts, default="zoom"),
        Row("touch_slide", "choice", tr("One-finger slide on the touch screen"),
            lambda env: trackpad_nav.load_touch_slide(), lambda env, v: slides(env, touch=v),
            options=slide_opts, default="orbit"),
        Row("trackpad_inertia", "check", tr("Keep moving after the fingers lift (inertia)"),
            lambda env: trackpad_nav.load_inertia(), lambda env, v: slides(env, inertia=v),
            default=True),
    ]
    tools = [
        Row("cursor_style", "choice", tr("Tool cursor"),
            lambda env: _gkey("cursor_style", "tool")(env).lower(),
            lambda env, v: (p.apply_cursor_style(env.win, v, env.st), env.st.sync()),
            options=[("tool", tr("The tool's icon")), ("crosshair", tr("Crosshair")),
                     ("arrow", tr("Arrow"))], default="tool"),
        Row("ask_section", "check", tr("Ask for a name and symbol when placing a section plane"),
            _key("section/ask_name", "1", bool), _set_key("section/ask_name", bool), default=True),
    ]
    return [("mouse", tr("Mouse"), mouse), ("touch", tr("Trackpad and touch screen"), touch),
            ("tools", tr("Tools"), tools)]


def _display(env) -> list:
    from views.document_interaction import EDIT_REST_FADE, SOFT_SHADOW_SPREAD
    p = _p()

    def msaa(env):
        n = _int(env.st.value("display/msaa", 4), 4)
        return n if n in (0, 2, 4, 8) else 4

    def soft_level(env):
        soft = _int(env.st.value("display/soft_shadows", 0), 0)
        if soft in SOFT_SHADOW_SPREAD:
            return soft
        level = _int(env.st.value("display/soft_shadows_level", 2), 2)
        return level if level in SOFT_SHADOW_SPREAD else 2

    def soft_on(env):
        return _int(env.st.value("display/soft_shadows", 0), 0) in SOFT_SHADOW_SPREAD

    def put_soft(env, on=None, level=None):
        p.apply_soft_shadows(env.win, soft_on(env) if on is None else on,
                             soft_level(env) if level is None else int(level), env.st)
        env.st.sync()

    def rest_mode(env):
        return getattr(env.vp, "edit_rest_mode", "fade")

    def rest_fade(env):
        try:
            return int(round(float(getattr(env.vp, "edit_rest_fade", EDIT_REST_FADE)) * 100))
        except (TypeError, ValueError):
            return int(round(EDIT_REST_FADE * 100))

    quality = [
        Row("msaa", "choice", tr("Anti-aliasing (MSAA)"), msaa,
            lambda env, v: (p.apply_msaa(env.win, int(v), env.st), env.st.sync()),
            options=[(n, tr("Off") if n == 0 else f"{n}x") for n in (0, 2, 4, 8)], default=4),
        Row("soft_shadows", "check", tr("Soft sun shadows"), soft_on, lambda env, v: put_soft(env, on=v),
            default=False,
            tip=tr("Shadow edges that soften away from what casts them, as in daylight. They cost the "
                   "graphics card more while the sun casts shadows.")),
        Row("soft_level", "choice", tr("Softness"), soft_level, lambda env, v: put_soft(env, level=v),
            options=[(1, tr("Slight")), (2, tr("Moderate")), (3, tr("Strong"))], default=2,
            enabled=soft_on),
    ]
    editing = [
        Row("rest_mode", "choice", tr("Rest of model while editing"), rest_mode,
            lambda env, v: p.apply_rest_mode(env.win, v, rest_fade(env) / 100.0),
            options=[("normal", tr("Show normally")), ("fade", tr("Fade")), ("hide", tr("Hide (fastest)"))],
            default="fade", words="group component"),
        Row("rest_fade", "slider", tr("Fade intensity"), rest_fade,
            lambda env, v: p.apply_rest_mode(env.win, rest_mode(env), int(v) / 100.0),
            lo=0, hi=100, fmt=lambda v: f"{int(v or 0)} %", default=int(round(EDIT_REST_FADE * 100)),
            enabled=lambda env: rest_mode(env) == "fade",
            tip=tr("How strongly the rest of the model is washed toward grey while a group or "
                   "component is open.")),
    ]
    return [("quality", tr("Quality"), quality), ("editing", tr("Editing a group"), editing)]


#: The six physical axes of the cap, as the dialog names them.
_AXIS_LABELS = ("Move left / right", "Move up / down", "Push / pull", "Tilt forward / back",
                "Twist left / right", "Roll left / right")
_ACTION_LABELS = ("Pan X", "Pan Y", "Zoom", "Orbit X", "Orbit Y", "Disabled")


def _ndof_state(env):
    """(settings, named profiles, the preset shown)."""
    from core.ndof import AXES, DEFAULT_MAPPING
    from views.ndof_input import load_settings
    p = _p()
    nd = load_settings()
    named = p.read_named_ndof_profiles()
    if nd.mapping_preset == "original":
        nd.axis_inversion = p.legacy_ndof_inversion(nd)
    chosen = str(env.st.value("ndof/selected_profile", "") or "")
    if chosen in named:
        return nd, named, "named:" + chosen
    if nd.mapping_preset in ("original", "freecad"):
        return nd, named, nd.mapping_preset
    # Original is stored as Custom (its inversion migrated): shown as what it is.
    if (nd.axis_mapping == DEFAULT_MAPPING and not any(nd.axis_inversion.values())
            and all(abs(nd.axis_sensitivity[a] - 1.0) < 1e-9 for a in AXES)
            and abs(nd.sensitivity - 1.0) < 1e-9):
        return nd, named, "original"
    return nd, named, "custom"


def _ndof_shown(env):
    """:func:`_ndof_state`, read once per build (the axis table asks for it
    eighteen times)."""
    got = env.__dict__.get("_ndof")
    if got is None:
        got = env._ndof = _ndof_state(env)
    return got


def _ndof_put(env, *, preset=None, axis=None, what=None, value=None, **flags):
    """One 3D mouse change, stored at once: a flag (enabled, lock), the
    global speed, a preset, or one axis's action, inversion or speed."""
    from core.ndof import AXES
    p = _p()
    nd, named, selected = _ndof_state(env)
    for k, v in flags.items():
        setattr(nd, k, v)
    if preset is not None:
        values = p.ndof_preset_values(preset, named)
        if values is not None:
            mapping, inversion, speeds, global_speed = values
            nd.sensitivity = global_speed
            for a in AXES:
                nd.axis_mapping[a] = mapping[a]
                nd.axis_inversion[a] = inversion[a]
                nd.axis_sensitivity[a] = speeds[a]
        selected = preset
    elif axis is not None or "sensitivity" in flags:
        if axis is not None:
            getattr(nd, what)[axis] = value
        selected = "custom"             # editing an axis makes it Custom
    p.save_ndof(nd, named, selected, env.st)
    env.st.sync()


def _ndof(env) -> list:
    from core.ndof import ACTIONS, AXES
    from views.ndof_input import load_settings

    def preset(env):
        return _ndof_shown(env)[2]

    def presets(env):
        named = _p().read_named_ndof_profiles()
        return [("original", tr("Original")), ("freecad", tr("FreeCAD-style")), ("custom", tr("Custom"))] + [
            ("named:" + n, n) for n in sorted(named, key=str.casefold)]

    device = [
        Row("ndof_on", "check", tr("Navigate with a 3D mouse (SpaceMouse)"),
            lambda env: load_settings().enabled, lambda env, v: _ndof_put(env, enabled=bool(v)),
            default=True),
        Row("ndof_speed", "number", tr("Speed"), lambda env: int(round(_ndof_shown(env)[0].sensitivity * 100)),
            lambda env, v: _ndof_put(env, sensitivity=int(v) / 100.0), lo=25, hi=400, suffix="%"),
        Row("ndof_lock", "check", tr("Pan and zoom only (no rotation — for drawing in plan)"),
            lambda env: load_settings().lock_rotation, lambda env, v: _ndof_put(env, lock_rotation=bool(v)),
            default=False),
        Row("ndof_driver", "text", tr("Driver"), lambda env: _ndof_driver()),
        Row("ndof_preset", "choice", tr("Axis mapping preset"), preset,
            lambda env, v: _ndof_put(env, preset=v), options=presets, default="original"),
    ]
    axes = []
    actions = [(k, tr(label)) for k, label in zip(ACTIONS, _ACTION_LABELS)]
    for axis, label in zip(AXES, _AXIS_LABELS):
        axes += [
            Row(f"ndof_map_{axis}", "choice", tr(label),
                lambda env, a=axis: _ndof_shown(env)[0].axis_mapping[a],
                lambda env, v, a=axis: _ndof_put(env, axis=a, what="axis_mapping", value=v),
                options=actions, words="axis"),
            Row(f"ndof_inv_{axis}", "check", tr("Invert"),
                lambda env, a=axis: bool(_ndof_shown(env)[0].axis_inversion[a]),
                lambda env, v, a=axis: _ndof_put(env, axis=a, what="axis_inversion", value=bool(v)),
                words=tr(label)),
            Row(f"ndof_speed_{axis}", "number", tr("Speed"),
                lambda env, a=axis: int(round(_ndof_shown(env)[0].axis_sensitivity[a] * 100)),
                lambda env, v, a=axis: _ndof_put(env, axis=a, what="axis_sensitivity", value=int(v) / 100.0),
                lo=25, hi=400, suffix="%", words=tr(label)),
        ]
    return [("device", tr("Device"), device), ("axes", tr("Axes"), axes)]


def _ndof_driver() -> str:
    try:
        from views.ndof_input import shared_input
        name = shared_input().backend_name
    except Exception:  # noqa: BLE001 — a status line, never the sheet
        name = ""
    if name:
        return tr("Device driver found: {name}", name=name)
    return tr("No 3D mouse driver found. On Linux install and start «spacenavd»; on Windows the "
              "3Dconnexion driver is enough. macOS is not supported yet.")


def _import(env) -> list:
    from views.settings_apply import _DXF_UNITS, _OBJ_UNITS
    rows = [
        Row("obj_unit", "choice", tr("Suggested unit for OBJ"), _key("import/obj_unit", "m"),
            _set_key("import/obj_unit"), options=[(k, tr(v)) for k, v in _OBJ_UNITS], default="m"),
        Row("dxf_unit", "choice", tr("Suggested unit for DXF/DWG"), _key("import/dxf_unit", "mm"),
            _set_key("import/dxf_unit"), options=[(k, tr(v)) for k, v in _DXF_UNITS], default="mm"),
        Row("coord_mode", "choice", tr("Coordinate entry"), _key("georef/coord_mode", "geo"),
            _set_key("georef/coord_mode"),
            options=[("geo", tr("Geographic (lat/lon)")), ("utm", tr("UTM WGS84"))], default="geo",
            words="georeference location"),
    ]
    return [("import", tr("Import"), rows)]


def _ai(env) -> list:
    p = _p()

    def providers(env):
        out = [("auto", tr("Auto (by key prefix)"))]
        try:
            from core import ai
            out += [(prov, ai.PROVIDER_INFO[prov][0]) for prov in ai.PROVIDERS]
        except Exception:  # noqa: BLE001 — the row, not the sheet
            pass
        return out

    def put(key, strip=False):
        def f(env, v):
            if isinstance(v, bool):
                v = "1" if v else "0"
            env.st.setValue(key, v.strip() if strip and isinstance(v, str) else v)
            env.st.sync()
            p.reload_ai_panel(env.win)
        return f

    def env_note(env):
        try:
            from plugins.ai_assistant import env_key_note
            return env_key_note(_key("ia/proveedor", "auto")(env), _key("ia/api_key", "")(env).strip())
        except Exception:  # noqa: BLE001 — a note, not the sheet
            return ""

    rows = [
        Row("ai_provider", "choice", tr("Provider"), _key("ia/proveedor", "auto"), put("ia/proveedor"),
            options=providers),
        Row("ai_key", "edit", tr("API key"), _key("ia/api_key", ""), put("ia/api_key"), password=True,
            tip=tr("empty = local AI (Ollama, LM Studio)")),
        Row("ai_key_env", "text", "", env_note, visible=lambda env: bool(env_note(env))),
        Row("ai_model", "edit", tr("Model"), _key("ia/modelo", ""), put("ia/modelo", strip=True),
            tip=tr("model (default per provider)")),
        Row("ai_url", "edit", tr("Local AI URL"), _key("ia/ollama_url", "http://localhost:11434"),
            put("ia/ollama_url", strip=True), words="ollama lm studio"),
        Row("ai_shots", "check", tr("Send viewport screenshots to the model"), _key("ia/capturas", "1", bool),
            put("ia/capturas"), words="capture"),
    ]
    return [("ai", tr("AI assistant"), rows)]


def _engine(env) -> list:
    from core import _native, renderer_settings
    from core.platform_choice import AUTO, WAYLAND, XCB
    p = _p()
    rust_ok = _native.available() is not None
    rows = [
        Row("rust", "check", tr("Rust engine"),
            lambda env: str(_gkey(_native.SETTING, _native.DEFAULT)(env)) == "1",
            lambda env, v: (p.apply_rust_engine(v, env.st), env.st.sync()),
            restart=tr("Rust engine"), enabled=rust_ok,
            default=(_native.DEFAULT == "1") if rust_ok else _NO,
            tip=(tr("Computes the heavy parts (opening big files, hidden lines of the sheets, "
                    "intersections, snapping) with the native engine. Takes effect at the next start.")
                 if rust_ok else tr("This build of Built does not carry the Rust engine."))),
    ]
    if p.graphics_server_applies():
        rows.append(Row("platform", "choice", tr("Graphics server"), _gkey("platform", AUTO),
                        _set_gkey("platform"),
                        options=[(AUTO, tr("Automatic (X11 on KDE Plasma or with a fractional display "
                                           "scale)")), (WAYLAND, tr("Wayland")), (XCB, tr("X11 (XWayland)"))],
                        restart=tr("Graphics server"), default=AUTO,
                        tip=tr("Which display server Qt draws through. Takes effect at the next start.")))

    def status(env):
        try:
            return renderer_settings.status_text()
        except Exception:  # noqa: BLE001
            return ""

    def runtime(env):
        try:
            cap = renderer_settings.capability()
            return f"Python {cap.python_version} / Qt {cap.qt_version}"
        except Exception:  # noqa: BLE001
            return ""
    about = [Row("renderer_status", "text", tr("Available renderer"), status),
             Row("runtime", "text", tr("Runtime"), runtime)]
    return [("engine", tr("Engine"), rows), ("about", tr("This installation"), about)]


# ---- This model ------------------------------------------------------------------
def _scene_changed(env) -> None:
    """The document was edited: dirty, and every view hears of it."""
    vp = env.vp
    notify = getattr(vp, "notify_scene_changed", None)
    if callable(notify):
        notify()
    elif vp is not None and hasattr(vp, "update"):
        vp.update()


def _units_now(env) -> dict:
    from core import units
    return units.model_units_of(env.scene)


def _units_raw(s: dict) -> dict:
    """``s`` as the units panel holds it: every key, defaults spelled out."""
    from core import units
    raw = {"length": s["length"], "precision": s["precision"]}
    for key in ("area", "volume", "volume_precision", "angle", "angle_precision", "decimal"):
        raw[key] = units.unit_setting(key, s)
    if raw["area"] != "auto":
        raw["area_precision"] = units.unit_setting("area_precision", s)
    return raw


def _units_put(env, key, value) -> None:
    """One units row changed: the document's units, at once (and those of
    new documents, when they follow)."""
    from core import units
    from views.settings_apply import _UNITS_FOR_NEW_KEY
    s = _units_now(env)
    if key == "profile":
        if value not in units.PROFILES:
            return
        chosen = units.with_profile(s, value)
    else:
        raw = _units_raw(s)
        if key == "length":
            frac_before = raw["length"] in ("in-frac", "ft-in-frac")
            raw["length"] = value
            if (value in ("in-frac", "ft-in-frac")) != frac_before:
                raw["precision"] = min(4, raw["precision"])
        elif key == "area":
            raw["area_precision"] = units.unit_setting("area_precision", s)
            raw["area"] = value
            if value == "auto":
                raw.pop("area_precision", None)
        else:
            raw[key] = value
        chosen = units.model_units_of({"units": raw})
    if not _p().apply_document_units(env.win, chosen):
        return
    if _b(env.st.value(_UNITS_FOR_NEW_KEY, "1")):
        env.st.setValue(_UNITS_FOR_NEW_KEY, "1")
        units.remember_new_document_units(chosen)


def _units(env) -> list:
    from core import units
    from views.settings_apply import _UNITS_FOR_NEW_KEY

    def g(key):
        return lambda env: units.unit_setting(key, _units_now(env))

    def frac(env):
        return _units_now(env)["length"] in ("in-frac", "ft-in-frac")

    def by_length(kind):
        def opts(env):
            length = _units_now(env)["length"]
            unit = (units.area_unit_for if kind == "area" else units.volume_unit_for)(length)
            first = tr("By the length ({unit})", unit=units.UNITS[unit].symbol)
            choices = units.AREA_CHOICES if kind == "area" else units.VOLUME_CHOICES
            return [("auto", first)] + [(c, units.unit_label(c)) for c in choices]
        return opts

    def for_new_put(env, v):
        env.st.setValue(_UNITS_FOR_NEW_KEY, "1" if v else "0")
        if v:
            units.remember_new_document_units(_units_now(env))
        env.st.sync()

    def preview(env):
        return units.preview_text(_units_now(env)).replace("\n", "   ·   ")

    def ask_put(env, v):
        from core.units import set_ask_at_start
        set_ask_at_start(bool(v))

    def ask_get(env):
        from core.units import ask_at_start
        return ask_at_start()

    shown = [
        Row("u_profile", "choice", tr("Profile"), lambda env: units.profile_of(_units_now(env)),
            lambda env, v: _units_put(env, "profile", v),
            options=[(n, tr(label)) for n, label in units.PROFILE_LABELS.items()], words="metric imperial"),
        Row("u_length", "choice", tr("Length"), g("length"), lambda env, v: _units_put(env, "length", v),
            options=[(c, units.unit_label(c)) for c in units.UNIT_CHOICES]),
        Row("u_precision", "number", tr("Decimals"), g("precision"),
            lambda env, v: _units_put(env, "precision", int(v)), lo=0, hi=6,
            visible=lambda env: not frac(env), words="precision"),
        Row("u_fraction", "choice", tr("Smallest fraction"), g("precision"),
            lambda env, v: _units_put(env, "precision", int(v)),
            options=list(enumerate(units.FRACTION_LABELS)), visible=frac, words="precision inch",
            tip=tr("The smallest fraction of an inch shown")),
        Row("u_area", "choice", tr("Area"), g("area"), lambda env, v: _units_put(env, "area", v),
            options=by_length("area")),
        Row("u_area_precision", "number", tr("Area decimals"), g("area_precision"),
            lambda env, v: _units_put(env, "area_precision", int(v)), lo=0, hi=6,
            enabled=lambda env: units.unit_setting("area", _units_now(env)) != "auto"),
        Row("u_volume", "choice", tr("Volume"), g("volume"), lambda env, v: _units_put(env, "volume", v),
            options=by_length("volume")),
        Row("u_volume_precision", "number", tr("Volume decimals"), g("volume_precision"),
            lambda env, v: _units_put(env, "volume_precision", int(v)), lo=0, hi=6),
        Row("u_angle", "choice", tr("Angle"), g("angle"), lambda env, v: _units_put(env, "angle", v),
            options=[(c, units.unit_label(c)) for c in units.ANGLE_CHOICES]),
        Row("u_angle_precision", "number", tr("Angle decimals"), g("angle_precision"),
            lambda env, v: _units_put(env, "angle_precision", int(v)), lo=0, hi=6),
        Row("u_decimal", "choice", tr("Decimal separator"), g("decimal"),
            lambda env, v: _units_put(env, "decimal", v),
            options=[(".", tr("Point (1.25)")), (",", tr("Comma (1,25)"))],
            tip=tr("How numbers are shown. Typing takes both: «1,20» and «1.20» are the same.")),
        Row("u_preview", "text", tr("Preview"), preview),
    ]
    beyond = [
        Row("u_for_new", "check", tr("Also use for new documents"),
            _key(_UNITS_FOR_NEW_KEY, "1", bool), for_new_put),
        Row("u_ask_start", "check", tr("Ask which units to use at startup"), ask_get, ask_put, default=True),
    ]
    return [("units", tr("Units of this model"), shown), ("unitsnew", tr("New documents"), beyond)]


def _dimension_font(env):
    from views.font_picker import annotation
    chrome_sheet.close(env.win)
    annotation(env.win)


def _dimensions(env) -> list:
    from core import units

    def style(env):
        s = getattr(env.scene, "dimension_style", None)
        return s if isinstance(s, dict) else {}

    def g(key, default):
        return lambda env: style(env).get(key, default)

    def put(key, conv=lambda v: v):
        def f(env, v):
            s = style(env)
            if env.scene is None or s.get(key) == conv(v):
                return
            s[key] = conv(v)
            env.scene.version += 1
            _scene_changed(env)
        return f

    def color(env):
        c = style(env).get("color", [45, 55, 75])
        try:
            return "#%02x%02x%02x" % tuple(int(x) for x in c[:3])
        except (TypeError, ValueError):
            return "#2d374b"

    def to_rgb(hex_):
        h = str(hex_).lstrip("#")
        if len(h) != 6:
            raise ValueError(tr("A colour is #rrggbb."))
        return [int(h[i:i + 2], 16) for i in (0, 2, 4)]

    rows = [
        Row("dim_decimals", "number", tr("Decimals"), lambda env: int(style(env).get("decimals", 2)),
            put("decimals", int), lo=0, hi=4, words="precision"),
        Row("dim_units", "choice", tr("Unit"), g("units", "m"), put("units"),
            options=[(c, units.unit_label(c)) for c in units.UNIT_CHOICES]),
        Row("dim_show_unit", "check", tr("Show unit"), lambda env: bool(style(env).get("show_unit", True)),
            put("show_unit", bool), tip=tr("«1.30» instead of «1.30 m»")),
        Row("dim_family", "button", tr("Font family…"), g("font_family", ""),
            press=_dimension_font),
        Row("dim_font", "number", tr("Font size"), lambda env: int(style(env).get("font_size", 9)),
            put("font_size", int), lo=6, hi=28, suffix="pt", words="text"),
        Row("dim_color", "color", tr("Colour"), color, put("color", to_rgb), words="color"),
        Row("dim_ends", "choice", tr("Ends"), lambda env: str(style(env).get("ends", "arrow") or "arrow"),
            put("ends"), options=[("arrow", tr("Arrows")), ("tick", tr("Oblique ticks")), ("none", tr("None"))],
            words="arrow tick"),
        Row("dim_base_step", "number", tr("Baseline step"),
            lambda env: float(style(env).get("base_step_mm", 8.0)), put("base_step_mm", float),
            lo=1.0, hi=60.0, decimals=1, suffix="mm",
            tip=tr("How far apart the rows of a baseline dimension run sit on paper.")),
    ]
    return [("dimensions", tr("Dimensions"), rows)]


def _location(env) -> list:
    def north(env):
        from core.sun import scene_north_deg
        return round(scene_north_deg(env.scene), 1) if env.scene is not None else 0.0

    def where(env):
        datum = getattr(env.scene, "georef", None)
        if datum is None:
            return tr("No location yet: north turns only the sun.")
        return tr("{lat}, {lon} — the base map and the terrain turn with north.",
                  lat=f"{datum.lat:.6f}", lon=f"{datum.lon:.6f}")

    def has_location(env):
        return getattr(env.scene, "georef", None) is not None

    rows = [
        Row("north", "number", tr("North"), north, _put_north, lo=-180.0, hi=180.0, decimals=1, suffix="°",
            words="sun shadows angle",
            tip=tr("Where true north lies, in degrees clockwise from the green (Y) axis. The shadows "
                   "turn with it; the model does not move.")),
        Row("north_tool", "button", tr("Aim north on screen"), press=_north_tool,
            tip=tr("Tools ▸ North: click a centre, then click toward north.")),
        Row("location", "text", tr("Location"), where),
        Row("add_location", "button", tr("Add location…"), press=_add_location,
            visible=lambda env: not has_location(env), enabled=lambda env: not chrome_sheet.in_shell(),
            words="georeference map site",
            tip=tr("Pick the site on the map: the sun and the base map stand there.")),
    ]
    return [("location", tr("Location & north"), rows)]


def _put_north(env, value) -> None:
    """North through the command the North tool and the Shadows panel use
    (one undo step): typing «30» is one step, not one per key."""
    from core.history import SetNorthCommand
    from core.sun import scene_north_deg
    from tools.north import refresh_north_display
    vp = env.vp
    history = getattr(vp, "history", None)
    if history is None or abs(float(value) - scene_north_deg(vp.scene)) < 1e-9:
        return
    last = SettingsSheet.north_cmd
    if last is not None and history.undo_stack and history.undo_stack[-1] is last:
        history.undo()                  # the step being typed: replaced
    cmd = SetNorthCommand(float(value))
    history.execute(cmd)
    SettingsSheet.north_cmd = cmd
    refresh_north_display(vp)


def _north_tool(env) -> None:
    chrome_sheet.close(env.win)
    activate = getattr(env.win, "_activate_tool", None)
    if callable(activate):
        activate("north")


def _add_location(env) -> None:
    """The tray's Add location (Shadows panel): pick the site on the map;
    the north set before carries over."""
    if chrome_sheet.in_shell():
        return
    from core.sun import DEFAULT_LAT, DEFAULT_LON, scene_north_deg
    from georef.datum import SceneDatum
    from georef.tiles import DEFAULT_SOURCE_ID, PRESETS
    from views.location_dialog import pick_location
    scene = env.scene
    result = pick_location(PRESETS[DEFAULT_SOURCE_ID], DEFAULT_LAT, DEFAULT_LON,
                           env.win if hasattr(env.win, "isWindow") else None)
    if result is None or scene is None:
        return
    lat, lon, _w, _l = result
    scene.georef = SceneDatum(lat, lon, north=scene_north_deg(scene))
    scene.version += 1
    _scene_changed(env)


_BUILDERS = {"general": _general, "navigation": _navigation, "display": _display, "ndof": _ndof,
             "import": _import, "ai": _ai, "engine": _engine, "units": _units,
             "dimensions": _dimensions, "location": _location}


# ---- Keyboard ---------------------------------------------------------------------
def _parse_keys(text: str):
    """The keys typed in the prompt: ``[]`` (none), ``[QKeySequence]``, or
    a reason they cannot be a shortcut (str)."""
    from PySide6.QtCore import Qt
    from PySide6.QtGui import QKeySequence
    from views.shortcuts import reserved_reason
    t = (text or "").strip()
    if not t:
        return []
    seq = QKeySequence.fromString(t, QKeySequence.NativeText)
    if seq.isEmpty():
        seq = QKeySequence.fromString(t, QKeySequence.PortableText)
    if seq.isEmpty() or seq.count() != 1 or seq[0].key() in (Qt.Key_unknown, 0):
        return tr("«{keys}» is not a key combination Built knows (write it like Ctrl+Shift+K).", keys=t)
    why = reserved_reason(seq)
    if why is not None:
        return tr("«{keys}» cannot be a shortcut: {why}.", keys=seq.toString(QKeySequence.NativeText),
                  why=why)
    return [seq]


def _keys_text(seqs) -> str:
    from PySide6.QtGui import QKeySequence
    return ", ".join(s.toString(QKeySequence.NativeText) for s in seqs if not s.isEmpty())


# ---- The sheet ---------------------------------------------------------------------
class SettingsSheet(chrome_sheet.Provider):
    """Settings on the large sheet: sections on the left, the rows of the
    one picked (or every row the search matches) in the middle."""
    id = "settings"
    #: The pending pieces of one change (class-wide, as rows read them).
    figure_custom = False
    note = None                         # (text, tone) shown once on top
    north_cmd = None

    def __init__(self, section="general"):
        self.section = section
        self.query = ""
        self.kb_focus = None            # "sc:<action key>"
        self._actions = None
        self._paths = None
        self._rows: dict = {}
        self._typed: dict = {}          # a number field's text as typed
        self._force_focus = False

    # -- navigation -------------------------------------------------------------
    def goto(self, page=None, shortcut_of=None) -> None:
        if shortcut_of is not None:
            from views.shortcuts import action_key
            self.section = "keyboard"
            self.kb_focus = "sc:" + action_key(shortcut_of)
            self._force_focus = True        # scrolled to, even on an open sheet
        elif page:
            page = _ALIASES.get(str(page), str(page))
            if page in _LABELS:
                self.section = page
        self.query = ""

    def signature(self, ctx):
        scene = getattr(getattr(ctx.win, "viewport", None), "scene", None)
        return (id(scene), getattr(scene, "version", 0))

    # -- the rows -----------------------------------------------------------------
    def groups(self, env, section) -> list:
        """[(group id, caption, [Row])] of a section (not Keyboard)."""
        try:
            return _BUILDERS[section](env)
        except Exception:  # noqa: BLE001 — one broken section never takes the sheet down
            import traceback
            traceback.print_exc()
            return []

    def _form(self, env, rows, gid) -> cf.Form:
        fields = []
        for r in rows:
            if not r.on("visible", env):
                continue
            self._rows[r.id] = r
            fields.append(r.field(env))
        form = cf.Form("", fields, buttons=[], on_change=self._changed, on_button=self._pressed,
                       key=gid)
        for fid, text in list(self._typed.items()):
            f = form.field(fid)
            if f is None:
                continue
            try:
                same = abs(float(text.replace(",", ".")) - float(f.value or 0)) < 1e-9
            except ValueError:
                same = False
            if same:
                form._texts[fid] = text     # «1.» stays as typed while it means 1
            else:
                self._typed.pop(fid, None)
        return form

    def _changed(self, form, fid, value) -> None:
        row = self._rows.get(fid)
        if row is None or row.put is None:
            return
        f = form.field(fid)
        if f is not None and f.kind == "number" and fid in form._texts:
            self._typed[fid] = form._texts[fid]
        self.apply(row, value)

    def apply(self, row, value) -> None:
        """``row`` set to ``value`` at once; a refusal (a file that is no
        picture, a colour that is none) shows on top of the sheet."""
        env = Env(self._win)
        before = row.start(env) if row.start is not None else row.value(env)
        try:
            row.put(env, value)
        except (ValueError, KeyError, OSError) as exc:
            SettingsSheet.note = (str(exc).strip("'\""), "error")
            return
        except Exception as exc:  # noqa: BLE001 — one setting never takes the sheet down
            import traceback
            traceback.print_exc()
            SettingsSheet.note = (tr("{what} could not be changed: {why}", what=row.label or row.id,
                                     why=exc), "error")
            return
        fid = row.id
        if row.restart:
            start = _PENDING.get(fid, (row.restart, before))[1]
            needs = row.needs_restart or (lambda v, s: v != s)
            if needs(value, start):
                _PENDING[fid] = (row.restart, start)
            else:
                _PENDING.pop(fid, None)

    def _pressed(self, form, fid) -> None:
        row = self._rows.get(fid)
        if row is not None and row.press is not None:
            row.press(Env(self._win))

    def build(self, ctx):
        self._win = ctx.win
        env = Env(ctx.win)
        self._rows = {}
        s = chrome_sheet.Sheet(self.id, tr("Settings"), width=1100)
        s.search(tr("Search settings"))
        query = self.query.strip().lower()
        hits = self._hits(env, query) if query else None
        for scope, caption in ((MAC, tr("Built · this Mac")), (MODEL, tr("This model"))):
            s.side_caption(caption)
            for sid, label, sc in SECTIONS:
                if sc != scope:
                    continue
                right = ""
                if hits is not None:
                    n = len(hits.get(sid, ()))
                    right = str(n) if n else ""
                s.side_item(sid, tr(label), on=hits is None and sid == self.section, right=right,
                            dim=hits is not None and not right)
        if hits is not None:
            self._search_pane(s, env, hits)
        else:
            self._section_pane(s, env)
        s.footer([("restore", tr("Restore defaults"))],
                 tr("Changes apply at once") + " · " + tr("esc back to the model"))
        if self.kb_focus and (self.section == "keyboard" or hits is not None):
            s.focus(self.kb_focus, force=self._force_focus)
        self._force_focus = False
        return s

    def _top(self, c) -> None:
        if _PENDING:
            names = ", ".join(label for label, _start in _PENDING.values())
            c.text(tr("Restart Built to apply: {names}.", names=names), tone="warn")
        if SettingsSheet.note:
            text, tone = SettingsSheet.note
            c.text(text, tone=tone)

    def _section_pane(self, s, env) -> None:
        sid = self.section
        scope = next((sc for i, _l, sc in SECTIONS if i == sid), MAC)
        sub = tr(_SUBS.get(sid, ""))
        if scope == MODEL:
            sub = (sub + " " if sub else "") + tr("(Saved in the document, not on this Mac.)")
        c = s.content(tr(_LABELS.get(sid, sid)), sub)
        self._top(c)
        if sid == "keyboard":
            self._keyboard(s, c, env, "")
            return
        for gid, caption, rows in self.groups(env, sid):
            if not any(r.on("visible", env) for r in rows):
                continue
            c.caption(caption)
            c.fields(self._form(env, rows, gid), id=gid)
        if sid == "ndof":
            c.buttons([chrome_sheet.verb("ndof_load", tr("Load profile…")),
                       chrome_sheet.verb("ndof_save", tr("Save profile…")),
                       chrome_sheet.verb("ndof_reset", tr("Reset to defaults"))], align="left")

    def _hits(self, env, query) -> dict:
        """{section: [(group caption, [Row])] or [action]} matching every word."""
        words = query.split()
        out: dict = {}
        for sid, label, _scope in SECTIONS:
            if sid == "keyboard":
                acts = [a for a in self.actions() if all(w in self._action_text(a) for w in words)]
                if acts:
                    out[sid] = acts
                continue
            found = []
            for gid, caption, rows in self.groups(env, sid):
                rows = [r for r in rows if r.kind not in ("text", "caption") and r.on("visible", env)
                        and all(w in r.haystack(env, tr(label), caption) for w in words)]
                if rows:
                    found.append((gid, caption, rows))
            if found:
                out[sid] = found
        return out

    def _search_pane(self, s, env, hits) -> None:
        n = sum(len(rows) for sid, groups in hits.items() if sid != "keyboard" for _g, _c, rows in groups)
        n += len(hits.get("keyboard", ()))
        c = s.content(tr("Results for “{q}”", q=self.query.strip()),
                      tr("{n} matching settings in every section", n=n),
                      empty=tr("No setting matches “{q}”.", q=self.query.strip()))
        self._top(c)
        for sid, label, _scope in SECTIONS:
            if sid not in hits:
                continue
            if sid == "keyboard":
                self._keyboard(s, c, env, self.query.strip().lower(), caption=tr(label))
                continue
            for gid, caption, rows in hits[sid]:
                c.caption(f"{tr(label)} · {caption}" if caption != tr(label) else tr(label))
                c.fields(self._form(env, rows, "q." + gid), id="q." + gid)

    # -- keyboard -------------------------------------------------------------------
    def actions(self) -> list:
        if self._actions is None:
            from views.shortcuts import collect_actions, remember_defaults
            try:
                remember_defaults(self._win)
                self._actions = collect_actions(self._win)
            except Exception:  # noqa: BLE001 — a window with no actions (the shell)
                self._actions = []
        return self._actions

    def paths(self) -> dict:
        if self._paths is None:
            try:
                from views.command_search import menu_paths
                self._paths = menu_paths(self._win)
            except Exception:  # noqa: BLE001
                self._paths = {}
        return self._paths

    def _action_text(self, act) -> str:
        from views.shortcuts import _plain, action_key
        return " ".join((_plain(act.text()), self.paths().get(action_key(act), ""),
                         _keys_text(act.shortcuts()))).lower()

    def _action(self, item):
        from views.shortcuts import action_key
        key = item[3:] if item and item.startswith("sc:") else None
        return next((a for a in self.actions() if action_key(a) == key), None) if key else None

    def _keyboard(self, s, c, env, query, caption="") -> None:
        from views.shortcuts import _plain, action_key
        acts = self.actions()
        if not acts:
            c.text(tr("The keyboard shortcuts are set in the classic app for now."))
            return
        if query:
            words = query.split()
            acts = [a for a in acts if all(w in self._action_text(a) for w in words)]
        if caption:
            c.caption(caption, right=str(len(acts)))
        items = [chrome_sheet.item("sc:" + action_key(a), _plain(a.text()),
                                   sub=self.paths().get(action_key(a), ""), right=_keys_text(a.shortcuts()),
                                   menu=[("change", tr("Change keys…")), ("clear", tr("Clear")),
                                         ("default", tr("Default"))])
                 for a in acts]
        c.rows(items, id="keyboard")
        if not query:
            c.buttons([chrome_sheet.verb("kb_export", tr("Export…")),
                       chrome_sheet.verb("kb_import", tr("Import…")),
                       chrome_sheet.verb("kb_reset_all", tr("Restore all defaults"))], align="left")
        act = self._action(self.kb_focus)
        d = s.detail(width=320, empty=tr("Pick a command to change its keys."))
        if act is None:
            return
        from views.shortcuts import default_shortcuts
        d.heading(_plain(act.text()))
        d.kv([(tr("Menu"), self.paths().get(action_key(act), "") or "—"),
              (tr("Keys"), _keys_text(act.shortcuts()) or tr("none")),
              (tr("Default"), _keys_text(default_shortcuts(act)) or tr("none"))])
        d.buttons([chrome_sheet.verb("kb_clear", tr("Clear")),
                   chrome_sheet.verb("kb_default", tr("Default")),
                   chrome_sheet.verb("kb_change", tr("Change keys…"), key="↩", primary=True)])

    def _assign(self, act, seqs, ask=True) -> None:
        """The panel's ``assign``: give ``act`` these keys, taking them from
        any other command that holds them (after asking) — two commands on
        one key both go dead."""
        from PySide6.QtGui import QKeySequence
        from views.shortcuts import _plain, save_shortcut
        wanted = {q.toString(QKeySequence.PortableText) for q in seqs}
        clash = [a for a in self.actions() if a is not act and wanted
                 & {q.toString(QKeySequence.PortableText) for q in a.shortcuts()}]

        def go():
            for other in clash:
                kept = [q for q in other.shortcuts() if q.toString(QKeySequence.PortableText) not in wanted]
                other.setShortcuts(kept)
                save_shortcut(other, kept)
            act.setShortcuts(seqs)
            save_shortcut(act, seqs)
            SettingsSheet.note = None
            chrome_sheet.refresh(self._win)
        if clash and ask:
            names = ", ".join(_plain(a.text()) for a in clash)
            self._confirm(tr("Keyboard shortcuts"),
                          tr("«{keys}» is already used by: {names}. Give it to «{action}» instead?",
                             keys=_keys_text(seqs), names=names, action=_plain(act.text())),
                          tr("Give it"), go)
            return
        go()

    def _confirm(self, title, text, ok, then) -> None:
        form = cf.Form(title, [cf.Field("t", "text", "", text, full=True)],
                       buttons=[("cancel", tr("Cancel")), ("ok", ok)],
                       on_done=lambda answer, _v: then() if answer == "ok" else None)
        cf.show(self._win, form)

    def _change_keys(self, act) -> None:
        from views.shortcuts import _plain

        def check(form, _answer):
            got = _parse_keys(form.value("keys"))
            return got if isinstance(got, str) else None

        def done(answer, values):
            if answer != "ok":
                return
            got = _parse_keys(values.get("keys", ""))
            if not isinstance(got, str):
                self._assign(act, got)
        form = cf.Form(tr("Keys for «{action}»", action=_plain(act.text())),
                       [cf.Field("keys", "edit", tr("Keys"), _keys_text(act.shortcuts()[:1]), focus=True)],
                       note=tr("Write the keys, like ⌘⇧K or Ctrl+Shift+K. Empty: no keys."),
                       validate=check, on_done=done, width=420)
        cf.show(self._win, form)

    # -- answers ----------------------------------------------------------------------
    def act(self, ctx, verb, args) -> None:
        self._win = ctx.win
        arg = args[0] if args else ""
        if verb == "search":
            self.query = arg
        elif verb == "side" and arg in _LABELS:
            self.section = arg
            SettingsSheet.note = None
            if self.query:
                self.query = ""
                chrome_sheet.open(ctx.win, self, search="")
        elif verb == "focus" and arg.startswith("sc:"):
            self.kb_focus = arg
        elif verb == "open" and arg.startswith("sc:"):
            self.kb_focus = arg
            act = self._action(arg)
            if act is not None:
                self._change_keys(act)
        elif verb == "menu" and len(args) > 1 and arg.startswith("sc:"):
            self.kb_focus = arg
            self._kb_verb("kb_" + args[1])
        elif verb == "verb":
            if arg.startswith("kb_"):
                self._kb_verb(arg)
            elif arg.startswith("ndof_"):
                self._ndof_verb(arg)
            elif arg == "restore":
                self._confirm(tr("Restore defaults"),
                              tr("Puts the settings of Built on this Mac back to how Built comes. The "
                                 "language, the AI Assistant, the keyboard shortcuts and the settings "
                                 "of this model stay as they are."),
                              tr("Restore defaults"), lambda: self.restore_defaults(ctx.win))

    def _kb_verb(self, verb) -> None:
        from views.shortcuts import default_shortcuts
        act = self._action(self.kb_focus)
        if verb == "kb_change" and act is not None:
            self._change_keys(act)
        elif verb == "kb_clear" and act is not None:
            self._assign(act, [], ask=False)
        elif verb == "kb_default" and act is not None:
            self._assign(act, default_shortcuts(act))
        elif verb == "kb_export":
            self._kb_export()
        elif verb == "kb_import":
            self._kb_import()
        elif verb == "kb_reset_all":
            def reset():
                from PySide6.QtCore import QSettings
                from views.shortcuts import _GROUP
                st = QSettings()
                st.remove(_GROUP)
                st.sync()
                for a in self.actions():
                    a.setShortcuts(default_shortcuts(a))
                chrome_sheet.refresh(self._win)
            self._confirm(tr("Keyboard shortcuts"), tr("Put every action back on its factory shortcut?"),
                          tr("Restore all"), reset)

    def _parent(self):
        return self._win if hasattr(self._win, "isWindow") else None

    def _kb_export(self) -> None:
        import json
        from views.filedialogs import file_dialogs
        from views.shortcuts import export_shortcuts
        path, _ = file_dialogs.getSaveFileName(self._parent(), tr("Export shortcuts"),
                                               "ingetrazo-shortcuts.json", tr("Shortcuts (*.json)"))
        if not path:
            return
        try:
            with open(path, "w", encoding="utf-8") as f:
                json.dump(export_shortcuts(self._win), f, indent=1, ensure_ascii=False)
        except OSError as exc:
            SettingsSheet.note = (str(exc), "error")
            return
        SettingsSheet.note = (tr("Shortcuts saved to {path}.", path=path), "ok")

    def _kb_import(self) -> None:
        import json
        from PySide6.QtGui import QKeySequence
        from views.filedialogs import file_dialogs
        from views.shortcuts import _plain, import_shortcuts
        path, _ = file_dialogs.getOpenFileName(self._parent(), tr("Import shortcuts"), "",
                                               tr("Shortcuts (*.json)"))
        if not path:
            return
        try:
            with open(path, encoding="utf-8") as f:
                applied, skipped, clashes = import_shortcuts(self._win, json.load(f))
        except (OSError, ValueError) as exc:
            SettingsSheet.note = (str(exc), "error")
            return
        msg = tr("{n} shortcuts imported.", n=applied)
        if skipped:
            msg += " " + tr("{n} belong to actions this installation does not have (a plugin, say) and "
                            "were left out.", n=skipped)
        if clashes:
            msg += " " + tr("The file gave the same key to more than one command; each stays with the "
                            "first:") + " " + "; ".join(
                tr("{key}: kept for «{kept}», taken from «{dropped}»",
                   key=QKeySequence(k).toString(QKeySequence.NativeText), kept=_plain(kept.text()),
                   dropped=_plain(dropped.text())) for k, kept, dropped in clashes)
        SettingsSheet.note = (msg, "warn" if clashes else "ok")

    def _ndof_verb(self, verb) -> None:
        from pathlib import Path
        p = _p()
        env = Env(self._win)
        if verb == "ndof_reset":
            _ndof_put(env, preset="original", lock_rotation=False)
        elif verb == "ndof_load":
            from core.ndof_profiles import load_profile
            from views.filedialogs import file_dialogs
            path, _ = file_dialogs.getOpenFileName(self._parent(), tr("Load SpaceMouse profile"), "",
                                                   "JSON (*.json)")
            if not path:
                return
            try:
                profile = load_profile(path)
            except (OSError, ValueError, UnicodeError) as exc:
                SettingsSheet.note = (tr("Invalid SpaceMouse profile") + ": " + str(exc), "error")
                return
            name = Path(path).stem
            if name in ("Original", "FreeCAD-style", "Custom"):
                name += " (imported)"
            self._ndof_add(env, name, profile)
        elif verb == "ndof_save":
            def named(name):
                if name is None:
                    return
                name = name.strip()
                if not name or len(name) > 80 or name in ("Original", "FreeCAD-style", "Custom"):
                    SettingsSheet.note = (tr("Choose a different name (1–80 characters)."), "error")
                    chrome_sheet.refresh(self._win)
                    return
                from core.ndof_profiles import profile_from_dict, profile_to_dict
                nd = _ndof_state(Env(self._win))[0]
                profile = profile_from_dict(profile_to_dict(nd))
                if name in p.read_named_ndof_profiles():
                    self._confirm(tr("Replace profile?"),
                                  tr("Replace the existing profile '{name}'?", name=name), tr("Replace"),
                                  lambda: self._ndof_add(Env(self._win), name, profile))
                    return
                self._ndof_add(Env(self._win), name, profile)
            cf.ask_text(self._win, tr("Save SpaceMouse profile"), tr("Profile name:"), done=named)

    def _ndof_add(self, env, name, profile) -> None:
        p = _p()
        nd, named, _selected = _ndof_state(env)
        named[name] = profile
        p.save_ndof(nd, named, "named:" + name, env.st)
        _ndof_put(env, preset="named:" + name)
        chrome_sheet.refresh(self._win)

    # -- restore defaults ---------------------------------------------------------------
    def restore_defaults(self, win) -> None:
        """Every Built setting with a default back to it (the dialog's
        Restore defaults, applied at once). Left alone: the language, the
        AI Assistant, the keyboard shortcuts and this model's settings."""
        self._win = win
        SettingsSheet.figure_custom = False
        for sid, _label, scope in SECTIONS:
            if scope != MAC or sid == "keyboard":
                continue
            for _gid, _caption, rows in self.groups(Env(win), sid):
                for r in rows:
                    if r.default is not _NO and r.put is not None and r.value(Env(win)) != r.default:
                        self.apply(r, r.default)
        chrome_sheet.refresh(win)

    def closed(self, ctx) -> None:
        SettingsSheet.note = None
        SettingsSheet.figure_custom = False
        SettingsSheet.north_cmd = None

def show(win, page=None, shortcut_of=None) -> bool:
    """Settings on the sheet, on ``page`` (a section id such as "units")
    or on the Keyboard section with ``shortcut_of`` (a QAction) focused."""
    cur = chrome_sheet.current(win)
    sheet = cur if isinstance(cur, SettingsSheet) else SettingsSheet()
    sheet._win = win
    sheet.goto(page, shortcut_of)
    focus = sheet.kb_focus if shortcut_of is not None else None
    return chrome_sheet.open(win, sheet, search="" if cur is sheet or page or shortcut_of else None,
                             focus=focus)
