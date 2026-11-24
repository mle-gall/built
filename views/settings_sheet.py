# SPDX-License-Identifier: GPL-3.0-or-later
"""Settings as a large sheet (``views/chrome_sheet.py``): option B of the
2026-11-23 review, the Preferences of the Built window.

Sections on the left, in two groups: **Built · this Mac** (General,
Navigation, Display, 3D mouse, Import and Keyboard),
kept in QSettings, and **This model** (Units, Dimension style, Location &
north), kept in the document and saved with it. A search lists the
matching rows of every section, each under its section's name, so nothing
hides behind one.

The Rust catalogue supplies preference rows, constraints and defaults. Edits
apply through ``views/settings_apply.py``. Changes that require a restart
appear at the top of the sheet.

"""
from __future__ import annotations

import json

from core.i18n import tr
from views import chrome_sheet
from views import chrome_forms as cf

#: The sections, in the side list's order: (id, label, scope).
from ingetrazo_render import PreferencesView, RestartPreferences

MAC, MODEL = "mac", "model"
SECTIONS = PreferencesView.sections()
_PENDING = RestartPreferences()
_NO = object()


# ---- What the rows read and write ----------------------------------------------
class Env:
    """What a row's getter and setter see: the window, its viewport and
    scene, and one QSettings for the whole build or change."""

    def __init__(self, win):
        from core.settings import settings as QSettings
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
                 fmt=None, press=None, restart_value=None, start=None):
        self.id, self.kind, self.label = rid, kind, label
        self.get, self.put, self.press = get, put, press
        self.options, self.lo, self.hi, self.decimals = options, lo, hi, decimals
        self.suffix, self.tip, self.restart, self.default = suffix, tip, restart, default
        self.enabled, self.visible, self.row_label, self.words = enabled, visible, row_label, words
        self.pick, self.filters, self.password, self.fmt = pick, filters, password, fmt
        self.restart_value = restart_value
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
class Binding:
    """Read and apply one Rust-defined preference using its owning subsystem."""
    def __init__(self, key, read, write=None, *, fmt=None, start=None, restart_value=None, **host):
        self.key, self.read, self.write = key, read, write
        self.fmt, self.start, self.restart_value = fmt, start, restart_value
        self.host = host

    def snapshot(self, env):
        data = {"value": self.read(env)}
        for key, item in self.host.items():
            value = item(env) if callable(item) else item
            if value is not _NO:
                data[key] = value
        return data


def _pref_page(env, section, bindings):
    from ingetrazo_render import PreferencePage
    snapshot = {b.key: b.snapshot(env) for b in bindings}
    page = PreferencePage(section, json.dumps(snapshot), tr)
    handlers = {b.key: b for b in bindings}
    def put(env, key, value):
        handlers[key].write(env, json.loads(page.edit(key, json.dumps(value))))
    groups = _native_groups(page.groups(), put)
    for _, _, rows in groups:
        for row in rows:
            binding = handlers.get(row.id, row)
            row.fmt, row.start, row.restart_value = binding.fmt, binding.start, binding.restart_value
    return groups


def _ai(env):
    from core import ai
    from ingetrazo_render import edit_ai_connection

    def binding(key, default="", **meta):
        return Binding(key, _key(key, default),
                       lambda env, v: edit_ai_connection(env.st, key, v, ai.detect_provider), **meta)

    options = [("auto", tr("Auto (by key prefix)"))] + [(k, ai.PROVIDER_INFO[k][0]) for k in ai.PROVIDERS]
    return _pref_page(env, "ai", [binding("ia/proveedor", "auto", options=options),
                                  binding("ia/modelo"), binding("ia/api_key"),
                                  binding("ia/ollama_url", "http://localhost:11434")])


def _general(env) -> list:
    from core.i18n import LANGUAGE_NAMES, SYSTEM, available_languages
    p = _p()

    def selected_language(value):
        from core.i18n import resolve_language
        return resolve_language(value)

    def running_language(env):
        from core.i18n import current_language
        return current_language()

    look = [
        Binding("language", _key("language", SYSTEM),
            lambda env, v: p.apply_language(v, env.st),
            options=[(SYSTEM, tr("Same as the system"))]
            + [(c, LANGUAGE_NAMES.get(c, c)) for c in available_languages()],
            restart_value=selected_language,
            start=running_language),
        Binding("hover_picture", _key("tabs/hover_picture", "1", bool),
            _set_key("tabs/hover_picture", bool)),
    ]

    def autosave_put(env, on=None, minutes=None, backup=None):
        g = _gkey
        on = g("autosave", "1", bool)(env) if on is None else on
        minutes = g("autosave_min", 5, int)(env) if minutes is None else minutes
        p.apply_autosave(env.win, on, minutes, backup, env.st)
        env.st.sync()

    files = [
        Binding("autosave", _gkey("autosave", "1", bool),
            lambda env, v: autosave_put(env, on=v)),
        Binding("autosave_min", _gkey("autosave_min", 5, int),
            lambda env, v: autosave_put(env, minutes=v)),
        Binding("backup", _gkey("backup", "1", bool),
            lambda env, v: autosave_put(env, backup=v)),
        Binding("undo_steps", _gkey("undo_steps", 200, int),
            lambda env, v: (p.apply_undo_steps(env.win, v, env.st), env.st.sync())),
    ]
    from core.update_check import KEY_AUTO, store_managed
    if not store_managed():
        files.append(Binding("check_updates", _key(KEY_AUTO, "1", bool),
                         _set_key(KEY_AUTO, bool)))
    return _pref_page(env, "general", look + files + _figure_rows(env))


def _figure_rows(env) -> list:
    from core import scale_figure as sf

    def put_pick(env, v):
        if v == sf.CUSTOM:
            from pathlib import Path
            from views.filedialogs import file_dialogs
            saved = str(env.st.value(sf.KEY_IMAGE, "") or "")
            if not saved or not Path(saved).is_file():
                path, _ = file_dialogs.getOpenFileName(env.win, tr("My own picture…"), "",
                                                       tr("Images") + " (*.png *.webp)")
                if path:
                    put_image(env, path)
                return
        env.st.setValue(sf.KEY_PICK, v)
        env.st.sync()

    def put_image(env, path):
        from pathlib import Path
        from ingetrazo_render import raster_dimensions
        if not path:
            return
        if not raster_dimensions(path):
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
        if not img.hasAlphaChannel():
            env.state.note = (tr("This picture has no transparent background, so it will show as "
                                     "a rectangle. A PNG cut out around the person looks best."), "warn")

    def height(env):
        try:
            return float(env.st.value(sf.KEY_HEIGHT, 1.70))
        except (TypeError, ValueError):
            return 1.70

    on = _key("new_document/scale_figure", "1", bool)
    return [
        Binding("scale_figure", on,
            _set_key("new_document/scale_figure", bool)),
        Binding("figure", _figure_pick,
            put_pick,
            options=lambda env: [(p["key"], f"{tr(p['name'])} ({float(p['height']):.2f} m)")
                                 for p in sf.people()] + [(sf.CUSTOM, tr("My own picture…"))]),
        Binding("figure_image", lambda env: str(env.st.value(sf.KEY_IMAGE, "") or ""),
            put_image,
            filters=tr("Images") + " (*.png *.webp);;" + tr("All files") + " (*)"),
        Binding("figure_height", height,
            lambda env, v: (env.st.setValue(sf.KEY_HEIGHT, round(float(v), 3)), env.st.sync())),
        Binding("figure_name", lambda env: str(env.st.value(sf.KEY_NAME, "") or ""),
            lambda env, v: (env.st.setValue(sf.KEY_NAME, v.strip()), env.st.sync())),
    ]


def _figure_pick(env):
    from core import scale_figure as sf
    return str(env.st.value(sf.KEY_PICK, sf.DEFAULT_PICK) or sf.DEFAULT_PICK)


def _navigation(env) -> list:
    from tools.walkthrough import look_sensitivity
    from ingetrazo_render import trackpad_nav
    p = _p()

    mouse = [
        Binding("invert_wheel", _key("nav/invert_wheel", "0", bool),
            lambda env, v: (p.apply_invert_wheel(env.win, v, env.st), env.st.sync())),
        Binding("invert_orbit", _key("nav/invert_orbit_y", "0", bool),
            lambda env, v: (p.apply_invert_orbit(env.win, v, env.st), env.st.sync())),
        Binding("look_sens", lambda env: look_sensitivity(),
            _set_key("walk/look_sensitivity")),
    ]
    touch = [
        Binding("touch_slide", _key("nav/touch_slide", "orbit"), _set_key("nav/touch_slide")),
        Binding("trackpad_inertia", _key("nav/trackpad_inertia", "1", bool), _set_key("nav/trackpad_inertia", bool)),
        Binding("pinch_sensitivity", _key("nav/pinch_sensitivity", 100, int), _set_key("nav/pinch_sensitivity")),
        Binding("trackpad_slide", lambda env: trackpad_nav.load_slide_mode(),
            lambda env, v: (p.apply_slides(env.win, v, env.st), env.st.sync())),
    ]
    tools = [
        Binding("cursor_style", lambda env: _gkey("cursor_style", "tool")(env).lower(),
            lambda env, v: (p.apply_cursor_style(env.win, v, env.st), env.st.sync())),
        Binding("ask_section", _key("section/ask_name", "1", bool),
            _set_key("section/ask_name", bool)),
    ]
    return _pref_page(env, "navigation", mouse + touch + tools)


def _display(env) -> list:
    from views.document_interaction import EDIT_REST_FADE, SOFT_SHADOW_SPREAD
    p = _p()

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

    from ingetrazo_render import msaa_levels, apply_msaa
    quality = [
        Binding("msaa", _key("display/msaa", 4, int), apply_msaa, levels=msaa_levels),
        Binding("soft_shadows", soft_on,
            lambda env, v: put_soft(env, on=v)),
        Binding("soft_level", soft_level,
            lambda env, v: put_soft(env, level=v)),
    ]
    editing = [
        Binding("rest_mode", rest_mode,
            lambda env, v: p.apply_rest_mode(env.win, v, rest_fade(env) / 100.0)),
        Binding("rest_fade", rest_fade,
            lambda env, v: p.apply_rest_mode(env.win, rest_mode(env), int(v) / 100.0),
            fmt=lambda v: f"{int(v or 0)} %"),
    ]
    return _pref_page(env, "display", quality + editing)


def _device_preferences(env, named=None):
    from core.ndof_profiles import profile_to_dict
    from ingetrazo_render import DevicePreferences
    from views.ndof_input import load_settings
    nd = load_settings()
    profile = profile_to_dict(nd)
    if nd.mapping_preset == "original":
        profile["inversion"] = _p().legacy_ndof_inversion(nd)
    if named is None:
        named = _p().read_named_ndof_profiles()
    model = DevicePreferences(json.dumps(profile), json.dumps({k: profile_to_dict(v) for k, v in named.items()}),
                              nd.mapping_preset, str(env.st.value("ndof/selected_profile", "") or ""),
                              nd.enabled, nd.lock_rotation)
    return model, named


def _device_edit(env, edits, named=None):
    from core.ndof_profiles import profile_from_dict
    model, named = _device_preferences(env, named)
    for key, value in edits.items():
        model.edit(key, json.dumps(value))
    nd = profile_from_dict(json.loads(model.profile()))
    nd.enabled, nd.lock_rotation = model.enabled, model.locked
    _p().save_ndof(nd, named, model.selected, env.st)
    env.st.sync()


def _native_groups(spec, put):
    """Bind the sheet toolkit's field callbacks to rows returned by Rust."""
    def bind(data):
        key, kind = data.pop("id"), data.pop("kind")
        value = data.pop("value")
        return Row(key, kind, data.pop("label"), lambda env: value,
                   None if kind == "text" else lambda env, v: put(env, key, v), **data)
    return [(gid, title, [bind(data) for data in rows]) for gid, title, rows in json.loads(spec)]


def _ndof(env):
    from views.ndof_input import shared_input
    model, _ = _device_preferences(env)
    driver = shared_input().backend_name or ""
    live = env.device_readings.text(tr) if getattr(env, "device_readings", None) else ("—", "—")
    return _native_groups(model.groups(tr, driver, live), lambda env, key, v: _device_edit(env, {key: v}))


def _import(env) -> list:
    rows = [
        Binding("dxf_unit", _key("import/dxf_unit", "mm"),
            _set_key("import/dxf_unit")),
    ]
    return _pref_page(env, "import", rows)



# ---- This model ------------------------------------------------------------------
def _scene_changed(env) -> None:
    """The document was edited: dirty, and every view hears of it."""
    vp = env.vp
    notify = getattr(vp, "notify_scene_changed", None)
    if callable(notify):
        notify()
    elif vp is not None and hasattr(vp, "update"):
        vp.update()


def _units(env) -> list:
    import json
    from core import units
    from views import units_question
    from views.settings_apply import _UNITS_FOR_NEW_KEY, apply_document_units

    def current(env):
        model = units_question.draft(env.scene)
        model.edit_document(_b(env.st.value(_UNITS_FOR_NEW_KEY, "1")), units.ask_at_start())
        return model

    def put(env, key, value):
        model = current(env)
        model.receive(key, str(int(value)))
        if key == "ask_again":
            units.set_ask_at_start(model.ask_again)
            return
        chosen = json.loads(model.settings())
        if key == "for_new":
            env.st.setValue(_UNITS_FOR_NEW_KEY, "1" if model.for_new else "0")
            env.st.sync()
        elif not apply_document_units(env.win, chosen):
            return
        if model.for_new:
            units.remember_new_document_units(chosen)

    shown, beyond = [], []
    for key, kind, label, text, value, _, _, options, flags, tip in units_question.model_rows(current(env)):
        rid = "u_ask_start" if key == "ask_again" else "u_" + key
        val = text if kind == "text" else int(value)
        row = Row(rid, kind, label, lambda env, v=val: v,
                  None if kind == "text" else lambda env, v, k=key: put(env, k, v),
                  options=list(enumerate(options)), enabled=not flags & cf.DISABLED, tip=tip)
        (beyond if key in ("for_new", "ask_again") else shown).append(row)
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
            from core.protocol.commands import set_dimension_style
            if set_dimension_style(_p().document_history(env.vp), key, conv(v))["ok"]:
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
        Row("add_location", "button", tr("Edit location…") if has_location(env) else tr("Add location…"),
            press=lambda env: edit_location(env.win), enabled=env.scene is not None,
            words="georeference map site",
            tip=tr("Pick the site on the map: the sun and the base map stand there.")),
    ]
    return [("location", tr("Location & north"), rows)]


def _put_north(env, value) -> None:
    """North through the command the North tool and the Shadows panel use
    (one undo step): typing «30» is one step, not one per key."""
    from core.protocol.commands import run
    from core.sun import scene_north_deg
    from tools.north import refresh_north_display
    vp = env.vp
    history = getattr(vp, "history", None)
    if history is None or abs(float(value) - scene_north_deg(vp.scene)) < 1e-9:
        return
    last = SettingsSheet.north_cmd
    if last is not None and history.undo_stack and history.undo_stack[-1] is last:
        history.undo()                  # the step being typed: replaced
    changed = run(history, "set_north", degrees=float(value))
    SettingsSheet.north_cmd = history.undo_stack[-1] if changed["ok"] else None
    refresh_north_display(vp)


def _north_tool(env) -> None:
    chrome_sheet.close(env.win)
    activate = getattr(env.win, "_activate_tool", None)
    if callable(activate):
        activate("north")


def edit_location(win, *, source=None, center=None, done=None):
    from georef.tiles import DEFAULT_SOURCE_ID, PRESETS
    from core.sun import DEFAULT_LAT, DEFAULT_LON
    from ingetrazo_core import LocationDraft
    show(win, page="location")
    sheet = chrome_sheet.current(win)
    scene = win.viewport.scene
    datum = scene.georef
    lat, lon = center or ((datum.lat, datum.lon) if datum else (DEFAULT_LAT, DEFAULT_LON))
    sheet.end_site()
    sheet.site = LocationDraft(lat, lon, Env(win).st.value("georef/coord_mode", "geo"))
    sheet.site_scene = scene
    sheet.site_done = done
    sheet.site_source = source or PRESETS[DEFAULT_SOURCE_ID]
    chrome_sheet.refresh(win)


_BUILDERS = {"ai": _ai, "general": _general, "navigation": _navigation, "display": _display, "ndof": _ndof,
             "import": _import, "units": _units,
             "dimensions": _dimensions, "location": _location}


# ---- Keyboard ---------------------------------------------------------------------
def _parse_keys(text: str):
    from ingetrazo_render import shortcut_text
    t = (text or "").strip(" \t\r\n")
    if not t:
        return []
    seq = shortcut_text(t)
    if seq is None or seq[2] is None:
        return tr("«{keys}» is not a key combination Built knows (write it like Ctrl+Shift+K).", keys=t)
    if seq[3]:
        return tr("«{keys}» cannot be a shortcut: {why}.", keys=seq[1], why=tr(seq[3]))
    return [seq[0]]


def _keys_text(keys):
    from ingetrazo_render import shortcut_text
    return ", ".join(seq[1] for text in keys if (seq := shortcut_text(text)))


# ---- The sheet ---------------------------------------------------------------------
class SettingsSheet(chrome_sheet.Provider):
    """Settings on the large sheet: sections on the left, the rows of the
    one picked (or every row the search matches) in the middle."""
    id = "settings"
    #: The pending pieces of one change (class-wide, as rows read them).
    north_cmd = None

    def __init__(self, section="general"):
        self.site = self.site_requests = None
        self._device = None
        self._readings = None
        self.state = PreferencesView(section)
        self._actions = None
        self._paths = None
        self._rows: dict = {}

    # -- navigation -------------------------------------------------------------
    @property
    def section(self):
        return self.state.section

    @property
    def kb_focus(self):
        return self.state.kb_focus

    def goto(self, page=None, shortcut_of=None) -> None:
        from core.shortcuts import action_key
        self.state.goto(page or "", action_key(shortcut_of) if shortcut_of is not None else None)

    def _monitor(self, active):
        from views.ndof_input import shared_input
        from ingetrazo_render import DeviceReadings
        if active and self._device is None:
            dev = shared_input()
            if dev.backend_name:
                self._readings = DeviceReadings()
                self._device = dev
                dev.raw.connect(self._raw_sample)
                dev.motion.connect(self._motion_sample)
        elif not active and self._device is not None:
            self._device.raw.disconnect(self._raw_sample)
            self._device.motion.disconnect(self._motion_sample)
            self._device = self._readings = None

    def _raw_sample(self, values):
        if self._readings is not None:
            self._readings.raw(values)
            chrome_sheet.refresh(self._win)

    def _motion_sample(self, sample, _dt):
        if self._readings is not None:
            self._readings.motion((sample.right, sample.up, sample.forward, sample.tilt, sample.spin))
            chrome_sheet.refresh(self._win)

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
        for field in fields:
            if field.kind == "number":
                text = self.state.number_text(field.id, float(field.value or 0))
                if text is not None:
                    form._texts[field.id] = text
        return form

    def _changed(self, form, fid, value) -> None:
        row = self._rows.get(fid)
        if row is None or row.put is None:
            return
        f = form.field(fid)
        if f is not None and f.kind == "number" and fid in form._texts:
            self.state.type_number(fid, form._texts[fid])
        self.apply(row, value)

    def apply(self, row, value) -> None:
        """``row`` set to ``value`` at once; a refusal (a file that is no
        picture, a colour that is none) shows on top of the sheet."""
        env = Env(self._win)
        env.state = self.state
        before = row.start(env) if row.start is not None else row.value(env)
        try:
            row.put(env, value)
        except (ValueError, KeyError, OSError) as exc:
            self.state.note = (str(exc).strip("'\""), "error")
            return
        except Exception as exc:  # noqa: BLE001 — one setting never takes the sheet down
            import traceback
            traceback.print_exc()
            self.state.note = (tr("{what} could not be changed: {why}", what=row.label or row.id,
                                     why=exc), "error")
            return
        fid = row.id
        if row.restart:
            value = row.restart_value(value) if row.restart_value else value
            _PENDING.record(fid, row.restart, json.dumps(before), json.dumps(value))

    def _pressed(self, form, fid) -> None:
        row = self._rows.get(fid)
        if row is not None and row.press is not None:
            row.press(Env(self._win))

    def build(self, ctx):
        self._win = ctx.win
        env = Env(ctx.win)
        env.state = self.state
        if self.site is not None and (env.scene is not self.site_scene or self.section != "location"):
            self.end_site()
        self._monitor(self.section == "ndof" and not self.state.query.strip())
        env.device_readings = self._readings
        self._rows = {}
        s = chrome_sheet.Sheet(self.id, tr("Settings"), width=1100)
        s.search(tr("Search settings"))
        query = self.state.query.strip().lower()
        keyboard = self._keyboard_snapshot() if query or self.section == "keyboard" else None
        hits = self._hits(env, query, keyboard["count"]) if query else None
        for scope, caption in ((MAC, tr("Built · this Mac")), (MODEL, tr("This model"))):
            s.side_caption(caption)
            for sid, label, sc in SECTIONS:
                if sc != scope:
                    continue
                right = ""
                if hits is not None:
                    n = hits.get(sid, 0) if sid == "keyboard" else len(hits.get(sid, ()))
                    right = str(n) if n else ""
                s.side_item(sid, tr(label), on=hits is None and sid == self.section, right=right,
                            dim=hits is not None and not right)
        if hits is not None:
            self._search_pane(s, env, hits, keyboard)
        else:
            self._section_pane(s, env, keyboard)
        if self.site is None:
            s.footer([("restore", tr("Restore defaults"))],
                     tr("Changes apply at once") + " · " + tr("esc back to the model"))
        else:
            s.footer(hint=tr("esc back to the model"))
        if self.kb_focus and (self.section == "keyboard" or hits is not None):
            s.focus(self.kb_focus, force=self.state.take_focus())
        return s

    def _top(self, c) -> None:
        names = ", ".join(_PENDING.labels())
        if names:
            c.text(tr("Restart Built to apply: {names}.", names=names), tone="warn")
        if self.state.note:
            text, tone = self.state.note
            c.text(text, tone=tone)

    def _section_pane(self, s, env, keyboard) -> None:
        sid = self.section
        title, scope, subtitle = self.state.heading()
        sub = tr(subtitle)
        if scope == MODEL:
            sub = (sub + " " if sub else "") + tr("(Saved in the document, not on this Mac.)")
        c = s.content(tr(title), sub)
        self._top(c)
        if sid == "location" and self.site is not None:
            for gid, caption, rows in _native_groups(self.site.groups(tr), self.site_edit):
                c.caption(caption)
                c.fields(self._form(env, rows, gid), id=gid)
            src = self.site_source
            root = str(self.site_requests.tiles.cache.root) if self.site_requests else ""
            c.blocks.extend(json.loads(self.site.map_blocks(tr, src.id, root, src.max_zoom, src.attribution)))
            c.buttons(json.loads(self.site.buttons(tr)), align="left")
            return
        if sid == "keyboard":
            self._keyboard(s, c, keyboard)
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

    def site_edit(self, env, key, value):
        if self.site is not None and env.scene is self.site_scene:
            if self.site.edit(key, str(value)):
                env.st.setValue("georef/coord_mode", self.site.mode)

    def end_site(self):
        if self.site_requests is not None:
            self.site_requests.close()
            self.site_requests = None
        self.site = self.site_scene = self.site_done = self.site_source = None

    def site_action(self, ctx, action, args=()):
        site = self.site
        env = Env(ctx.win)
        if site is None or env.scene is not self.site_scene or action == "site_cancel":
            self.end_site()
            return
        if action == "site_map":
            site.map_toggle()
            if site.map and self.site_requests is None:
                from views.location_backend import SiteRequests
                self.site_requests = SiteRequests(site, self.site_source, ctx.refresh)
            elif not site.map and self.site_requests is not None:
                self.site_requests.close()
                self.site_requests = None
            return
        if site.map and self.site_requests is not None:
            if action == "site_tiles":
                self.site_requests.request(args)
            elif action == "site_center":
                site.move_pin(float(args[0]), float(args[1]), int(args[2]))
            elif action == "site_rect":
                site.bounds(list(map(float, args)))
            elif action == "site_search":
                self.site_requests.searcher.search(site.query)
            elif action == "site_locate":
                self.site_requests.locator.locate()
            elif site.choose(action):
                return
        if action != "site_apply" or site.selected() is None:
            return
        lat, lon, width, length = site.selected()
        if self.site_done is not None:
            done = self.site_done
            self.end_site()
            chrome_sheet.close(ctx.win)
            done(lat, lon, width, length)
            return
        datum = env.scene.georef
        if datum is not None and not site.moved(datum.lat, datum.lon):
            self.end_site()
            return
        if datum is not None:
            from views.chrome_dialogs import ask
            message = tr("This project already has a location. Moving it makes "
                         "the new point the model's origin (0,0): everything "
                         "drawn keeps its local coordinates and shows up at the "
                         "new spot on the map.\n\nMove the origin?")
            if ask(ctx.win, tr("Move the project origin?"), message,
                   [("cancel", tr("Cancel")), ("move", tr("Move"))], default="cancel", cancel="cancel") != "move":
                return
        if ctx.win.viewport.scene is not self.site_scene:
            self.end_site()
            return
        from core.protocol.commands import set_location
        result = set_location(_p().document_history(env.vp), lat, lon)
        if result["ok"]:
            self.end_site()
            _scene_changed(env)
        else:
            self.state.note = (result["error"] or result["rejected"] or tr("Location could not be changed"), "error")

    def _hits(self, env, query, keyboard_count) -> dict:
        """{section: [(group caption, [Row])] or keyboard count} matching every word."""
        words = query.split()
        out: dict = {}
        for sid, label, _scope in SECTIONS:
            if sid == "keyboard":
                if keyboard_count:
                    out[sid] = keyboard_count
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

    def _search_pane(self, s, env, hits, keyboard) -> None:
        n = sum(len(rows) for sid, groups in hits.items() if sid != "keyboard" for _g, _c, rows in groups)
        n += hits.get("keyboard", 0)
        c = s.content(tr("Results for “{q}”", q=self.state.query.strip()),
                      tr("{n} matching settings in every section", n=n),
                      empty=tr("No setting matches “{q}”.", q=self.state.query.strip()))
        self._top(c)
        for sid, label, _scope in SECTIONS:
            if sid not in hits:
                continue
            if sid == "keyboard":
                self._keyboard(s, c, keyboard)
                continue
            for gid, caption, rows in hits[sid]:
                c.caption(f"{tr(label)} · {caption}" if caption != tr(label) else tr(label))
                c.fields(self._form(env, rows, "q." + gid), id="q." + gid)

    # -- keyboard -------------------------------------------------------------------
    def actions(self) -> list:
        if self._actions is None:
            from core.shortcuts import collect_actions, remember_defaults
            try:
                if sync := getattr(type(self._win), "refresh_commands", None):
                    sync(self._win)
                remember_defaults(self._win)
                self._actions = collect_actions(self._win)
            except Exception:  # noqa: BLE001 — a window with no actions (the shell)
                self._actions = []
        return self._actions

    def paths(self) -> dict:
        if self._paths is None:
            try:
                from views.command_metadata import menu_paths
                self._paths = menu_paths(self._win)
            except Exception:  # noqa: BLE001
                self._paths = {}
        return self._paths

    def _keyboard_snapshot(self):
        from core.shortcuts import _plain, action_key, action_keys, default_keys
        paths = self.paths()
        actions = []
        for act in self.actions():
            key = action_key(act)
            actions.append((key, _plain(act.text()), paths.get(key, ""),
                            _keys_text(action_keys(act)), _keys_text(default_keys(act))))
        return json.loads(self.state.keyboard(actions, tr))

    def _action(self, item):
        from core.shortcuts import action_key
        key = item[3:] if item and item.startswith("sc:") else None
        return next((a for a in self.actions() if action_key(a) == key), None) if key else None

    def _keyboard(self, sheet, content, keyboard):
        content.blocks.extend(keyboard["content"])
        detail = sheet.detail(width=320, empty=tr("Pick a command to change its keys."))
        detail.blocks.extend(keyboard["detail"])

    def _assign(self, act, seqs, ask=True) -> None:
        """The panel's ``assign``: give ``act`` these keys, taking them from
        any other command that holds them (after asking) — two commands on
        one key both go dead."""
        from ingetrazo_render import shortcut_conflicts
        from core.shortcuts import _plain, action_key, action_keys, portable_keys, save_shortcut, set_action_keys
        actions = {action_key(a): a for a in self.actions()}
        seqs = portable_keys(seqs)
        clash = shortcut_conflicts(action_key(act), set(seqs),
                                   [(k, action_keys(a)) for k, a in actions.items()])

        def go():
            for key, remaining in clash:
                set_action_keys(actions[key], remaining)
                save_shortcut(actions[key], remaining)
            set_action_keys(act, seqs)
            save_shortcut(act, seqs)
            self.state.note = None
            chrome_sheet.refresh(self._win)
        if clash and ask:
            names = ", ".join(_plain(actions[k].text()) for k, _ in clash)
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
        from core.shortcuts import _plain, action_keys

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
                       [cf.Field("keys", "edit", tr("Keys"), _keys_text(action_keys(act)[:1]), focus=True)],
                       note=tr("Write the keys, like ⌘⇧K or Ctrl+Shift+K. Empty: no keys."),
                       validate=check, on_done=done, width=420)
        cf.show(self._win, form)

    # -- answers ----------------------------------------------------------------------
    def act(self, ctx, verb, args) -> None:
        self._win = ctx.win
        arg = args[0] if args else ""
        if verb == "search":
            self.state.query = arg
        elif verb in ("site_center", "site_rect", "site_tiles"):
            self.site_action(ctx, verb, args)
        elif verb == "side":
            searched = bool(self.state.query)
            self.state.goto(arg, None)
            if searched:
                chrome_sheet.open(ctx.win, self, search="")
        elif verb == "focus" and arg.startswith("sc:"):
            self.state.kb_focus = arg
        elif verb == "open" and arg.startswith("sc:"):
            self.state.kb_focus = arg
            act = self._action(arg)
            if act is not None:
                self._change_keys(act)
        elif verb == "menu" and len(args) > 1 and arg.startswith("sc:"):
            self.state.kb_focus = arg
            self._kb_verb("kb_" + args[1])
        elif verb == "verb":
            if arg.startswith("site_"):
                self.site_action(ctx, arg)
            elif arg.startswith("kb_"):
                self._kb_verb(arg)
            elif arg.startswith("ndof_"):
                self._ndof_verb(arg)
            elif arg == "restore":
                self._confirm(tr("Restore defaults"),
                              tr("Restore default application settings? This keeps your language, "
                                 "keyboard shortcuts and model settings."),
                              tr("Restore defaults"), lambda: self.restore_defaults(ctx.win))

    def _kb_verb(self, verb) -> None:
        from core.shortcuts import default_keys, set_action_keys
        act = self._action(self.kb_focus)
        if verb == "kb_change" and act is not None:
            self._change_keys(act)
        elif verb == "kb_clear" and act is not None:
            self._assign(act, [], ask=False)
        elif verb == "kb_default" and act is not None:
            self._assign(act, default_keys(act))
        elif verb == "kb_export":
            self._kb_export()
        elif verb == "kb_import":
            self._kb_import()
        elif verb == "kb_reset_all":
            def reset():
                from core.settings import settings as QSettings
                from app_shell.windows import hosts
                from core.shortcuts import _GROUP
                st = QSettings()
                st.remove(_GROUP)
                st.sync()
                for action in self.actions():
                    set_action_keys(action, default_keys(action))
                for host in hosts():
                    if host.window is not self._win:
                        for action in host.window._registry_actions.values():
                            set_action_keys(action, default_keys(action))
                chrome_sheet.refresh(self._win)
            self._confirm(tr("Keyboard shortcuts"), tr("Put every action back on its factory shortcut?"),
                          tr("Restore all"), reset)

    def _parent(self):
        return self._win if hasattr(self._win, "isWindow") else None

    def _kb_export(self) -> None:
        import json
        from views.filedialogs import file_dialogs
        from core.shortcuts import export_shortcuts
        path, _ = file_dialogs.getSaveFileName(self._parent(), tr("Export shortcuts"),
                                               "ingetrazo-shortcuts.json", tr("Shortcuts (*.json)"))
        if not path:
            return
        try:
            with open(path, "w", encoding="utf-8") as f:
                json.dump(export_shortcuts(self._win), f, indent=1, ensure_ascii=False)
        except OSError as exc:
            self.state.note = (str(exc), "error")
            return
        self.state.note = (tr("Shortcuts saved to {path}.", path=path), "ok")

    def _kb_import(self) -> None:
        import json
        from views.filedialogs import file_dialogs
        from core.shortcuts import _plain, import_shortcuts
        path, _ = file_dialogs.getOpenFileName(self._parent(), tr("Import shortcuts"), "",
                                               tr("Shortcuts (*.json)"))
        if not path:
            return
        try:
            with open(path, encoding="utf-8") as f:
                applied, skipped, clashes = import_shortcuts(self._win, json.load(f))
        except (OSError, ValueError) as exc:
            self.state.note = (str(exc), "error")
            return
        msg = tr("{n} shortcuts imported.", n=applied)
        if skipped:
            msg += " " + tr("{n} belong to actions this installation does not have (a plugin, say) and "
                            "were left out.", n=skipped)
        if clashes:
            msg += " " + tr("The file gave the same key to more than one command; each stays with the "
                            "first:") + " " + "; ".join(
                tr("{key}: kept for «{kept}», taken from «{dropped}»",
                   key=_keys_text([k]), kept=_plain(kept.text()),
                   dropped=_plain(dropped.text())) for k, kept, dropped in clashes)
        self.state.note = (msg, "warn" if clashes else "ok")

    def _ndof_verb(self, verb) -> None:
        from pathlib import Path
        p = _p()
        env = Env(self._win)
        if verb == "ndof_reset":
            _device_edit(env, {"ndof_preset": "original", "ndof_lock": False})
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
                self.state.note = (tr("Invalid SpaceMouse profile") + ": " + str(exc), "error")
                return
            from ingetrazo_render import DevicePreferences
            name = DevicePreferences.profile_name(Path(path).stem, True)
            self._ndof_add(env, name, profile)
        elif verb == "ndof_save":
            def named(name):
                if name is None:
                    return
                from ingetrazo_render import DevicePreferences
                try:
                    name = DevicePreferences.profile_name(name, False)
                except ValueError as exc:
                    self.state.note = (tr(str(exc)), "error")
                    chrome_sheet.refresh(self._win)
                    return
                from core.ndof_profiles import profile_from_dict
                model, _ = _device_preferences(Env(self._win))
                profile = profile_from_dict(json.loads(model.profile()))
                if name in p.read_named_ndof_profiles():
                    self._confirm(tr("Replace profile?"),
                                  tr("Replace the existing profile '{name}'?", name=name), tr("Replace"),
                                  lambda: self._ndof_add(Env(self._win), name, profile))
                    return
                self._ndof_add(Env(self._win), name, profile)
            cf.ask_text(self._win, tr("Save SpaceMouse profile"), tr("Profile name:"), done=named)

    def _ndof_add(self, env, name, profile) -> None:
        named = _p().read_named_ndof_profiles()
        named[name] = profile
        _device_edit(env, {"ndof_preset": "named:" + name}, named)
        chrome_sheet.refresh(self._win)

    # -- restore defaults ---------------------------------------------------------------
    def restore_defaults(self, win) -> None:
        """Every Built setting with a default back to it (the dialog's
        Restore defaults, applied at once). Language, keyboard shortcuts and
        model settings keep their values."""
        self._win = win
        for sid, _label, scope in SECTIONS:
            if scope != MAC or sid == "keyboard":
                continue
            for _gid, _caption, rows in self.groups(Env(win), sid):
                for r in rows:
                    if r.default is not _NO and r.put is not None and r.value(Env(win)) != r.default:
                        self.apply(r, r.default)
        from core import units
        from views.settings_apply import _UNITS_FOR_NEW_KEY
        Env(win).st.setValue(_UNITS_FOR_NEW_KEY, "1")
        units.set_ask_at_start(True)
        chrome_sheet.refresh(win)

    def closed(self, ctx) -> None:
        self.end_site()
        self._monitor(False)
        self.state.note = None
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
