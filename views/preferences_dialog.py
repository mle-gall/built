# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Window ▸ Preferences — the scattered QSettings, gathered in one dialog.

Three tabs. **General**: UI language (applied on restart, same contract as
the Window ▸ Language menu) and how the rest of the model reads while
editing a group (applied live through ``set_edit_rest_mode``, the same path
the Camera menu uses, so its checkmarks stay honest). **Import**: the units
suggested by the OBJ and DXF/DWG import dialogs (each dialog still asks —
these are the preselected answers) and the coordinate-entry mode shared by
the georef dialogs. **AI Assistant**: the same ``ia/*`` keys the assistant
dialog reads when it opens — provider, API key, model, Ollama URL,
screenshots.

Values are written on OK (with a ``sync()`` flush, the house crash-safety
pattern); Cancel touches nothing.
"""
from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import QRect, QSettings, QSize, Qt
from PySide6.QtGui import QGuiApplication, QIcon, QPixmap
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDialog,
    QDoubleSpinBox,
    QDialogButtonBox,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLineEdit,
    QInputDialog,
    QMessageBox,
    QPushButton,
    QScrollArea,
    QSlider,
    QSpinBox,
    QTabWidget,
    QLabel,
    QVBoxLayout,
    QWidget,
)

from core import _native
from core import general_settings as _gs
from core.i18n import LANGUAGE_NAMES as _LANGUAGE_NAMES
from core.i18n import SYSTEM as SYSTEM_LANGUAGE
from core.i18n import available_languages, tr


from views.settings_apply import (
    _DXF_UNITS, _OBJ_UNITS, apply_accent_choice,
    apply_autosave, apply_cursor_style, apply_invert_orbit,
    apply_invert_wheel, apply_language, apply_msaa, apply_rest_mode,
    apply_rust_engine, apply_slides, apply_soft_shadows, apply_theme_choice,
    apply_toolbar_icon_px, apply_undo_steps, apply_window_glass,
    graphics_server_applies, native_views_of,
    reload_ai_panel,
    viewports_of,
)

_REST_MODES = (("normal", "Show normally"), ("fade", "Fade"),
               ("hide", "Hide (fastest)"))
_DEFAULTS_NOTE = (
    "Puts the settings of this window back to how Built comes. "
    "Nothing is saved until you press OK. The language, the AI Assistant, "
    "the keyboard shortcuts and the units of the open document stay as "
    "they are.")


class _ScrollingPage(QScrollArea):
    """A Preferences page that scrolls when the dialog is shorter than it
    (#1336). It still ASKS for its whole size, so on a screen with room
    the dialog opens as before; only its minimum drops, so the dialog can
    be made smaller than its tallest page."""

    def __init__(self, page: QWidget) -> None:
        super().__init__()
        self.setWidget(page)
        self.setWidgetResizable(True)
        self.setFrameShape(QScrollArea.NoFrame)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarAsNeeded)

    def sizeHint(self) -> QSize:
        hint = self.widget().sizeHint()
        bar = self.verticalScrollBar().sizeHint().width()
        return QSize(hint.width() + bar, hint.height())

    def minimumSizeHint(self) -> QSize:
        # A few rows, in both directions: the dialog opens at its natural
        # size wherever it fits, so a page only scrolls on a screen too
        # small for it, or when made smaller by hand.
        return QSize(320, 120)


#: Room left for the window's title bar and frame when the dialog is
#: fitted to the screen (they are not part of its own geometry).
_FRAME_ALLOWANCE = 48


class PreferencesDialog(QDialog):
    def __init__(self, window) -> None:
        super().__init__(window)
        self._window = window
        self.setWindowTitle(tr("Preferences") + " — Built")
        self.setMinimumWidth(440)
        st = QSettings()

        layout = QVBoxLayout(self)
        tabs = QTabWidget()
        layout.addWidget(tabs)

        # ---- General --------------------------------------------------------
        general = QWidget()
        form = QFormLayout(general)
        self._lang = QComboBox()
        self._lang.addItem(tr("Same as the system"), SYSTEM_LANGUAGE)
        for code in available_languages():
            self._lang.addItem(_LANGUAGE_NAMES.get(code, code), code)
        pending = str(st.value("language") or SYSTEM_LANGUAGE)
        self._lang.setCurrentIndex(max(0, self._lang.findData(pending)))
        form.addRow(tr("Language:"), self._lang)

        from views.theme import (CAMPBELL, DARK, LIGHT, SAND, SLATE,
                                 SOLARIZED, STUDIO, SYSTEM, saved_theme)
        self._theme = QComboBox()
        for key, label in ((DARK, tr("Dark")), (SLATE, tr("Slate")),
                           (STUDIO, tr("Studio")), (SOLARIZED, tr("Solarized")),
                           (CAMPBELL, tr("Campbell")),
                           (LIGHT, tr("Light")),
                           (SAND, tr("Sand")),
                           (SYSTEM, tr("Same as the system"))):
            self._theme.addItem(label, key)
        self._theme.setCurrentIndex(max(0, self._theme.findData(
            saved_theme())))
        self._theme.setToolTip(tr(
            "Colours of menus, panels and toolbars. «Same as the system» "
            "switches along with the desktop's light or dark mode. The 3D "
            "view keeps its own style. Applies at once."))
        form.addRow(tr("Theme:"), self._theme)

        from views.theme import ACCENTS, accent_color, saved_accent
        self._accent = QComboBox()
        for key, label, _rgb in ACCENTS:
            swatch = QPixmap(14, 14)
            swatch.fill(accent_color(key))
            self._accent.addItem(QIcon(swatch), tr(label), key)
        self._accent.setCurrentIndex(max(0, self._accent.findData(
            saved_accent())))
        self._accent.setToolTip(tr(
            "Colour of the details of the icons and of the toolbar buttons "
            "under the mouse. «Same as the system» uses the desktop's "
            "accent colour. The selection in the 3D view stays orange. "
            "Applies at once."))
        form.addRow(tr("Accent colour:"), self._accent)

        # macOS 26+: the Liquid Glass chrome (views/mac_glass.py), on by
        # default. Elsewhere the row is not there at all.
        from views import mac_glass
        self._glass = None
        if mac_glass.supported():
            self._glass = QCheckBox(tr("Liquid Glass window chrome"))
            self._glass.setChecked(mac_glass.enabled())
            self._glass.setToolTip(tr(
                "Toolbars, panels and menus on macOS's translucent glass, "
                "with the toolbars in the title bar. Off: the opaque look "
                "Built has on every system. Applies after a restart."))
            form.addRow("", self._glass)

        self._rest = QComboBox()
        for key, label in _REST_MODES:
            self._rest.addItem(tr(label), key)
        self._rest.setCurrentIndex(max(0, self._rest.findData(
            window.viewport.edit_rest_mode)))
        form.addRow(tr("Rest of model while editing:"), self._rest)

        # A colour wash rather than translucent geometry: the surroundings
        # keep hiding what is behind them while the open group remains at
        # full strength.
        fade_row = QHBoxLayout()
        self._rest_fade = QSlider(Qt.Horizontal)
        self._rest_fade.setRange(0, 100)
        current_fade = getattr(window.viewport, "edit_rest_fade", 0.75)
        try:
            current_fade = float(current_fade)
        except (TypeError, ValueError):
            # Lightweight viewport stand-ins used by embedders/tests may
            # provide only generic callable attributes.
            current_fade = 0.75
        self._rest_fade.setValue(round(current_fade * 100))
        from views.tray import style_slider
        style_slider(self._rest_fade)
        self._rest_fade.setToolTip(tr(
            "How strongly the rest of the model is washed toward grey while "
            "a group or component is open."))
        self._rest_fade_value = QLabel()
        self._rest_fade_value.setMinimumWidth(42)
        fade_row.addWidget(self._rest_fade, 1)
        fade_row.addWidget(self._rest_fade_value)
        form.addRow(tr("Fade intensity:"), fade_row)
        self._rest_fade.valueChanged.connect(
            self._sync_rest_fade_controls)
        self._rest.currentIndexChanged.connect(
            self._sync_rest_fade_controls)
        self._sync_rest_fade_controls()

        row = QHBoxLayout()
        self._autosave = QCheckBox(tr("Auto-save every"))
        self._autosave.setChecked(str(_gs.value("autosave", "1", st))
                                  != "0")
        row.addWidget(self._autosave)
        self._autosave_min = QSpinBox()
        self._autosave_min.setRange(1, 60)
        self._autosave_min.setSuffix(" " + tr("min"))
        try:
            self._autosave_min.setValue(int(_gs.value("autosave_min", 5, st)))
        except (TypeError, ValueError):
            self._autosave_min.setValue(5)
        self._autosave.toggled.connect(self._autosave_min.setEnabled)
        self._autosave_min.setEnabled(self._autosave.isChecked())
        row.addWidget(self._autosave_min)
        row.addStretch()
        form.addRow("", row)

        self._backup = QCheckBox(tr("Keep a backup of the previous save "
                                    "(.bak)"))
        self._backup.setChecked(str(_gs.value("backup", "1", st)) != "0")
        form.addRow("", self._backup)

        self._undo_steps = QSpinBox()
        self._undo_steps.setRange(0, 5000)
        self._undo_steps.setSpecialValueText(tr("unlimited"))
        self._undo_steps.setValue(int(_gs.value("undo_steps", 200, st)))
        self._undo_steps.setToolTip(tr(
            "Steps kept for Undo. Each one holds a snapshot of the model, "
            "so fewer steps mean less memory on a big model; 0 = unlimited."))
        form.addRow(tr("Undo steps"), self._undo_steps)

        self._ask_section = QCheckBox(tr(
            "Ask for a name and symbol when placing a section plane"))
        self._ask_section.setChecked(
            str(st.value("section/ask_name", "1")) != "0")
        form.addRow("", self._ask_section)

        self._hover_picture = QCheckBox(tr(
            "Show a picture of the project on hover over its tab"))
        self._hover_picture.setChecked(
            str(st.value("tabs/hover_picture", "1")) != "0")
        form.addRow("", self._hover_picture)

        from core.update_check import auto_enabled, store_managed
        self._check_updates = QCheckBox(tr("Check for new versions at startup"))
        self._check_updates.setToolTip(tr(
            "Once a day, Built asks GitHub for the latest version number. "
            "Nothing about you or your files is sent."))
        self._check_updates.setChecked(auto_enabled(st))
        # Into the form, or nowhere: setVisible(True) while it had no
        # parent yet showed it for an instant as a window of its own.
        if not store_managed():
            form.addRow("", self._check_updates)

        # A new document starts with the 1.70 m scale figure; someone who
        # models parts for a 3D printer starts on an empty sheet (#221).
        self._scale_figure = QCheckBox(tr(
            "Put the scale figure in new documents"))
        self._scale_figure.setChecked(
            str(st.value("new_document/scale_figure", "1")) != "0")
        form.addRow("", self._scale_figure)
        self._build_figure_rows(form, st)

        self._invert = QCheckBox(tr("Invert mouse wheel zoom"))
        self._invert.setChecked(str(st.value("nav/invert_wheel", "0"))
                                != "0")
        form.addRow("", self._invert)

        self._invert_orbit = QCheckBox(tr("Invert vertical orbit"))
        self._invert_orbit.setChecked(str(st.value("nav/invert_orbit_y", "0"))
                                      != "0")
        form.addRow("", self._invert_orbit)

        # A finger's slide on the touch screen orbits by default; two
        # fingers on the trackpad zoom, as before 0.6.1. Shift pans and
        # Cmd/Ctrl zooms whatever is chosen here (#248).
        from views import trackpad_nav

        def slide_combo(current):
            box = QComboBox()
            for key, label in (("orbit", "Orbits the view"),
                               ("pan", "Pans the view"),
                               ("zoom", "Zooms the view")):
                box.addItem(tr(label), key)
            box.setCurrentIndex(max(0, box.findData(current)))
            return box
        self._touch_slide = slide_combo(trackpad_nav.load_touch_slide())
        form.addRow(tr("One-finger slide on the touch screen:"), self._touch_slide)
        self._trackpad_slide = slide_combo(trackpad_nav.load_slide_mode())
        form.addRow(tr("Two-finger slide on the trackpad:"), self._trackpad_slide)
        self._trackpad_inertia = QCheckBox(tr(
            "Keep moving after the fingers lift (inertia)"))
        self._trackpad_inertia.setChecked(trackpad_nav.load_inertia())
        form.addRow("", self._trackpad_inertia)

        from tools.walkthrough import look_sensitivity
        self._look_sens = QSpinBox()
        self._look_sens.setRange(1, 100)
        self._look_sens.setValue(look_sensitivity())
        self._look_sens.setToolTip(tr(
            "How fast a drag turns the head in First Person"))
        form.addRow(tr("Mouse-look sensitivity:"), self._look_sens)

        # The pointer while a tool is active: its own icon (the default),
        # or the classic crosshair or the plain arrow for who prefers them.
        self._cursor_style = QComboBox()
        for key, label in (("tool", "The tool's icon"),
                           ("crosshair", "Crosshair"), ("arrow", "Arrow")):
            self._cursor_style.addItem(tr(label), key)
        self._cursor_style.setCurrentIndex(max(0, self._cursor_style.findData(
            str(_gs.value("cursor_style", "tool", st)).lower())))
        form.addRow(tr("Tool cursor:"), self._cursor_style)

        self._msaa = QComboBox()
        for n in (0, 2, 4, 8):
            self._msaa.addItem(tr("Off") if n == 0 else f"{n}x", n)
        try:
            msaa_now = int(st.value("display/msaa", 4))
        except (TypeError, ValueError):
            msaa_now = 4
        self._msaa.setCurrentIndex(max(0, self._msaa.findData(
            msaa_now if msaa_now in (0, 2, 4, 8) else 4)))
        form.addRow(tr("Anti-aliasing (MSAA):"), self._msaa)

        # Soft sun shadows (#1281): off by default, they cost the graphics
        # card more on every frame while the sun casts shadows.
        from views.document_interaction import SOFT_SHADOW_SPREAD
        row = QHBoxLayout()
        self._soft_shadows = QCheckBox(tr("Soft sun shadows"))
        self._soft_shadows.setToolTip(tr(
            "Shadow edges that soften away from what casts them, as in "
            "daylight. They cost the graphics card more while the sun casts "
            "shadows."))
        row.addWidget(self._soft_shadows)
        self._soft_level = QComboBox()
        for level, label in ((1, "Slight"), (2, "Moderate"), (3, "Strong")):
            self._soft_level.addItem(tr(label), level)
        try:
            soft_now = int(st.value("display/soft_shadows", 0))
        except (TypeError, ValueError):
            soft_now = 0
        try:
            level_now = int(st.value("display/soft_shadows_level", 2))
        except (TypeError, ValueError):
            level_now = 2
        if soft_now in SOFT_SHADOW_SPREAD:
            level_now = soft_now
        self._soft_level.setCurrentIndex(max(0, self._soft_level.findData(
            level_now if level_now in SOFT_SHADOW_SPREAD else 2)))
        self._soft_shadows.setChecked(soft_now in SOFT_SHADOW_SPREAD)
        self._soft_shadows.toggled.connect(self._soft_level.setEnabled)
        self._soft_level.setEnabled(self._soft_shadows.isChecked())
        row.addWidget(self._soft_level)
        row.addStretch()
        form.addRow("", row)

        from views.icons import TOOLBAR_ICON_SIZES, toolbar_icon_px
        self._icon_px = QComboBox()
        for px, label in TOOLBAR_ICON_SIZES:
            self._icon_px.addItem(f"{tr(label)} ({px} px)", px)
        self._icon_px.setCurrentIndex(max(0, self._icon_px.findData(
            toolbar_icon_px())))
        self._icon_px.setToolTip(tr(
            "Size of the icons on every toolbar — the model's and the "
            "sheet composer's. Applies at once."))
        form.addRow(tr("Toolbar icons:"), self._icon_px)

        # How the extensions' entries show (18-10: with the whole catalog
        # the Extensions menu and the right-click menu outgrew the screen).
        self._ext_menus = QComboBox()
        for code, label in (
                ("auto", "Group them when there are many"),
                ("always", "Always in submenus"),
                ("flat", "Each entry on its own")):
            self._ext_menus.addItem(tr(label), code)
        self._ext_menus.setCurrentIndex(max(0, self._ext_menus.findData(
            str(_gs.value("ext_menus", "auto", st) or "auto"))))
        self._ext_menus.setToolTip(tr(
            "How the extensions' entries show in the right-click menu "
            "(at once) and in the Extensions menu (at the next start)."))
        form.addRow(tr("Extension menus:"), self._ext_menus)

        from core.platform_choice import AUTO, WAYLAND, XCB
        self._platform = QComboBox()
        for key, label in ((AUTO, tr("Automatic (X11 on KDE Plasma or with a fractional display scale)")),
                           (WAYLAND, tr("Wayland")), (XCB, tr("X11 (XWayland)"))):
            self._platform.addItem(label, key)
        self._platform.setCurrentIndex(max(0, self._platform.findData(
            str(_gs.value("platform", AUTO, st) or AUTO))))
        self._platform.setToolTip(tr(
            "Which display server Qt draws through. Under Wayland with a "
            "125 % / 150 % display scale the viewport stutters; X11 "
            "(XWayland) draws smoothly and stays crisp. Takes effect at the "
            "next start."))
        # Wayland or X11 is a question only Linux asks: on Windows and
        # macOS the row spoke of KDE and XWayland to someone with neither
        # (#1179). The box is still made — reading and saving go through
        # it — only not shown.
        if graphics_server_applies():
            form.addRow(tr("Graphics server:"), self._platform)
        else:
            self._platform.setParent(general)
            self._platform.hide()

        # The Rust engine (core._native): on by default, read
        # once at start. Greyed out in a build that does not carry it.
        self._rust = QCheckBox(tr("Rust engine"))
        self._rust.setChecked(str(_gs.value(_native.SETTING, _native.DEFAULT, st)) == "1")
        if _native.available() is None:
            self._rust.setEnabled(False)
            self._rust.setToolTip(tr("This build of Built does not carry "
                                     "the Rust engine."))
        else:
            self._rust.setToolTip(tr(
                "Computes the heavy parts (opening big files, hidden lines "
                "of the sheets, intersections, snapping) with the native "
                "engine, much faster on big models, with the same results. "
                "On by default: turn it off only if something looks "
                "different, and report it with the label rust-engine. "
                "Takes effect at the next start."))
        form.addRow("", self._rust)
        from core import renderer_settings
        cap = renderer_settings.capability()
        self._renderer_status = QLabel(renderer_settings.status_text())
        self._renderer_status.setWordWrap(True)
        form.addRow(tr("Available renderer:"), self._renderer_status)
        form.addRow(tr("Runtime:"), QLabel(
            f"Python {cap.python_version} / Qt {cap.qt_version}"))
        tabs.addTab(_ScrollingPage(general), tr("General"))

        # ---- Import ---------------------------------------------------------
        imp = QWidget()
        form = QFormLayout(imp)
        self._obj_unit = QComboBox()
        for key, label in _OBJ_UNITS:
            self._obj_unit.addItem(tr(label), key)
        self._obj_unit.setCurrentIndex(max(0, self._obj_unit.findData(
            str(st.value("import/obj_unit", "m") or "m"))))
        form.addRow(tr("Suggested unit for OBJ:"), self._obj_unit)

        self._dxf_unit = QComboBox()
        for key, label in _DXF_UNITS:
            self._dxf_unit.addItem(tr(label), key)
        self._dxf_unit.setCurrentIndex(max(0, self._dxf_unit.findData(
            str(st.value("import/dxf_unit", "mm") or "mm"))))
        form.addRow(tr("Suggested unit for DXF/DWG:"), self._dxf_unit)

        self._coord = QComboBox()
        self._coord.addItem(tr("Geographic (lat/lon)"), "geo")
        self._coord.addItem(tr("UTM WGS84"), "utm")
        self._coord.setCurrentIndex(max(0, self._coord.findData(
            str(st.value("georef/coord_mode", "geo") or "geo"))))
        form.addRow(tr("Coordinate entry:"), self._coord)
        tabs.addTab(_ScrollingPage(imp), tr("Import"))

        # ---- AI Assistant ---------------------------------------------------
        ia = QWidget()
        form = QFormLayout(ia)
        self._provider = QComboBox()
        self._provider.addItem(tr("Auto (by key prefix)"), "auto")
        try:
            from core import ai
            for prov in ai.PROVIDERS:
                self._provider.addItem(ai.PROVIDER_INFO[prov][0], prov)
        except Exception:           # noqa: BLE001 — the tab, not the dialog
            pass
        self._provider.setCurrentIndex(max(0, self._provider.findData(
            str(st.value("ia/proveedor", "auto") or "auto"))))
        form.addRow(tr("Provider:"), self._provider)

        self._api_key = QLineEdit(str(st.value("ia/api_key", "") or ""))
        self._api_key.setEchoMode(QLineEdit.Password)
        self._api_key.setPlaceholderText(tr("empty = local AI (Ollama, LM Studio)"))
        form.addRow(tr("API key:"), self._api_key)
        # The same note as the AI tab: a key the environment lends (#998).
        self._key_env = QLabel("")
        self._key_env.setWordWrap(True)
        form.addRow("", self._key_env)
        self._provider.currentIndexChanged.connect(self._refresh_key_env)
        self._api_key.textChanged.connect(self._refresh_key_env)
        self._refresh_key_env()

        self._model = QLineEdit(str(st.value("ia/modelo", "") or ""))
        self._model.setPlaceholderText(tr("model (default per provider)"))
        form.addRow(tr("Model:"), self._model)

        self._ollama = QLineEdit(str(st.value("ia/ollama_url",
                                              "http://localhost:11434") or ""))
        form.addRow(tr("Local AI URL:"), self._ollama)

        self._shots = QCheckBox(tr("Send viewport screenshots to the model"))
        self._shots.setChecked(str(st.value("ia/capturas", "1")) != "0")
        form.addRow("", self._shots)
        tabs.addTab(_ScrollingPage(ia), tr("AI Assistant"))

        # ---- Keyboard shortcuts (issue #138) -------------------------------
        # (Not wrapped: its list already scrolls on its own.)
        from views.shortcuts import ShortcutsPanel
        self._shortcuts = ShortcutsPanel(self._window)
        tabs.addTab(self._shortcuts, tr("Keyboard shortcuts"))
        self._tabs = tabs

        buttons = QDialogButtonBox(QDialogButtonBox.Ok
                                   | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        # Back to how IngeTrazo comes: only the controls change, so OK
        # saves it and Cancel still touches nothing.
        self._defaults_btn = buttons.addButton(tr("Restore defaults"),
                                               QDialogButtonBox.ResetRole)
        # Never the default button: focused after a click it would take
        # the highlight from OK, and Enter would restore again.
        self._defaults_btn.setAutoDefault(False)
        self._defaults_btn.setToolTip(tr(_DEFAULTS_NOTE))
        self._defaults_btn.clicked.connect(self._ask_restore_defaults)
        layout.addWidget(buttons)
        self._buttons = buttons
        screen = ((window.screen() if window is not None else None)
                  or QGuiApplication.primaryScreen())
        if screen is not None:
            self.fit_to(screen.availableGeometry())

    def fit_to(self, available: QRect) -> None:
        """Open no bigger than ``available`` (the screen's free area), so
        OK and Cancel stay reachable on a short laptop screen (#1336):
        the pages scroll instead. With room to spare it opens at its
        natural size, as before."""
        self.layout().activate()
        hint = self.sizeHint()
        self.resize(min(hint.width(), max(available.width(), 1)),
                    min(hint.height(),
                        max(available.height() - _FRAME_ALLOWANCE, 1)))

    def _refresh_key_env(self, *_args) -> None:
        try:
            from plugins.ai_assistant import env_key_note, key_placeholder
            self._api_key.setPlaceholderText(
                key_placeholder(self._provider.currentData()))
            note = env_key_note(self._provider.currentData(),
                                self._api_key.text().strip())
        except Exception:           # noqa: BLE001 — a note, not the dialog
            note = ""
        self._key_env.setText(note)
        self._key_env.setVisible(bool(note))

    def show_shortcut_of(self, action) -> None:
        """Open on the Keyboard shortcuts page with ``action`` picked."""
        self._tabs.setCurrentWidget(self._shortcuts)
        self._shortcuts.pick(action)

    # ---- Scale figure of new documents (#655) ----------------------------

    def _build_figure_rows(self, form, st) -> None:
        """Which person stands in a new document: one of the bundled cutouts
        or the user's own picture (a PNG on a transparent background, and
        the height that person stands)."""
        from core import scale_figure as sf
        self._figure = QComboBox()
        for p in sf.people():
            icon = QIcon(str(sf.people_dir() / f"{p['key']}.png"))
            self._figure.addItem(
                icon, f"{tr(p['name'])} ({float(p['height']):.2f} m)",
                p["key"])
        self._figure.addItem(tr("My own picture…"), sf.CUSTOM)
        pick = str(st.value(sf.KEY_PICK, sf.DEFAULT_PICK) or sf.DEFAULT_PICK)
        self._figure.setCurrentIndex(max(0, self._figure.findData(pick)))
        form.addRow(tr("Scale figure:"), self._figure)

        # The user's own picture: what is stored, a new file only once OK
        # copies it into the data folder (Cancel leaves nothing behind).
        image = str(st.value(sf.KEY_IMAGE, "") or "")
        self._figure_image = image if Path(image).is_file() else ""
        self._figure_source = ""
        row = QWidget()
        lay = QHBoxLayout(row)
        lay.setContentsMargins(0, 0, 0, 0)
        self._figure_pick_btn = QPushButton(tr("Choose picture…"))
        self._figure_pick_btn.clicked.connect(self._choose_figure_picture)
        self._figure_height = QDoubleSpinBox()
        self._figure_height.setRange(sf.MIN_HEIGHT, sf.MAX_HEIGHT)
        self._figure_height.setDecimals(2)
        self._figure_height.setSingleStep(0.01)
        self._figure_height.setSuffix(" m")
        self._figure_height.setToolTip(tr("The height this person stands"))
        try:
            self._figure_height.setValue(float(st.value(sf.KEY_HEIGHT, 1.70)))
        except (TypeError, ValueError):
            self._figure_height.setValue(1.70)
        self._figure_name = QLineEdit(str(st.value(sf.KEY_NAME, "") or ""))
        self._figure_name.setPlaceholderText(tr("Name"))
        lay.addWidget(self._figure_pick_btn)
        lay.addWidget(self._figure_height)
        lay.addWidget(self._figure_name, 1)
        self._figure_custom_row = row
        form.addRow("", row)
        self._figure_file_label = QLabel()
        self._figure_file_label.setWordWrap(True)
        form.addRow("", self._figure_file_label)

        self._figure.currentIndexChanged.connect(self._on_figure_pick)
        self._scale_figure.toggled.connect(self._sync_figure_rows)
        self._figure_last = self._figure.currentIndex()
        self._sync_figure_rows()

    def _figure_is_custom(self) -> bool:
        from core import scale_figure as sf
        return self._figure.currentData() == sf.CUSTOM

    def _sync_figure_rows(self) -> None:
        on = self._scale_figure.isChecked()
        custom = self._figure_is_custom()
        self._figure.setEnabled(on)
        self._figure_custom_row.setVisible(custom)
        self._figure_custom_row.setEnabled(on)
        shown = self._figure_source or self._figure_image
        self._figure_file_label.setVisible(custom and bool(shown))
        if shown:
            self._figure_file_label.setText(
                "<small>" + Path(shown).name + "</small>")

    def _on_figure_pick(self, _index: int) -> None:
        # «My own picture…» with none chosen yet asks for one at once; a
        # cancelled dialog goes back to the person shown before.
        if (self._figure_is_custom() and not self._figure_image
                and not self._figure_source
                and not self._choose_figure_picture()):
            self._figure.blockSignals(True)
            self._figure.setCurrentIndex(self._figure_last)
            self._figure.blockSignals(False)
        self._figure_last = self._figure.currentIndex()
        self._sync_figure_rows()

    def _choose_figure_picture(self) -> bool:
        from views.filedialogs import file_dialogs as fd
        path, _ = fd.getOpenFileName(
            self, tr("Choose the picture of your scale figure"), "",
            tr("Images") + " (*.png *.webp);;" + tr("All files") + " (*)")
        if not path:
            return False
        from PySide6.QtGui import QImage
        img = QImage(path)
        if img.isNull() or img.height() == 0:
            QMessageBox.warning(self, tr("Scale figure"),
                                tr("That file is not a picture Built can read."))
            return False
        if not img.hasAlphaChannel():
            QMessageBox.information(
                self, tr("Scale figure"),
                tr("This picture has no transparent background, so it will "
                   "show as a rectangle. A PNG cut out around the person "
                   "looks best."))
        self._figure_source = path
        if not self._figure_name.text().strip():
            self._figure_name.setText(Path(path).stem.replace("_", " "))
        self._sync_figure_rows()
        return True

    def _save_figure(self, st) -> None:
        from core import scale_figure as sf
        pick = self._figure.currentData()
        if pick == sf.CUSTOM and self._figure_source:
            try:
                self._figure_image = str(sf.store_picture(self._figure_source))
            except (OSError, ValueError):
                QMessageBox.warning(
                    self, tr("Scale figure"),
                    tr("The picture could not be copied; the previous "
                       "figure is kept."))
                return
        if pick == sf.CUSTOM and not self._figure_image:
            return
        st.setValue(sf.KEY_PICK, pick)
        if pick == sf.CUSTOM:
            st.setValue(sf.KEY_IMAGE, self._figure_image)
            st.setValue(sf.KEY_HEIGHT, round(self._figure_height.value(), 3))
            st.setValue(sf.KEY_NAME, self._figure_name.text().strip())

    def _sync_rest_fade_controls(self, _value=None) -> None:
        """Keep the percentage legible and disable it outside Fade mode."""
        self._rest_fade_value.setText(f"{self._rest_fade.value()} %")
        self._rest_fade.setEnabled(self._rest.currentData() == "fade")

    def _viewports(self) -> list:
        return viewports_of(self._window)

    def _native_views(self) -> list:
        return native_views_of(self._window)

    # ---- Restore defaults -------------------------------------------------

    def _ask_restore_defaults(self) -> None:
        answer = QMessageBox.question(
            self, tr("Restore defaults"), tr(_DEFAULTS_NOTE),
            QMessageBox.Ok | QMessageBox.Cancel, QMessageBox.Cancel)
        if answer == QMessageBox.Ok:
            self.restore_defaults()

    def restore_defaults(self) -> None:
        """Set every control to the value a fresh profile gets. Left alone:
        the language (it has its own menu), the AI Assistant (a key and a
        model are an account, not a taste), the keyboard shortcuts (their
        page resets them) and the units, which belong to the document."""
        from core import scale_figure as sf
        from core.platform_choice import AUTO
        from tools.walkthrough import DEFAULT_LOOK_SENSITIVITY
        from views.icons import default_toolbar_icon_px
        from views.theme import DARK, ORANGE

        def pick(combo, data):
            combo.setCurrentIndex(max(0, combo.findData(data)))

        pick(self._theme, DARK)
        pick(self._accent, ORANGE)
        if self._glass is not None:
            self._glass.setChecked(True)
        pick(self._rest, "fade")
        from views.document_interaction import EDIT_REST_FADE
        self._rest_fade.setValue(round(EDIT_REST_FADE * 100))
        self._autosave.setChecked(True)
        self._autosave_min.setValue(5)
        self._backup.setChecked(True)
        self._undo_steps.setValue(200)
        self._ask_section.setChecked(True)
        self._hover_picture.setChecked(True)
        self._check_updates.setChecked(True)
        self._scale_figure.setChecked(True)
        pick(self._figure, sf.DEFAULT_PICK)
        self._invert.setChecked(False)
        self._invert_orbit.setChecked(False)
        pick(self._touch_slide, "orbit")
        pick(self._trackpad_slide, "zoom")
        self._trackpad_inertia.setChecked(True)
        self._look_sens.setValue(DEFAULT_LOOK_SENSITIVITY)
        pick(self._cursor_style, "tool")
        pick(self._msaa, 4)
        self._soft_shadows.setChecked(False)
        pick(self._soft_level, 2)
        pick(self._icon_px, default_toolbar_icon_px())
        pick(self._ext_menus, "auto")
        pick(self._platform, AUTO)
        if self._rust.isEnabled():
            self._rust.setChecked(_native.DEFAULT == "1")

        pick(self._obj_unit, "m")
        pick(self._dxf_unit, "mm")
        pick(self._coord, "geo")

    def accept(self) -> None:  # noqa: D102 — QDialog override
        st = QSettings()

        if apply_language(self._lang.currentData(), st):
            QMessageBox.information(
                self, tr("Language changed"),
                tr("Restart Built to apply the new language."))

        apply_theme_choice(self._theme.currentData())

        if self._glass is not None and apply_window_glass(
                self._glass.isChecked()):
            QMessageBox.information(
                self, tr("Window chrome changed"),
                tr("Restart Built to apply the new window chrome."))

        apply_accent_choice(self._accent.currentData())

        apply_rest_mode(self._window, self._rest.currentData(),
                        self._rest_fade.value() / 100.0)

        _gs.set_value("autosave", "1" if self._autosave.isChecked() else "0",
                      st)
        _gs.set_value("autosave_min", self._autosave_min.value(), st)
        _gs.set_value("backup", "1" if self._backup.isChecked() else "0", st)
        st.setValue("section/ask_name",
                    "1" if self._ask_section.isChecked() else "0")
        st.setValue("tabs/hover_picture",
                    "1" if self._hover_picture.isChecked() else "0")
        from core.update_check import KEY_AUTO
        st.setValue(KEY_AUTO, "1" if self._check_updates.isChecked() else "0")
        st.setValue("new_document/scale_figure",
                    "1" if self._scale_figure.isChecked() else "0")
        self._save_figure(st)
        apply_undo_steps(self._window, self._undo_steps.value(), st)
        _gs.set_value("platform", self._platform.currentData(), st)
        if self._rust.isEnabled() and apply_rust_engine(
                self._rust.isChecked(), st):
            QMessageBox.information(
                self, tr("Rust engine"),
                tr("Restart Built to switch the engine."))
        setup = getattr(self._window, "_setup_autosave", None)
        if callable(setup):
            setup()                     # re-arm the timer with the new pace

        apply_invert_wheel(self._window, self._invert.isChecked(), st)
        apply_cursor_style(self._window, self._cursor_style.currentData(), st)
        apply_invert_orbit(self._window, self._invert_orbit.isChecked(), st)
        apply_slides(self._window, self._trackpad_slide.currentData(),
                     self._touch_slide.currentData(),
                     self._trackpad_inertia.isChecked(), st)

        # First Person reads it on every look move: nothing to push.
        st.setValue("walk/look_sensitivity", int(self._look_sens.value()))

        apply_msaa(self._window, self._msaa.currentData(), st)
        apply_soft_shadows(self._window, self._soft_shadows.isChecked(),
                           int(self._soft_level.currentData()), st)
        apply_toolbar_icon_px(self._window, int(self._icon_px.currentData()))

        _gs.set_value("ext_menus", self._ext_menus.currentData(), st)

        st.setValue("import/obj_unit", self._obj_unit.currentData())
        st.setValue("import/dxf_unit", self._dxf_unit.currentData())
        st.setValue("georef/coord_mode", self._coord.currentData())

        st.setValue("ia/proveedor", self._provider.currentData())
        st.setValue("ia/api_key", self._api_key.text())
        st.setValue("ia/modelo", self._model.text().strip())
        st.setValue("ia/ollama_url", self._ollama.text().strip())
        st.setValue("ia/capturas", "1" if self._shots.isChecked() else "0")
        st.sync()
        reload_ai_panel(self._window)
        super().accept()
