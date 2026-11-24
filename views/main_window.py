# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""IngeTrazo main window: toolbar, viewport, side panels, status bar.

Owns the open document path and dispatches File menu actions (New, Open,
Save, Save As) onto :mod:`formats.igz`.
"""
from __future__ import annotations

from views import chrome_dialogs as _dialogs
from views import chrome_forms as _forms

import sys
import os
from pathlib import Path
from typing import Optional

from core.settings import settings as QSettings
from PySide6.QtCore import (Qt, QEvent, QCoreApplication, QPoint,
                            QSize, QTimer, Signal)
from PySide6.QtGui import QAction, QActionGroup, QKeySequence
from ingetrazo_core import Vec3 as QVector3D
from PySide6.QtWidgets import (
    QApplication,
    QFileDialog,
    QHBoxLayout,
    QLabel,
    QMainWindow,
    QMenu,
    QMessageBox,
    QStatusBar,
    QSlider,
    QTabBar,
    QToolBar,
    QToolButton,
    QWidget,
    QWidgetAction,
)

from core import general_settings as _gs
from core.i18n import current_language, tr
from views.theme import style as theme_style
from core.units import fmt_pair
from views.filedialogs import file_dialogs
from core.version import __version__
from core.group import Group
from core.mesh import Edge, Face
from formats import built
from formats import igz as igz_format
from formats import dae as dae_format
from formats import obj as obj_format
from formats import ifc as ifc_format
from formats import stl as stl_format
from formats import gltf as gltf_format
from tools.texture_position import TexturePositionTool
from views.document_commands import WindowDocumentCommands, _repeatable
from ingetrazo_render import build_tools
from tools.paint import PaintTool
from tools.flip import FlipTool
from views.tray import BimTray, GeorefTray, Tray, style_slider
from views.icons import tool_icon


IGZ_FILE_FILTER = built.DOCUMENT_FILTER + ";;All files (*)"

# What each command does, for the status bar, the button tooltips and F3
# (Blender's descriptions): a sentence, in English — ``tr`` translates it
# where it is set — never repeating the command's name or its keys.
_VIEW_TIPS = {
    "top": "Look straight down on the model, in plan.",
    "bottom": "Look straight up at the model from below.",
    "front": "Look at the model from the front.",
    "back": "Look at the model from the back.",
    "left": "Look at the model from the left.",
    "right": "Look at the model from the right.",
    "iso": "Look at the model from a corner above, at the same angle to "
           "the three axes.",
}
_STYLE_TIPS = {
    "Default": "Faces with their materials and textures, under the sky.",
    "Architectural": "Faces with their materials on a plain white "
                     "background, without the sky.",
    "Shaded": "Faces in their colours, without textures.",
    "Hidden line": "White faces that hide what lies behind them — a clean "
                   "line drawing.",
    "Monochrome": "Every face in the front or back colour, without "
                  "materials.",
    "Wireframe": "Only the edges: the faces are not drawn.",
    "X-ray": "See-through faces, so the edges behind them show.",
}
_SAVE_TIP = ("Save the document — the model and every sheet — to its "
             "file.")
_TEXT3D_TIP = ("Build a text as an extruded solid, in the font and height "
               "you choose.")

class _OpenAborted(Exception):
    """Raised by an open's progress callback when the user closed the
    window while the file was still coming in (#1269): the load unwinds
    at its next milestone instead of finishing behind the close."""


class MainWindow(WindowDocumentCommands, QMainWindow):
    """Top-level IngeTrazo window."""

    def show_home(self):
        from views import wgpu_start
        wgpu_start.show(self)

    def _dismiss_home(self):
        from views import wgpu_start
        wgpu_start.hide(self)

    def __init__(self, defer_extensions: bool = False) -> None:
        super().__init__()
        # macOS 26+: Liquid Glass chrome needs a translucent native window,
        # so the attribute goes on before anything below creates it; the
        # glass itself is installed at the end (views/mac_glass.py).
        from views import mac_glass
        mac_glass.prepare(self)
        # main.py passes True: the plugins' setup runs once the window is up
        # (see load_extensions). Set first — the menu bar is built below.
        self._defer_extensions = bool(defer_extensions)
        self.resize(1280, 800)
        self._tools = build_tools()
        self._tool_actions: dict[str, QAction] = {}

        # The open projects, one per tab (#1066; views/documents.py). What
        # the window used to keep for its one document — the viewport, the
        # file, the saved state — now lives on the ACTIVE one, behind the
        # properties of the same names (see «Documents» below).
        from views.documents import Document
        self._doc = Document()
        self._documents: list = [self._doc]
        self._current_path: Optional[Path] = None
        # Name of an IMPORTED file (.skp/.dae) shown in the title until the
        # model is saved as .igz — opening a .skp file natively should
        # read as opening THAT file (user request).
        self._import_name: Optional[str] = None
        #: An extension's workspace shown instead of the model, and the model
        #: parked meanwhile; see :meth:`enter_workspace`.
        self._workspace = None
        self._parked: Optional[dict] = None
        #: Tool keys a workspace allows (None = every tool).
        self._tool_filter: Optional[set] = None
        #: Suffix (".xyz") → callable(path) -> bool: documents an extension
        #: opens itself (``ExtensionApp.add_file_opener``), from Open Recent,
        #: the command line or a double-click.
        self.file_openers: dict = {}
        self._saved_version: int = 0

        self._setup_ui()
        # The user's own keyboard shortcuts over the factory ones (#138).
        from views import shortcuts as _shortcuts
        _shortcuts.remember_defaults(self)
        _shortcuts.apply_user_shortcuts(self)
        self._activate_tool("select")
        self._apply_new_document_units()
        self._insert_scale_figure()
        self._update_title()

    # ---- Layout -------------------------------------------------------------
    def _setup_ui(self) -> None:
        # The modelling area: the project tabs over a stack of viewports,
        # one per open project (#1066). Only the tab in front is shown.
        from PySide6.QtWidgets import QStackedWidget, QVBoxLayout
        from views.documents import ProjectTabBar
        area = self._doc_area = QWidget(self)
        area.setObjectName("document_area")
        box = QVBoxLayout(area)
        box.setContentsMargins(0, 0, 0, 0)
        box.setSpacing(0)
        bar = self._project_tabs = ProjectTabBar(area)
        bar.activated.connect(self.activate_document)
        bar.closeRequested.connect(self.close_document)
        bar.newRequested.connect(self._on_new_tab)
        box.addWidget(bar)
        self._doc_stack = QStackedWidget(area)
        box.addWidget(self._doc_stack, 1)
        self._empty_on = False
        self._doc.viewport = self._make_viewport()
        self._doc_stack.addWidget(self._doc.viewport.keys_stand_in or self._doc.viewport)
        bar.add(self._doc)
        self.setCentralWidget(area)

        self._extension_docks: list = []   # side panels added by plugins
        self._build_toolbar()
        self._build_tray()
        self._build_menubar()
        self._build_statusbar()
        self._describe_buttons()

        self._saved_version = self.viewport.scene.version
        self._wire_viewport(self.viewport)

        # An .igz open in flight, a close that lands during it, and a
        # close that has been accepted for good (#1269).
        self._open_in_progress = False
        self._open_aborted = False
        self._closed = False

        # Toolbar/dock layout: the user's own arrangement survives sessions
        # (saved on close); a fresh profile starts from the factory layout
        # blob when one is shipped. Restored AFTER every toolbar and dock
        # exists — saveState matches them by objectName.
        st = QSettings()
        state = st.value("ui/window_state")
        if state is None:
            from core.paths import app_root
            factory = app_root() / "resources" / "ui" / "default_layout.state"
            if factory.is_file():
                state = factory.read_bytes()
        if state:
            self.restoreState(state)
        self._show_default_trays(st)
        self._place_new_toolbars(st)
        geo = st.value("ui/window_geometry")
        if geo:
            self.restoreGeometry(geo)
            # Saved on another (bigger) display, it may not fit this one.
            # Here and now, not after the show: a size set on purpose later
            # (a script, a test, the user) is left alone.
            self._clamp_to_screen()

        self._setup_autosave()
        # An untitled recovery slot on disk means the last session died with
        # unsaved work (a clean exit clears it) — offer it back, once the
        # window is up (QTimer 0: not during __init__, the viewport isn't
        # ready to paint a restored scene yet).
        from core import autosave
        if autosave.pending_untitled():
            from PySide6.QtCore import QTimer
            QTimer.singleShot(0, self, self._offer_untitled_recovery)
        # macOS 26+: the chrome on Liquid Glass (views/mac_glass.py); a
        # no-op elsewhere. Before the first show — it needs a translucent
        # native window.
        from views import mac_glass
        mac_glass.install(self)
        self._choose_document_renderer(self._doc)

    def showEvent(self, event) -> None:  # noqa: N802 — Qt override
        super().showEvent(event)
        if self.focusWidget() is None:
            # The keyboard starts on the model: left to Qt, the window's
            # activation gave it to the first button of the Tab chain (the
            # project tabs'), and the arrows, a typed value and Enter did
            # nothing until a click in the view.
            view = self._shown_viewport()
            if view is not None:
                view.setFocus(Qt.OtherFocusReason)
        if not getattr(self, "_ndof_connected", False):
            self._connect_ndof()

    # ---- 3D mouse (issue #108) ----------------------------------------------
    def _route_window_toggle(self, name: str, own):
        """A slot for a Window-menu toggle (clean screen, sidebar) that acts
        on the window being worked in. On macOS the menu bar is global: its
        Cmd+0 answered for the MODEL window even with the sheet composer in
        front — «al pulsarlo me lleva a la ventana del Modelador y le oculta
        todo» (#114). When the composer is the active window, the press goes
        to its own action and this one keeps its state."""
        def slot(on: bool) -> None:
            from PySide6.QtWidgets import QApplication
            comp = getattr(self, "_composer", None)
            if comp is not None and QApplication.activeWindow() is comp:
                mine = getattr(self, name)
                mine.blockSignals(True)
                mine.setChecked(not on)
                mine.blockSignals(False)
                getattr(comp, name).toggle()
                return
            own(on)
        return slot

    def _connect_ndof(self) -> None:
        """Listen to the 3D mouse, if the machine has one (spacenavd on
        Linux, Raw Input on Windows). Silent when there is none."""
        self._ndof_connected = True
        try:
            from views.ndof_input import shared_input
            dev = shared_input()
            dev.motion.connect(self._on_ndof_motion)
            dev.button.connect(self._on_ndof_button)
        except Exception as exc:  # noqa: BLE001 — never block the window
            import sys as _sys
            print(f"Built: 3D mouse unavailable ({exc!r})",
                  file=_sys.stderr)

    def createPopupMenu(self):  # noqa: N802 — Qt override
        """Include toolbar customization in the standard Qt context menu."""
        menu = super().createPopupMenu()
        if menu is None:
            menu = QMenu(self)
        custom = getattr(self, "_act_customize_toolbars", None)
        if custom is not None and custom.isVisible():
            menu.addAction(custom)
        return menu

    def _place_new_toolbars(self, st) -> None:
        """A toolbar born after a profile saved its layout is unknown to
        that saved state, so Qt leaves it wherever it was created (the
        top). Put it where the factory layout has it, ONCE, and remember
        that: Marco's own arrangement keeps it from then on.

        Walkthrough (2026-09-19) goes at the left, under Annotate — below
        the 3D Text button, where Marco asked for it."""
        from PySide6.QtCore import Qt
        key = "ui/placed/walkthrough"
        tb = self.toolbars.get("walkthrough")
        if tb is None or st.value(key) is not None:
            return
        if self.toolBarArea(tb) != Qt.LeftToolBarArea:
            self.removeToolBar(tb)
            self.addToolBar(Qt.LeftToolBarArea, tb)
            tb.show()
        st.setValue(key, 1)

    # ---- Auto-save (Preferences ▸ General) ----------------------------------
    def _setup_autosave(self) -> None:
        """(Re)arm the auto-save timer from settings — called at startup and
        by the Preferences dialog after a change."""
        from PySide6.QtCore import QTimer
        timer = getattr(self, "_autosave_timer", None)
        if timer is None:
            timer = self._autosave_timer = QTimer(self)
            timer.timeout.connect(self._on_autosave_tick)
        self._configure_recovery(timer)

    def _build_tray(self) -> None:
        # Two role-based right-side docks (tabbed): Properties (what you're
        # working with) and Georef (the location workspace).
        self.tray = Tray(self)
        self.bim_tray = BimTray(self)
        self.georef_tray = GeorefTray(self)
        from views.outliner import ProjectBrowser
        self.project_browser = ProjectBrowser(self)
        self._lay_out_trays()
        for dock, key in ((self.tray, "panel_properties"),
                          (self.bim_tray, "panel_bim"),
                          (self.georef_tray, "panel_terrain"),
                          (self.project_browser, "panel_browser")):
            self.set_dock_icon(dock, key)
            dock.setTitleBarWidget(QWidget(dock))
        self.tray.raise_()
        self.viewport.setAcceptDrops(True)
        # The keyboard never strands on a button or on nothing (see
        # _keys_back_to_view).
        QApplication.instance().focusChanged.connect(self._keys_back_to_view)
        # (The panels follow the viewport's changes: see _wire_viewport.)
        # Styles, Shadows and Dimension style hang OFF THE TOOLBAR as
        # dropdown panels (user: the lateral bar was drowning, and floating
        # windows felt loose — "¿no hay forma de integrarlo en el
        # toolbar?"). Click the button, the panel drops under it; click
        # outside, it folds away.
        from PySide6.QtWidgets import QToolButton, QWidgetAction
        from views.tray import DimensionStylePanel, ShadowsPanel, StylesPanel
        self.styles_panel = StylesPanel(self)
        self.shadows_panel = ShadowsPanel(self)
        self.dimstyle_panel = DimensionStylePanel(self)
        panels_tb = self._new_toolbar(tr("Panels"), "panels_toolbar")
        for panel, key, title, tip in (
                (self.styles_panel, "styles", tr("Styles"),
                 tr("How the model looks: faces, edges, background and "
                    "sky.")),
                (self.shadows_panel, "shadows", tr("Shadows"),
                 tr("The sun's shadows: on or off, the date, the time and "
                    "how dark they are.")),
                (self.dimstyle_panel, "dimension_style", tr("Dimension style"),
                 tr("How dimensions look: text, arrows, units and "
                    "precision."))):
            btn = QToolButton(panels_tb)
            btn.setIcon(tool_icon(key))
            btn.setToolTip(f"{title}\n{tip}")
            btn.setPopupMode(QToolButton.InstantPopup)
            btn.setStyleSheet(
                "QToolButton::menu-indicator { image: none; }")
            menu = QMenu(btn)
            wa = QWidgetAction(menu)
            wa.setDefaultWidget(panel)
            menu.addAction(wa)
            btn.setMenu(menu)
            panels_tb.addWidget(btn)
            self._icon_actions.append((btn, key))

    def _new_toolbar(self, title: str, object_name: str) -> QToolBar:
        """A separate, independently draggable/floatable icons-only toolbar
        (Draw, Modify, View… each move on their own)."""
        from PySide6.QtCore import QSize
        tb = QToolBar(title, self)
        tb.setObjectName(object_name)
        tb.setMovable(True)
        tb.setFloatable(True)
        tb.setAllowedAreas(Qt.AllToolBarAreas)
        from views.icons import DEFAULT_TOOLBAR_ICON_PX
        px = DEFAULT_TOOLBAR_ICON_PX
        tb.setIconSize(QSize(px, px))
        tb.setToolButtonStyle(Qt.ToolButtonIconOnly)
        tb.toggleViewAction().setStatusTip(tr("Show or hide this toolbar."))
        self.addToolBar(Qt.TopToolBarArea, tb)
        from views.icons import style_overflow_button
        style_overflow_button(tb)
        return tb

    def _add_tool_button(self, tb: QToolBar, key: str) -> QAction:
        tool = self._tools[key]
        name = tr(tool.name)
        action = QAction(tool_icon(key), name, self)
        self._icon_actions.append((action, key))
        action.setCheckable(True)
        if tool.shortcut:
            seqs = [QKeySequence(tool.shortcut)]
            alt = getattr(tool, "shortcut_alt", None)
            if alt:
                seqs.append(QKeySequence(alt))
            action.setShortcuts(seqs)
        if tool.description:
            action.setStatusTip(tr(tool.description))
        from views.shortcuts import set_tooltip
        set_tooltip(action, name)
        from views.view_commands import bind, entries
        if any(row[1] == key and row[0] == "walkthrough" for row in entries()):
            bind(self, action, key)
        else:
            action.triggered.connect(lambda _c, k=key: self._activate_tool(k))
        self._tool_group.addAction(action)
        tb.addAction(action)
        self._tool_actions[key] = action
        return action

    def _build_toolbar(self) -> None:
        self._tool_group = QActionGroup(self)
        self._tool_group.setExclusive(True)
        self.toolbars: dict[str, QToolBar] = {}
        # Qt host toolbars use their fixed icon size.
        # (action, icon_key) pairs so programmatic icons can be re-drawn when
        # the palette flips (dark ↔ light) at runtime — see changeEvent below.
        self._icon_actions: list[tuple[QAction, str]] = []

        # One toolbar per task, each independently movable.
        layout = [
            ("main", tr("Main"), ["select", "eraser", "paint"]),
            ("draw", tr("Draw"),
             ["line", "freehand", "rectangle", "rotated_rect", "circle",
              "polygon", "arc", "arc3", "center_arc", "pie"]),
            # Push/Pull first (Marco, 2026-09-14), then move, rotate, scale, flip…
            ("modify", tr("Modify"), ["pushpull", "move", "rotate", "scale", "flip", "followme", "offset", "fillet"]),
            ("annotate", tr("Annotate"), ["tape", "protractor", "north", "dimension", "text", "geopath"]),
            ("sections", tr("Sections"), ["section"]),
            # The Solid Tools toolbar, in the usual order.
            ("solids", tr("Solid Tools"),
             ["outer_shell", "solid_union", "solid_subtract", "solid_trim",
              "solid_intersect", "solid_split"]),
            # The Walkthrough toolbar, in the usual order.
            ("walkthrough", tr("Walkthrough"),
             ["position_camera", "walk", "look_around", "first_person"]),
        ]
        for oname, title, keys in layout:
            tb = self._new_toolbar(title, oname)
            self.toolbars[oname] = tb
            for key in keys:
                self._add_tool_button(tb, key)

        # Bridge (issue #614): a one-shot command, not a tool, so a plain
        # button after Fillet; Tools ▸ Bridge is this same action. No key
        # of its own: Repeat (Shift+R) does the next one, and Preferences
        # can give it one.
        from views.shortcuts import set_tooltip
        self._act_bridge = QAction(tool_icon("bridge"), tr("Bridge"), self)
        self._act_bridge.setStatusTip(tr(
            "Join two faces, a face and a line, or two lines with the faces "
            "between them."))
        set_tooltip(self._act_bridge, tr("Bridge"))
        self._act_bridge.triggered.connect(self._on_bridge)
        self.toolbars["modify"].addAction(self._act_bridge)
        self._icon_actions.append((self._act_bridge, "bridge"))
        # Make Faces: the faces the selected edges close, in place (lines
        # imported from a drawing file come in without them). A one-shot
        # command beside Bridge, the other one that builds faces.
        self._act_make_faces = QAction(tool_icon("make_faces"),
                                       tr("Make Faces"), self)
        self._act_make_faces.setStatusTip(tr(
            "Make the faces that the selected edges close, on those same "
            "edges."))
        set_tooltip(self._act_make_faces, tr("Make Faces"))
        self._act_make_faces.triggered.connect(self._on_make_faces)
        self.toolbars["modify"].addAction(self._act_make_faces)
        self._icon_actions.append((self._act_make_faces, "make_faces"))

        # Save, first on the Main bar (the composer's Sheet bar has had it
        # since the start; Marco, 2026-09-14: «el icono de guardar también en
        # modelo, antes de la flechita»). The shortcut stays on the menu
        # action, so Ctrl+S never becomes ambiguous.
        main_tb = self.toolbars["main"]
        act_save = QAction(tool_icon("save"), tr("Save"), self)
        act_save.setToolTip(f"{tr('Save')}  (Ctrl+S)")
        act_save.setStatusTip(tr(_SAVE_TIP))
        act_save.triggered.connect(self._on_save)
        first = main_tb.actions()[0] if main_tb.actions() else None
        main_tb.insertAction(first, act_save)
        main_tb.insertSeparator(first)
        self._icon_actions.append((act_save, "save"))
        self._act_save_tb = act_save

        # A pipette sits beside the material you paint with: it is
        # how you FIND the eyedropper. Alt+click does the same for people who
        # know the modifier — Marco asked for the button because that is what
        # he reaches for ("hay un icono al costado de pintura").
        self._act_eyedropper = QAction(
            tool_icon("eyedropper"), tr("Sample material"), self)
        self._act_eyedropper.setCheckable(True)
        from views.shortcuts import set_tooltip
        self._act_eyedropper.setStatusTip(tr(
            "Click a face to pick up its paint or texture "
            "(or hold Alt with the Paint tool)."))
        set_tooltip(self._act_eyedropper, tr("Sample material"))
        self._act_eyedropper.toggled.connect(self._on_toggle_eyedropper)
        main_tb.addAction(self._act_eyedropper)
        self._icon_actions.append((self._act_eyedropper, "eyedropper"))

        # The Sections toolbar carries the three usual display toggles next
        # to the tool: Display Section Planes / Cuts / Fill. Created here
        # (the menubar builds later and reuses the same actions).
        sec_tb = self.toolbars["sections"]
        sec_tb.addSeparator()

        from views.view_commands import bind, entries

        def _sec_toggle(key: str, text: str, tip: str):
            act = QAction(tool_icon(key), text, self)
            act.setCheckable(True)
            act.setChecked(True)
            act.setToolTip(text)
            act.setStatusTip(tip)
            bind(self, act, key, toggle=True)
            self._icon_actions.append((act, key))
            sec_tb.addAction(act)
            return act

        self._act_show_splanes = _sec_toggle(
            "section_planes", tr("Section Planes"),
            tr("Show the section planes themselves; the cut they make "
               "stays either way."))
        self._act_show_scuts = _sec_toggle(
            "section_cuts", tr("Section Cuts"),
            tr("Cut the model open at its active section planes; off, the "
               "model shows whole."))
        self._act_section_fill = _sec_toggle(
            "section_fill", tr("Section Fill"),
            tr("Fill the cut faces of a section with a solid colour."))

        # 3D Text opens a dialog (it's a one-shot action, not a checkable tool),
        # so it gets its own button on the Annotate bar next to the 2D Text tool.
        self._act_3dtext = QAction(tool_icon("text3d"), tr("3D Text"), self)
        self._act_3dtext.setStatusTip(tr(_TEXT3D_TIP))
        self._act_3dtext.triggered.connect(self._on_insert_3d_text)
        self.toolbars["annotate"].addAction(self._act_3dtext)
        self._icon_actions.append((self._act_3dtext, "text3d"))

        # Spacebar returns to Select, the usual convention ("S" now
        # belongs to Scale, as users of push/pull modellers expect).
        select_action = self._tool_actions["select"]
        select_action.setShortcuts([QKeySequence(Qt.Key_Space)])

        # View toolbar: camera nav (Orbit / Pan / Zoom / Zoom Window) + Zoom
        # Extents + iso view.
        view_tb = self._new_toolbar(tr("View"), "view")
        self.toolbars["view"] = view_tb
        self._nav_actions: dict[str, QAction] = {}
        nav_tips = dict(orbit="Left-drag to rotate the view.", pan="Left-drag to slide the view.",
                        zoom="Drag up or down to zoom in or out.", zoom_window="Drag a box to zoom to that region.")
        for group, key, label, _, _ in entries():
            if key not in nav_tips:
                continue
            tip = nav_tips[key]
            action = QAction(tool_icon(key), tr(label), self)
            action.setCheckable(True)
            bind(self, action, key)
            action.setStatusTip(tr(tip))
            set_tooltip(action, tr(label))  # names the keys, as tools do
            self._tool_group.addAction(action)
            view_tb.addAction(action)
            self._nav_actions[key] = action
            self._icon_actions.append((action, key))
        view_tb.addSeparator()
        # ONE action for Zoom Extents, shared with the Camera menu below.
        # It used to be built twice — same key on two QActions is a Qt
        # ambiguity, and an ambiguous shortcut fires NEITHER, so F2 did
        # nothing at all. Shift+Z is the usual key for it; F2 stays as
        # an alternate because it is the one that was documented here.
        act_ze = QAction(tool_icon("zoom_extents"), tr("Zoom Extents"), self)
        self._icon_actions.append((act_ze, "zoom_extents"))
        bind(self, act_ze, "zoom_extents")
        act_ze.setStatusTip(tr("Frame the whole model in the view."))
        from views.shortcuts import set_tooltip
        set_tooltip(act_ze, tr("Zoom Extents"))
        view_tb.addAction(act_ze)
        self._act_zoom_extents = act_ze

        # Zoom Selection: the same framing over the selection alone. With
        # nothing selected it does nothing (Zoom Extents is one key away).
        act_zs = QAction(tool_icon("zoom_selection"), tr("Zoom Selection"), self)
        self._icon_actions.append((act_zs, "zoom_selection"))
        bind(self, act_zs, "zoom_selection")
        act_zs.setStatusTip(tr("Frame the selection in the view."))
        set_tooltip(act_zs, tr("Zoom Selection"))
        view_tb.addAction(act_zs)
        self._act_zoom_selection = act_zs

        # Standard-views toolbar: one-shot camera orientations, icon-only.
        views_tb = self._new_toolbar(tr("Standard Views"), "views")
        self.toolbars["views"] = views_tb
        # Order as Marco reads them (2026-09-14): iso, top, front, right,
        # left, back, bottom — the two you use most right after the iso.
        # The keys go by comfort, not by this order: the left hand holds
        # Ctrl with the little finger and reaches 1-4 without leaving the
        # keys, so the four daily views (plan, elevation, side, 3D) take
        # them and the rare ones sit further away. The digits alone type a
        # measure in the value box (views/shortcuts.py, reserved_reason).
        self._view_actions = {}
        for group, command, label, _, icon in entries():
            if group != "Views" or not command.startswith("view_"):
                continue
            key = command.removeprefix("view_")
            act = QAction(tool_icon(icon), tr(label), self)
            bind(self, act, command)
            act.setStatusTip(tr(_VIEW_TIPS[key]))
            set_tooltip(act, tr(label))
            views_tb.addAction(act)
            self._icon_actions.append((act, icon))
            self._view_actions[key] = act

        # Projection quick buttons, one click each instead of Camera ▸:
        # parallel, perspective, two-point (verticals kept vertical, the
        # usual view for an architectural presentation). One of the three
        # is always down; the viewport says when anything else (Shift+P, a
        # scene, a saved view) changed the projection.
        views_tb.addSeparator()
        self._projection_group = QActionGroup(self)
        self._projection_group.setExclusive(True)
        self._projection_actions: dict[str, QAction] = {}
        for mode, label, tip in [
            ("parallel", "Parallel Projection",
             "No vanishing points: measured elevations, plans and "
             "isometrics."),
            ("perspective", "Perspective",
             "The view as the eye sees it."),
            ("two_point", "Two-Point Perspective",
             "Vertical lines stay vertical, the usual view for "
             "architectural presentations."),
        ]:
            icon = "proj_" + mode
            act = QAction(tool_icon(icon), tr(label), self)
            act.setObjectName("projection_" + mode)
            act.setCheckable(True)
            act.setStatusTip(tr(tip))
            set_tooltip(act, tr(label))     # names the keys, if given any
            bind(self, act, "projection_" + mode)
            self._projection_group.addAction(act)
            views_tb.addAction(act)
            self._icon_actions.append((act, icon))
            self._projection_actions[mode] = act
        self._sync_projection_buttons(self.viewport.camera.projection_mode())

    def _describe_buttons(self) -> None:
        """A toolbar button's tooltip says what it does under its name and
        keys, as Blender's do: the action's status tip, the sentence the
        status bar shows for its menu entry. Run once the menus — which
        give some of the toolbar actions their tip — are built."""
        for tb in self.findChildren(QToolBar):
            for act in tb.actions():
                tip = act.statusTip()
                if tip and tip not in act.toolTip():
                    act.setToolTip(f"{act.toolTip()}\n{tip}")

    def _wake_link_watch(self):
        timer = self.__dict__.get("_link_poll_timer")
        if timer is None:
            timer = self._link_poll_timer = QTimer(self, singleShot=True)
            timer.timeout.connect(self._poll_links)
        timer.start(0)

    def _poll_links(self):
        delay = self._link_watcher().poll()
        if delay is not None:
            self._link_poll_timer.start(delay)

    def _sync_projection_buttons(self, mode: str) -> None:
        act = self._projection_actions.get(mode)
        if act is not None and not act.isChecked():
            act.setChecked(True)

    def _set_projection(self, mode: str) -> None:
        """Parallel, perspective or two-point, in the active view only."""
        vp = self._active_view()
        vp.set_projection(mode)
        self._sync_projection_buttons(vp.camera.projection_mode())

    def _refresh_toolbar_icons(self) -> None:
        """Re-draw the programmatic toolbar icons for the current palette so a
        dark ↔ light theme switch (while the app is open) doesn't leave the
        icons in the previous theme's ink — they were baked at build time."""
        for action, key in getattr(self, "_icon_actions", []):
            action.setIcon(tool_icon(key))
        for dock in self._sidebar_docks():
            key = dock.property("icon_key")
            if key:
                dock.setWindowIcon(tool_icon(str(key)))
        browser = getattr(getattr(self, "project_browser", None),
                          "outliner", None)
        if browser is not None:
            browser.refresh_icons()

    def changeEvent(self, event) -> None:
        # Qt posts these when the OS/Qt theme (palette) flips at runtime.
        if event.type() in (
            QEvent.ApplicationPaletteChange,
            QEvent.PaletteChange,
            QEvent.ThemeChange,
        ):
            self._refresh_toolbar_icons()
        super().changeEvent(event)

    def _build_menubar(self) -> None:
        menubar = self.menuBar()

        # File menu
        file_menu = menubar.addMenu(tr("File"))
        for action in self._file_actions():
            if isinstance(action, QMenu):
                file_menu.addMenu(action)
            else:
                file_menu.addAction(action)

        # Edit menu
        edit_menu = menubar.addMenu(tr("Edit"))
        self._act_simplify_mesh = QAction(tr("Simplify Mesh…"), self)
        self._act_simplify_mesh.setToolTip(tr(
            "Simplify Mesh — merge coplanar and near-coplanar faces"))
        self._act_simplify_mesh.triggered.connect(self._on_simplify_mesh)

        self._undo_action = QAction(tr("Undo"), self)
        self._undo_action.setStatusTip(tr(
            "Take back the last change to the model."))
        self._undo_action.setShortcut(QKeySequence.Undo)
        self._undo_action.triggered.connect(self._on_undo)
        edit_menu.addAction(self._undo_action)

        self._redo_action = QAction(tr("Redo"), self)
        self._redo_action.setStatusTip(tr(
            "Bring back the change that was just undone."))
        # Both keys, spelled out: Ctrl+Y (Windows' habit) and Ctrl+Shift+Z
        # (Linux and macOS). The platform's «Redo» key is Ctrl+Shift+Z on
        # KDE and GNOME, so it came out twice and Ctrl+Y not at all — and a
        # key listed twice on one action did not fire (#313, @RaphaelNunes10).
        self._redo_action.setShortcuts(
            [QKeySequence("Ctrl+Y"), QKeySequence("Ctrl+Shift+Z")]
        )
        self._redo_action.triggered.connect(self._on_redo)
        edit_menu.addAction(self._redo_action)

        # On the Main toolbar right after the pointer, the usual place
        # (Marco, 23-09). The SAME actions as the Edit menu: a second QAction
        # with Ctrl+Z would make the shortcut ambiguous, and an ambiguous
        # shortcut fires neither (the F2 lesson on Zoom Extents).
        main_tb = getattr(self, "toolbars", {}).get("main")
        if main_tb is not None:
            from views.icons import tool_icon
            for act, key, tip in ((self._undo_action, "undo", tr("Undo")),
                                  (self._redo_action, "redo", tr("Redo"))):
                act.setIcon(tool_icon(key))
                from views.shortcuts import set_tooltip
                set_tooltip(act, tip)
                self._icon_actions.append((act, key))
            after = self._tool_actions.get("select")
            acts = main_tb.actions()
            nxt = (acts[acts.index(after) + 1]
                   if after in acts and acts.index(after) + 1 < len(acts)
                   else None)
            for act in (self._undo_action, self._redo_action):
                if nxt is None:
                    main_tb.addAction(act)
                else:
                    main_tb.insertAction(nxt, act)

        edit_menu.addSeparator()

        cut_action = QAction(tr("Cut"), self)
        cut_action.setStatusTip(tr(
            "Take the selection out of the model and keep it to paste "
            "elsewhere."))
        cut_action.setShortcut(QKeySequence.Cut)
        cut_action.triggered.connect(lambda: self.viewport.cut_selection())
        edit_menu.addAction(cut_action)

        copy_action = QAction(tr("Copy"), self)
        copy_action.setStatusTip(tr(
            "Keep a copy of the selection to paste here or in another "
            "Built window."))
        copy_action.setShortcut(QKeySequence.Copy)
        copy_action.triggered.connect(lambda: self.viewport.copy_selection())
        edit_menu.addAction(copy_action)

        paste_action = QAction(tr("Paste"), self)
        paste_action.setStatusTip(tr(
            "Place what was cut or copied, with a click where it goes."))
        paste_action.setShortcut(QKeySequence.Paste)
        paste_action.triggered.connect(self._on_paste)
        edit_menu.addAction(paste_action)

        paste_in_place_action = QAction(tr("Paste in Place"), self)
        paste_in_place_action.setStatusTip(tr(
            "Paste the copy exactly where the original was, in the group "
            "that is open — to move things into or out of groups without "
            "shifting them."))
        paste_in_place_action.setShortcut(QKeySequence("Ctrl+Alt+V"))
        paste_in_place_action.triggered.connect(self._on_paste_in_place)
        edit_menu.addAction(paste_in_place_action)

        edit_menu.addSeparator()

        select_all_action = QAction(tr("Select All"), self)
        select_all_action.setStatusTip(tr(
            "Select everything: edges, faces, groups and dimensions."))
        select_all_action.setShortcut(QKeySequence.SelectAll)
        select_all_action.triggered.connect(self._on_select_all)
        edit_menu.addAction(select_all_action)

        # Edit ▸ Invert Selection, the usual shortcut.
        invert_action = QAction(tr("Invert Selection"), self)
        invert_action.setStatusTip(tr(
            "Select what is not selected, and drop what is."))
        invert_action.setShortcut(QKeySequence("Ctrl+Shift+I"))
        invert_action.triggered.connect(self._on_invert_selection)
        edit_menu.addAction(invert_action)

        # Edit ▸ Select none, the usual shortcut.
        select_none_action = QAction(tr("Select None"), self)
        select_none_action.setStatusTip(tr(
            "Deselect everything: edges, faces, groups and dimensions."))
        select_none_action.setShortcut(QKeySequence("Ctrl+T"))
        select_none_action.triggered.connect(self._on_select_none)
        edit_menu.addAction(select_none_action)

        edit_menu.addSeparator()

        group_action = QAction(tr("Make Group"), self)
        group_action.setStatusTip(tr(
            "Wrap the selection in a group that moves as one and keeps "
            "apart from the geometry around it."))
        group_action.setShortcut(QKeySequence("Ctrl+G"))
        group_action.triggered.connect(self._on_make_group)
        edit_menu.addAction(group_action)

        component_action = QAction(tr("Make Component…"), self)
        component_action.setStatusTip(tr(
            "Turn the selection into a component: every copy shares it, "
            "so changing one changes them all."))
        component_action.setShortcut(QKeySequence("G"))   # the usual G
        component_action.triggered.connect(self._on_make_component)
        edit_menu.addAction(component_action)

        explode_action = QAction(tr("Explode Group"), self)
        explode_action.setStatusTip(tr(
            "Break the selected groups apart, back into the geometry "
            "around them."))
        explode_action.setShortcut(QKeySequence("Ctrl+Shift+G"))
        explode_action.triggered.connect(self._on_explode_group)
        edit_menu.addAction(explode_action)

        # Edit ▸ Intersect Faces (core/intersect.py).
        self._intersect_menu = QMenu(tr("Intersect Faces"), edit_menu)
        self._fill_intersect_menu(self._intersect_menu)
        edit_menu.addMenu(self._intersect_menu)

        split_action = QAction(tr("Split into Pieces"), self)
        split_action.setStatusTip(tr(
            "Regroup the selected group by the pieces that do not touch; "
            "explode it afterwards for each piece on its own."))
        split_action.triggered.connect(self._on_split_into_pieces)
        edit_menu.addAction(split_action)

        edit_menu.addAction(self._act_simplify_mesh)

        convert_path_action = QAction(tr("Convert Path to Geometry"), self)
        convert_path_action.setStatusTip(tr(
            "Turn the selected paths into edges of the model; a closed "
            "path becomes a face, ready to push up."))
        convert_path_action.triggered.connect(self._on_convert_geopath)
        edit_menu.addAction(convert_path_action)

        delete_guides_action = QAction(tr("Delete Guides"), self)
        delete_guides_action.setStatusTip(tr(
            "Remove every guide line and guide point from the model."))
        delete_guides_action.triggered.connect(self._on_delete_guides)
        edit_menu.addAction(delete_guides_action)

        edit_menu.addSeparator()

        # Edit ▸ Hide and Edit ▸ Unhide ▸ Last / All. Hide takes
        # the selected OBJECTS (groups, components) and edges; there was
        # only «Ocultar aristas» and Rafael looked for the object one and
        # did not find it (2026-09-16, 38:40). The keys sit on Ctrl+H
        # because H alone is Pan and Shift+H the Protractor; Shift adds
        # the way back for everything.
        hide_action = QAction(tr("Hide"), self)
        hide_action.setShortcut(QKeySequence("Ctrl+H"))
        hide_action.setStatusTip(tr(
            "Stop showing the selected objects, faces and edges; they "
            "stay in the document until unhidden."))
        hide_action.triggered.connect(self._on_hide)
        edit_menu.addAction(hide_action)

        unhide_menu = edit_menu.addMenu(tr("Unhide"))
        unhide_selected_action = QAction(tr("Selected"), self)
        unhide_selected_action.setStatusTip(tr(
            "Show again the hidden things that are selected — Hidden "
            "Objects or Hidden Geometry lets you select them."))
        unhide_selected_action.triggered.connect(self._on_unhide_selected)
        unhide_menu.addAction(unhide_selected_action)
        unhide_last_action = QAction(tr("Last"), self)
        unhide_last_action.setStatusTip(tr(
            "Show again what the last hiding put away."))
        unhide_last_action.triggered.connect(self._on_unhide_last)
        unhide_menu.addAction(unhide_last_action)
        unhide_all_action = QAction(tr("All"), self)
        unhide_all_action.setShortcut(QKeySequence("Ctrl+Shift+H"))
        unhide_all_action.setStatusTip(tr(
            "Show again everything that is hidden."))
        unhide_all_action.triggered.connect(self._on_unhide_all)
        unhide_menu.addAction(unhide_all_action)

        soften_action = QAction(tr("Soften / Smooth Edges…"), self)
        soften_action.setStatusTip(tr(
            "Smooth the selected edges, faces or groups where their faces "
            "meet at a shallow angle."))
        soften_action.triggered.connect(self._on_soften_edges)
        edit_menu.addAction(soften_action)

        reverse_action = QAction(tr("Reverse Faces"), self)
        reverse_action.setStatusTip(tr(
            "Swap the front and back sides of the selected faces."))
        reverse_action.triggered.connect(self._on_reverse_faces)
        edit_menu.addAction(reverse_action)

        orient_action = QAction(tr("Orient Faces"), self)
        orient_action.setStatusTip(tr(
            "Turn every face connected to the selected one so its front "
            "side matches."))
        orient_action.triggered.connect(self._on_orient_faces)
        edit_menu.addAction(orient_action)

        heal_action = QAction(tr("Heal Overlapping Faces"), self)
        heal_action.setStatusTip(tr(
            "Remove the faces left lying over their own subdivisions "
            "after drawing or erasing."))
        heal_action.triggered.connect(self._on_heal_overlaps)
        edit_menu.addAction(heal_action)

        join_action = QAction(tr("Join Segments"), self)
        join_action.setStatusTip(tr(
            "Join collinear segments into single lines: the selected ones, "
            "or all of them when nothing is selected."))
        join_action.triggered.connect(self._on_join_segments)
        edit_menu.addAction(join_action)

        rebuild_action = QAction(tr("Rebuild Faces (Planar)"), self)
        rebuild_action.setStatusTip(tr(
            "Work the faces out again from the edges: on the plane of the "
            "selected faces, or over the whole drawing when it is flat."))
        rebuild_action.triggered.connect(self._on_rebuild_planar)
        edit_menu.addAction(rebuild_action)

        # Camera menu (navigation + projection + canned views)
        camera_menu = menubar.addMenu(tr("Camera"))

        standard_menu = camera_menu.addMenu(tr("Standard Views"))
        # The SAME actions as the toolbar buttons: two actions holding one
        # key leave it dead (tests/test_shortcuts.py).
        for key in ("top", "bottom", "front", "back", "left", "right", "iso"):
            standard_menu.addAction(self._view_actions[key])

        camera_menu.addAction(self._act_zoom_extents)   # la MISMA del botón
        camera_menu.addAction(self._act_zoom_selection)

        camera_menu.addSeparator()

        action_proj = QAction(tr("Toggle Perspective / Parallel"), self)
        action_proj.setStatusTip(tr(
            "Switch between perspective and a parallel projection, where "
            "sizes do not shrink with distance."))
        # P went back to Push/Pull, the usual convention; the
        # projection toggle usually has no key at all, so it takes Shift+P —
        # the same "the tool that yields keeps Shift+key" rule as the
        # Protractor and the centre arc.
        action_proj.setShortcut(QKeySequence("Shift+P"))
        action_proj.triggered.connect(
            lambda _c=False: self._toggle_projection())
        camera_menu.addAction(action_proj)

        # The Camera menu: Parallel Projection / Perspective /
        # Two-Point Perspective (verticals stay vertical, as an
        # architectural drawing wants them — José Castro Basso,
        # FADU–UDELAR). The SAME actions as the quick buttons on the
        # Standard Views bar, so a shortcut given to one is the other's.
        for mode in ("parallel", "perspective", "two_point"):
            camera_menu.addAction(self._projection_actions[mode])

        # The lens of the 3D view (#826, Alvaro Celis: «lo pongo en 70 para
        # perspectivas más fugadas»). The document's camera and its scenes
        # already keep the angle; the range is the composer's.
        self._act_field_of_view = QAction(tr("Field of View…"), self)
        self._act_field_of_view.setStatusTip(tr(
            "Change the camera's angle of view, like a lens: wider takes "
            "more in, narrower keeps the lines calm."))
        self._act_field_of_view.triggered.connect(self._ask_field_of_view)
        camera_menu.addAction(self._act_field_of_view)
        # Camera Settings: the camera by the numbers — eye and target,
        # focal length, roll, frame shift, clipping and the picture's frame
        # The native sidebar follows the active camera.
        self._act_camera_settings = QAction(
            tool_icon("camera_settings"), tr("Camera Settings…"), self)
        self._icon_actions.append((self._act_camera_settings,
                                   "camera_settings"))
        self._act_camera_settings.setStatusTip(tr(
            "Set the camera exactly: where the eye is and what it looks at, "
            "the lens in millimetres, roll, shift, clipping and frame."))
        self._act_camera_settings.triggered.connect(
            lambda _c=False: self._show_camera_settings())
        camera_menu.addAction(self._act_camera_settings)

        # Styles: the model's display look — face mode, edges,
        # background. Scenes remember the style; the composer's live-look
        # frames inherit it.
        from views.view_commands import bind, entries
        camera_menu.addSeparator()
        style_menu = camera_menu.addMenu(tr("Style"))
        self._style_group = QActionGroup(self)
        self._style_actions: dict[str, QAction] = {}
        for group, command, label, _, _ in entries():
            if group != "Style":
                continue
            act = QAction(tr(label), self)
            act.setStatusTip(tr(_STYLE_TIPS.get(label, "")))
            act.setCheckable(True)
            self._style_group.addAction(act)
            bind(self, act, command)
            style_menu.addAction(act)
            self._style_actions[label] = act
        # X-ray on and off with one key: a glance at what hides behind a
        # face, then back to the style you were in.
        self._style_before_xray = (None, None)
        self._act_xray_toggle = QAction(tr("Toggle X-ray"), self)
        bind(self, self._act_xray_toggle, "xray")
        self._act_xray_toggle.setStatusTip(tr(
            "Switch to X-ray, or back to the style you were in."))
        style_menu.addSeparator()
        style_menu.addAction(self._act_xray_toggle)
        edge_menu = camera_menu.addMenu(tr("Edge Style"))
        self._act_style_edges = QAction(tr("Edges"), self)
        self._act_style_edges.setStatusTip(tr("Draw the edges of the model."))
        self._act_style_edges.setCheckable(True)
        self._act_style_edges.toggled.connect(
            lambda on: self._set_style_field("edges", on))
        edge_menu.addAction(self._act_style_edges)
        self._act_style_profiles = QAction(tr("Profiles"), self)
        self._act_style_profiles.setStatusTip(tr(
            "Draw the outline of each shape with a thicker line."))
        self._act_style_profiles.setCheckable(True)
        self._act_style_profiles.toggled.connect(
            lambda on: self._set_style_field("profiles", on))
        self._act_style_back_edges = QAction(tr("Back edges"), self)
        self._act_style_back_edges.setStatusTip(tr(
            "Draw the edges hidden behind faces as dashed lines."))
        self._act_style_back_edges.setCheckable(True)
        bind(self, self._act_style_back_edges, "back_edges", toggle=True)
        edge_menu.addAction(self._act_style_back_edges)
        edge_menu.addAction(self._act_style_profiles)
        self._sync_style_menu()

        # How the model outside a group reads while you edit it (the usual
        # Model Info ▸ Components setting). Hiding it is also the fastest on a heavy
        # import: what is not in the frame never reaches the GPU.
        rest_menu = camera_menu.addMenu(tr("Rest of model while editing"))
        self._rest_group = QActionGroup(self)
        self._rest_actions: dict[str, QAction] = {}
        for key, label, tip in (
                ("normal", tr("Show normally"),
                 tr("While a group is edited, the rest of the model shows "
                    "as usual.")),
                ("fade", tr("Fade"),
                 tr("While a group is edited, the rest of the model shows "
                    "pale, so the group stands out.")),
                ("hide", tr("Hide (fastest)"),
                 tr("While a group is edited, the rest of the model is not "
                    "drawn — the quickest on a heavy model."))):
            act = QAction(label, self)
            act.setStatusTip(tip)
            act.setCheckable(True)
            act.setChecked(self.viewport.edit_rest_mode == key)
            self._rest_group.addAction(act)
            act.triggered.connect(
                lambda _c=False, k=key: self.viewport.set_edit_rest_mode(k))
            rest_menu.addAction(act)
            self._rest_actions[key] = act

        # A live control next to the three coarse modes. It changes the grey
        # wash only; Hide remains a separate, truly omitted render path.
        rest_menu.addSeparator()
        self._rest_fade_widget = QWidget(rest_menu)
        fade_layout = QHBoxLayout(self._rest_fade_widget)
        fade_layout.setContentsMargins(12, 4, 12, 6)
        fade_layout.addWidget(QLabel(tr("Fade intensity:")))
        self._rest_fade_slider = QSlider(Qt.Horizontal)
        self._rest_fade_slider.setRange(0, 100)
        self._rest_fade_slider.setFixedWidth(140)
        style_slider(self._rest_fade_slider)
        self._rest_fade_slider.setToolTip(tr(
            "How strongly the rest of the model is washed toward grey while "
            "a group or component is open."))
        self._rest_fade_value = QLabel()
        self._rest_fade_value.setMinimumWidth(42)
        fade_layout.addWidget(self._rest_fade_slider)
        fade_layout.addWidget(self._rest_fade_value)
        self._rest_fade_slider.valueChanged.connect(
            self._on_edit_rest_fade_changed)
        fade_action = QWidgetAction(rest_menu)
        fade_action.setDefaultWidget(self._rest_fade_widget)
        rest_menu.addAction(fade_action)

        # Camera ▸ Component Edit, with keys of their own: Alt+Q
        # toggles the rest of the model between hidden and the mode it had
        # (fade or normal), Alt+W hides the other copies of the component
        # being edited.
        comp_menu = camera_menu.addMenu(tr("Component Edit"))
        self._act_hide_rest = QAction(tr("Hide Rest of Model"), self)
        self._act_hide_rest.setCheckable(True)
        self._act_hide_rest.setShortcut(QKeySequence("Alt+Q"))
        self._act_hide_rest.triggered.connect(self._on_toggle_hide_rest)
        comp_menu.addAction(self._act_hide_rest)
        self._act_hide_similar = QAction(tr("Hide Similar Components"), self)
        self._act_hide_similar.setCheckable(True)
        self._act_hide_similar.setShortcut(QKeySequence("Alt+W"))
        self._act_hide_similar.triggered.connect(self._on_toggle_hide_similar)
        comp_menu.addAction(self._act_hide_similar)
        self._rest_shown_mode = (self.viewport.edit_rest_mode
                                 if self.viewport.edit_rest_mode != "hide"
                                 else "fade")
        self._sync_component_edit_menu()
        camera_menu.aboutToShow.connect(self._sync_component_edit_menu)

        # Sun shadows (core/sun.py) — the checkbox mirrors the tray panel.
        self._act_shadows = QAction(tr("Shadows"), self)
        self._act_shadows.setStatusTip(tr(
            "Cast the sun's shadows; the Shadows panel sets the date, the "
            "time and how dark they are."))
        self._act_shadows.setCheckable(True)
        bind(self, self._act_shadows, "shadows", toggle=True)
        camera_menu.addAction(self._act_shadows)

        # View ▸ Section Planes / Cuts / Fill — the same actions
        # as the Sections toolbar buttons (created in _build_toolbar).
        camera_menu.addSeparator()
        camera_menu.addAction(self._act_show_splanes)
        camera_menu.addAction(self._act_show_scuts)
        camera_menu.addAction(self._act_section_fill)

        # View ▸ Hidden Objects / Hidden Geometry: what Hide put
        # away comes back as a see-through grid and can be selected again
        # (Marco, 2026-09-18, with two reference captures).
        camera_menu.addSeparator()
        self._act_hidden_objects = QAction(tr("Hidden Objects"), self)
        self._act_hidden_objects.setStatusTip(tr(
            "Show hidden groups and components as a see-through grid, so "
            "they can be selected again."))
        self._act_hidden_objects.setCheckable(True)
        self._act_hidden_objects.toggled.connect(
            lambda on: self._set_hidden_view("show_hidden_objects", on))
        camera_menu.addAction(self._act_hidden_objects)
        self._act_hidden_geometry = QAction(tr("Hidden Geometry"), self)
        self._act_hidden_geometry.setStatusTip(tr(
            "Show softened and hidden edges as dashed lines, and hidden "
            "faces as a see-through grid."))
        self._act_hidden_geometry.setCheckable(True)
        bind(self, self._act_hidden_geometry, "hidden_geometry", toggle=True)
        edge_menu.addSeparator()
        edge_menu.addAction(self._act_hidden_geometry)
        camera_menu.addSeparator()
        for action in self._nav_actions.values():   # Orbit / Pan / Zoom / Zoom Window
            camera_menu.addAction(action)
        # Camera ▸ Position Camera / Walk / Look Around.
        camera_menu.addSeparator()
        for key in ("position_camera", "walk", "look_around", "first_person"):
            camera_menu.addAction(self._tool_actions[key])

        # Draw menu (the drawing tools, grouped by family)
        draw_menu = menubar.addMenu(tr("Draw"))
        draw_menu.addAction(self._tool_actions["line"])
        draw_menu.addAction(self._tool_actions["freehand"])
        arcs_menu = draw_menu.addMenu(tr("Arcs"))
        for key in ("arc", "arc3", "center_arc", "pie"):
            arcs_menu.addAction(self._tool_actions[key])
        shapes_menu = draw_menu.addMenu(tr("Shapes"))
        for key in ("rectangle", "rotated_rect", "circle", "polygon"):
            shapes_menu.addAction(self._tool_actions[key])
        draw_menu.addSeparator()
        draw_menu.addAction(self._tool_actions["geopath"])

        # Tools menu (select/modify/measure — drawing lives in Draw)
        tools_menu = menubar.addMenu(tr("Tools"))
        for keys in (("select", "eraser", "paint"),
                     ("move", "rotate", "scale", "flip"),
                     ("pushpull", "followme", "offset", "fillet"),
                     ("tape", "protractor", "north"),
                     ("dimension", "text"),
                     ("section",)):
            for key in keys:
                tools_menu.addAction(self._tool_actions[key])
            if "followme" in keys:
                # Bridge (issue #614), with the tools that build faces
                # out of others: the Modify toolbar's own action.
                tools_menu.addAction(self._act_bridge)
                tools_menu.addAction(self._act_make_faces)
            tools_menu.addSeparator()
        # Tools ▸ Outer Shell, and Tools ▸ Solid Tools ▸ the rest.
        tools_menu.addAction(self._tool_actions["outer_shell"])
        solids_menu = QMenu(tr("Solid Tools"), tools_menu)
        for key in ("solid_intersect", "solid_union", "solid_subtract",
                    "solid_trim", "solid_split"):
            solids_menu.addAction(self._tool_actions[key])
        tools_menu.addMenu(solids_menu)
        self._solids_menu = solids_menu        # a QMenu dies with its locals
        tools_menu.addSeparator()
        action_3dtext = QAction(tool_icon("text3d"), tr("3D Text…"), self)
        action_3dtext.setStatusTip(tr(_TEXT3D_TIP))
        action_3dtext.triggered.connect(self._on_insert_3d_text)
        tools_menu.addAction(action_3dtext)
        self._icon_actions.append((action_3dtext, "text3d"))
        tools_menu.addSeparator()
        action_profile = QAction(tr("Terrain profile of selection"), self)
        action_profile.setStatusTip(tr(
            "Draw the terrain profile along the selected path in the "
            "profile panel."))
        action_profile.triggered.connect(self._on_terrain_profile)
        tools_menu.addAction(action_profile)
        tools_menu.addSeparator()
        action_cancel = QAction(tr("Cancel current tool"), self)
        action_cancel.setStatusTip(tr(
            "Stop what the tool is doing; with nothing in progress, clear "
            "the selection."))
        action_cancel.setShortcut(QKeySequence("Esc"))
        action_cancel.triggered.connect(self._cancel_tool)
        tools_menu.addAction(action_cancel)

        # Window menu (panels + app preferences)
        window_menu = menubar.addMenu(tr("Window"))
        window_menu.addAction(tr("Cut list…"), self._on_cut_list)

        # Every tray, extension tabs included, has its entry here — and the
        # same list opens on a right-click over the tabs. Only this list
        # says a tray is unwanted: that choice is remembered and every other
        # tray opens at start-up (see _show_default_trays). Built when it
        # opens: extension tabs are added after the menus.
        # macOS glass: the toolbars are locked and grip-less; they are
        # rearranged in a mode of their own, as in AppKit apps.
        from views import mac_glass
        self._act_customize_toolbars = window_menu.addAction(
            tr("Customize Toolbars…"))
        self._act_customize_toolbars.setStatusTip(tr(
            "Rearrange the toolbars: drag them by their grips, then click "
            "Done."))
        self._act_customize_toolbars.triggered.connect(
            lambda: mac_glass.set_customizing(self, True))
        self._act_customize_toolbars.setVisible(
            self.testAttribute(Qt.WA_TranslucentBackground))

        window_menu.addAction(tr("Terrain profile"), self._on_terrain_profile)

        # Ctrl+U: the units panel straight away (the Preferences page).
        units_action = QAction(tr("Units…"), self)
        units_action.setShortcut(QKeySequence("Ctrl+U"))
        units_action.setStatusTip(tr(
            "Choose the units lengths, areas, volumes and angles are shown "
            "and typed in."))
        units_action.triggered.connect(self._on_units)
        window_menu.addAction(units_action)
        self._act_units = units_action
        # Ctrl+, (Control + comma): the usual key for Preferences elsewhere.
        prefs_action = QAction(tr("Preferences…"), self)
        prefs_action.setShortcut(QKeySequence("Ctrl+,"))
        prefs_action.setStatusTip(tr(
            "Change the units, auto-save, icons and the rest of the "
            "settings."))
        prefs_action.triggered.connect(self._on_preferences)
        window_menu.addAction(prefs_action)
        self._act_preferences = prefs_action

        # Extensions — third-party plugin tools (core.extensions engine).
        self._build_extensions_menu(menubar)

        help_menu = menubar.addMenu(tr("Help"))
        docs_action = QAction(tr("Documentation"), self)
        docs_action.setStatusTip(tr(
            "Open the IngeTrazo documentation in the web browser."))
        docs_action.triggered.connect(self._on_documentation)
        help_menu.addAction(docs_action)
        help_menu.addSeparator()
        get_models_action = QAction(tr("Get more models and textures…"), self)
        get_models_action.setStatusTip(tr(
            "Free websites with models and textures that open in "
            "Built."))
        get_models_action.triggered.connect(self._on_get_models)
        help_menu.addAction(get_models_action)
        # Only as an AppImage: put a launcher in the menu, or take it away.
        from core.appimage import appimage_path
        if appimage_path() is not None:
            add_act = QAction(tr("Add to the applications menu"), self)
            add_act.setStatusTip(tr(
                "Put a launcher for this AppImage in the system's "
                "applications menu."))
            add_act.triggered.connect(self.add_appimage_to_menu)
            help_menu.addAction(add_act)
            rm_act = QAction(tr("Remove from the applications menu"), self)
            rm_act.setStatusTip(tr(
                "Take this AppImage's launcher out of the system's "
                "applications menu."))
            rm_act.triggered.connect(self.remove_appimage_from_menu)
            help_menu.addAction(rm_act)
            help_menu.addSeparator()
        from core.update_check import store_managed
        if not store_managed():
            updates_action = QAction(tr("Check for Updates…"), self)
            updates_action.setStatusTip(tr(
                "See whether a newer version of Built is out."))
            updates_action.triggered.connect(self._on_check_updates)
            help_menu.addAction(updates_action)
        about_action = QAction(tr("About Built"), self)
        about_action.setStatusTip(tr(
            "The version, the authors and the licence of Built."))
        about_action.triggered.connect(self._on_about)
        help_menu.addAction(about_action)

    #: Trays that start closed until the user opens them (Window ▸
    #: Panels): the specialised workspaces. Properties, the Browser and
    #: the bundled AI and Render tabs open. An extension may
    #: ask the same for a panel of its own (``add_panel(start_hidden=True)``).
    DEFAULT_HIDDEN_TRAYS = frozenset({"tray_bim", "tray_georef"})

    def _show_default_trays(self, st) -> None:
        """The trays open at start-up as the user left them in Window ▸
        Panels — a tray stays closed only when it was closed there (Marco,
        24-09: «que siempre se muestren por defecto con opción de ocultarlas
        desde el menú Ventana»). A saved layout alone (a folded sidebar, a
        tray lost by the old fold) does not keep one away. Before any
        choice, the ones most work needs open and the rest wait there
        (``DEFAULT_HIDDEN_TRAYS``)."""
        hidden = self._hidden_tray_names(st)
        docks = self._sidebar_docks()
        for d in docks:
            self._keep_docked(d)
            d.setVisible(d.objectName() not in hidden)
            # Qt's own dock list (right-click on a toolbar) hides one too.
            d.toggleViewAction().triggered.connect(
                lambda on, dock=d: self._remember_tray_choice(dock, on))
        if st.contains("ui/hidden_trays"):
            # Seen now: a panel that asked to start hidden has had its
            # first start; from here on the user's choice rules it.
            st.setValue("ui/hidden_trays", sorted(hidden))
        self._mark_trays_known(st, [d.objectName() for d in docks])
        if not self.tray.isHidden():
            self.tray.raise_()

    def _keep_docked(self, dock) -> None:
        """The trays stay in the sidebar, as Blender's side panel does
        (Marco, 29-09 — floating tabs were tried and dropped): no floating,
        and one a saved layout left floating goes back among the tabs."""
        from PySide6.QtWidgets import QDockWidget
        if sys.platform == "darwin":
            # A restored floating dock retains an NSView when docked again.
            # Qt interprets an off-screen tab's position as safe-area insets
            # (e.g. 1040 x 1596), then adds them to the window minimum.
            # Only the top-level window owns the screen's safe area.
            dock.setAttribute(Qt.WA_ContentsMarginsRespectsSafeArea, False)
            dock.setContentsMargins(0, 0, 0, 0)
        dock.setFeatures(dock.features() & ~QDockWidget.DockWidgetFloatable)
        if dock.isFloating():
            dock.setFloating(False)
            if dock.titleBarWidget() is None:
                dock.setTitleBarWidget(QWidget(dock))
            if self.dockWidgetArea(dock) != Qt.RightDockWidgetArea:
                self.addDockWidget(Qt.RightDockWidgetArea, dock)
            if dock is not self.tray:
                self.tabifyDockWidget(self.tray, dock)

    def _hidden_tray_names(self, st) -> set:
        """The trays kept closed. A first run (nothing chosen yet) opens
        the ones most work needs and leaves the rest in Window ▸ Panels;
        once any is opened or closed there, the whole choice is saved and
        kept. An extension panel that asks to start hidden starts hidden
        the first time it is seen, whenever it is installed."""
        asked = set(getattr(self, "_start_hidden", ()))
        if not st.contains("ui/hidden_trays"):
            return set(self.DEFAULT_HIDDEN_TRAYS) | asked
        raw = st.value("ui/hidden_trays", []) or []
        if isinstance(raw, str):
            raw = [raw]
        known = st.value("ui/known_trays", []) or []
        if isinstance(known, str):
            known = [known]
        return {str(x) for x in raw} | (asked - {str(k) for k in known})

    def _remember_tray_choice(self, dock, shown: bool) -> None:
        st = QSettings()
        hidden = self._hidden_tray_names(st)
        if shown:
            hidden.discard(dock.objectName())
        else:
            hidden.add(dock.objectName())
        st.setValue("ui/hidden_trays", sorted(hidden))
        self._mark_trays_known(st, [dock.objectName()])

    @staticmethod
    def _mark_trays_known(st, names) -> None:
        """Seen: a panel that asked to start hidden has had its first
        start, and the user's choice (``ui/hidden_trays``) rules it now."""
        known = st.value("ui/known_trays", []) or []
        if isinstance(known, str):
            known = [known]
        st.setValue("ui/known_trays",
                    sorted({str(k) for k in known} | set(names)))

    #: The factory order of the side tray's tabs, all on the right:
    #: Properties first, the model's tree next, then the workspaces.
    def _default_tray_order(self) -> list:
        return [self.tray, self.project_browser,
                self.bim_tray, self.georef_tray]

    def _lay_out_trays(self) -> None:
        """The trays in their factory place: every one on the right, tabbed
        in the default order, extension panels after them."""
        order = self._default_tray_order()
        order += [d for d in getattr(self, "_extension_docks", ())
                  if d not in order]
        # Each one straight into the tabs: docked one under the other first,
        # their heights added up past the window, which grew to hold them.
        self.addDockWidget(Qt.RightDockWidgetArea, order[0])
        for first, nxt in zip(order, order[1:]):
            self.tabifyDockWidget(first, nxt)

    def set_tray_shown(self, dock, shown: bool) -> None:
        dock.setVisible(shown)
        if shown:
            dock.raise_()
        self._remember_tray_choice(dock, shown)

    def _sidebar_docks(self) -> list:
        return [d for d in (getattr(self, "tray", None),
                            getattr(self, "bim_tray", None),
                            getattr(self, "georef_tray", None),
                            getattr(self, "project_browser", None),
                            *getattr(self, "_extension_docks", ()))
                if d is not None]

    def _keys_back_to_view(self, _old, new) -> None:
        """Keep the view's keys reachable: focus that lands on a toolbar or
        tray button, or on a tab, without a click (the Tab chain at start-up,
        or a tray hidden while it held the focus, hands it to the next
        button), and focus gone with the window still active, go back to the
        model's view. There, letters still fired the tool shortcuts, but the
        arrows (axis and Down locks), typed values, Enter and Delete reached
        a button and nothing happened. A button clicked keeps its focus (it
        is taken on the press; taking it back mid-click would cancel the
        click), and so does anything in a dialog or a popup."""
        from PySide6.QtWidgets import QAbstractButton, QDockWidget
        if new is not None:
            if not isinstance(new, (QAbstractButton, QTabBar)):
                return
            top = new.window()
            if not (top is self or (isinstance(top, QDockWidget)
                                    and top.parentWidget() is self)):
                return
        if QApplication.mouseButtons() != Qt.NoButton:
            return
        from PySide6.QtCore import QTimer
        QTimer.singleShot(0, self, lambda w=new: self._focus_view_from(w))

    def _focus_view_from(self, stranded) -> None:
        if QApplication.focusWidget() is not stranded:
            return                          # the focus moved on meanwhile
        if (QApplication.activePopupWidget() is not None
                or QApplication.activeModalWidget() is not None
                or QApplication.mouseButtons() != Qt.NoButton):
            return
        if stranded is None and QApplication.activeWindow() is not self:
            return                          # another window, another app
        view = self._shown_viewport()
        if view is not None and view.isVisible():
            view.setFocus(Qt.OtherFocusReason)

    def _shown_viewport(self):
        """The widget the model is drawn in right now: the Metal stand-in
        while it is up, else the GL viewport. The hidden one keeps a stale
        size in the stack, so anything placed from its geometry (sidebar
        handle, clean-screen exit) landed over the view (Maxence,
        2026-11-09, new Mac on Metal)."""
        stand_in = getattr(getattr(self, "_doc", None), "stand_in", None)
        if stand_in is not None and stand_in.isVisible():
            return stand_in
        return self.viewport

    def set_dock_icon(self, dock, icon) -> None:
        """The icon of a side tray's tab: a ``tool_icon`` key — redrawn
        when the theme or the accent colour change — or a QIcon."""
        if isinstance(icon, str):
            dock.setProperty("icon_key", icon)
            icon = tool_icon(icon)
        dock.setWindowIcon(icon)

    def _on_preferences(self) -> None:
        """Window ▸ Preferences: the scattered QSettings in one dialog."""
        self.open_preferences()

    def _build_extensions_menu(self, menubar) -> None:
        """The Extensions menu: one entry per plugin tool discovered by
        :mod:`core.extensions`. The engine guarantees a broken plugin cannot
        break startup — it arrives here as an error entry, shown disabled
        with the exception in its tooltip so the author can fix it."""
        import logging
        from core.extensions import discover_plugins

        ext_menu = menubar.addMenu(tr("Extensions"))
        self._ext_menu = ext_menu           # ExtensionApp.add_menu_action
        # Plugin entries go above this mark; the static entries (examples,
        # plugins folder, guide) sit below it from the start.
        self._ext_anchor = ext_menu.addSeparator()
        # The example extensions (Levels, Polygon properties, Windowizer)
        # install from Manage extensions…, not from a submenu of their own.

        ext_menu.addSeparator()
        manage_action = ext_menu.addAction(tr("Manage extensions…"))
        manage_action.setStatusTip(tr(
            "Browse, install and update community extensions."))
        manage_action.triggered.connect(self._on_manage_extensions)

        # The on-ramp for plugin authors: their folder and the dev guide.
        ext_menu.addSeparator()
        act = ext_menu.addAction(tr("Open extensions folder"))
        act.setStatusTip(tr(
            "Open the folder of your extensions; one put there loads at the "
            "next start."))
        from ingetrazo_render import run_model_menu
        act.triggered.connect(lambda: run_model_menu(self, "plugins_folder"))
        act = ext_menu.addAction(tr("Develop an extension…"))
        act.setStatusTip(tr(
            "Open the guide to writing extensions for IngeTrazo."))
        act.triggered.connect(lambda: run_model_menu(self, "develop_plugin"))

        # The app itself (main.py) defers the plugins' setup until the window
        # is on screen (#440, @cdblue999: 26 third-party extensions held the
        # first frame for 2.5 s). Anything else — tests, scripts — gets them
        # loaded right here, as before.
        if not self._defer_extensions:
            self.load_extensions()

    def load_extensions(self) -> None:
        """Run every discovered plugin's ``setup(app)`` and add its entries
        to the Extensions menu. Once; later calls do nothing.

        Deferred (the app's own start-up), it runs after ``restoreState``:
        a plugin panel finds its saved place by itself
        (:meth:`_register_extension_dock`), but a plugin TOOLBAR is only
        placed by ``restoreState`` — so if one appears, the saved layout is
        applied again, followed by the same steps that follow it at
        start-up."""
        if getattr(self, "_extensions_loaded", False):
            return
        self._extensions_loaded = True
        import logging
        from core.extensions import discover_plugins

        log = logging.getLogger("ingetrazo.plugins")
        ext_menu = self._ext_menu
        anchor = self._ext_anchor
        bars_before = set(self.findChildren(QToolBar))
        plugins, errors = discover_plugins()
        from core.plugin_panels import install
        install(self, plugins)
        from core.extension_manager import extension_startup_state
        self._extension_startup_state = extension_startup_state()

        # Shortcuts the app already claimed (toolbar tools, menus — all built
        # before this menu): first come, first served. A plugin asking for a
        # taken key gets its entry without the shortcut, instead of a Qt
        # ambiguity that silently disables the built-in key for both.
        taken = {a.shortcut().toString() for a in self.findChildren(QAction)
                 if not a.shortcut().isEmpty()}
        # The tools that ship with IngeTrazo (Python Console, Model Info…)
        # keep their keys whatever an extension's setup() asks for first:
        # with Point Cloud Splat installed, the console lost Ctrl+Shift+P
        # (extension bench, 10-10). Reserved here, granted to them below.
        from pathlib import Path
        from core.paths import app_root
        bundled = (app_root() / "plugins").resolve()
        reserved: dict = {}
        for plug in plugins:
            try:
                ships = bundled in Path(plug.path).resolve().parents
            except (OSError, TypeError):
                ships = False
            if not ships:
                continue
            for tool in plug.tools:
                seq = (QKeySequence(tool.shortcut).toString()
                       if tool.shortcut else "")
                if seq and seq not in taken and seq not in reserved:
                    reserved[seq] = f"plugin_{plug.stem}_{type(tool).__name__}"
        taken |= set(reserved)
        self._ext_taken_keys = taken

        count = 0
        from views.extension_api import ExtensionApp
        for plug in list(plugins):
            if plug.setup is None:
                continue
            extension_app = ExtensionApp(self, plug.stem)
            try:
                plug.setup(extension_app)
            # SystemExit too: sys.exit() in a setup closed the whole app.
            except (Exception, SystemExit) as exc:  # noqa: BLE001 — never break startup
                extension_app.remove_agent_tools()
                log.exception("plugin %r setup failed", plug.stem)
                from core.extensions import PluginError, note_failure
                note_failure(f"plugin {plug.stem!r} setup failed")
                errors.append(PluginError(
                    plug.stem, plug.path, f"{type(exc).__name__}: {exc}"))
                plugins.remove(plug)
                continue
            if not plug.tools:
                count += 1                  # a panel-only extension counts
        self._running_extensions = {plug.stem for plug in plugins}
        self._extension_load_errors = {
            err.stem: {"error": err.error, "path": str(err.path)}
            for err in errors
        }
        entries = sorted(((plug, tool) for plug in plugins
                          for tool in plug.tools),
                         key=lambda pt: getattr(pt[1], "menu_order", 0))
        for plug, tool in entries:
            try:
                self._add_plugin_tool_entry(plug, tool, taken, reserved)
                count += 1
            except Exception:  # noqa: BLE001 — the others still get theirs
                log.exception("plugin %r: bad menu entry", plug.stem)
                from core.extensions import note_failure
                note_failure(f"plugin {plug.stem!r}: bad menu entry")

        for err in errors:
            action = QAction(tr("\u26a0 {name} (load error)", name=err.stem), self)
            action.setEnabled(False)
            action.setToolTip(err.error)
            ext_menu.insertAction(anchor, action)

        if count == 0 and not errors:
            action = QAction(tr("(no plugins found)"), self)
            action.setEnabled(False)
            ext_menu.insertAction(anchor, action)

        # The user's keys for the extensions' entries (#595). The window put
        # the remembered keys on its actions while it was being built, and
        # since #440 the extensions come after it shows: their entries did
        # not exist yet, so a key given to one lasted until the restart.
        # Their own keys are noted first as their factory keys («Reset»).
        from views import shortcuts as _shortcuts
        _shortcuts.remember_defaults(self)
        _shortcuts.apply_user_shortcuts(self)

        if self._defer_extensions and set(self.findChildren(QToolBar)) - bars_before:
            self.restore_late_toolbars()

    def _add_plugin_tool_entry(self, plug, tool, taken, reserved) -> None:
        """One tool's entry in the Extensions menu. Apart so that a tool
        with an unusable name or shortcut costs only its own entry."""
        import logging
        log = logging.getLogger("ingetrazo.plugins")
        key = f"plugin_{plug.stem}_{type(tool).__name__}"
        action = QAction(tr(tool.name), self)
        if tool.description:
            action.setStatusTip(tr(tool.description))
        if tool.shortcut:
            seq = QKeySequence(tool.shortcut).toString()
            if seq and (seq not in taken or reserved.get(seq) == key):
                action.setShortcut(QKeySequence(tool.shortcut))
                from views.shortcuts import set_tooltip
                set_tooltip(action, tr(tool.name))
                taken.add(seq)
            else:
                log.warning("plugin %r wants shortcut %r, already "
                            "taken; entry added without it",
                            plug.stem, tool.shortcut)
        self._tools[key] = tool
        action.triggered.connect(
            lambda _c, k=key: self._activate_plugin_tool(k))
        self._ext_menu.insertAction(self._ext_anchor, action)
        self.__dict__.setdefault("_ext_action_owner", {})[action] = plug.stem

    def restore_late_toolbars(self) -> bool:
        """Restore saved areas for extension toolbars added after startup."""
        st = QSettings()
        state = st.value("ui/window_state")
        if not state:
            return False
        restored = bool(self.restoreState(state))
        self._show_default_trays(st)
        self._place_new_toolbars(st)
        return restored

    def _clamp_to_screen(self) -> None:
        """Fit restored geometry to the current display, including its frame.
        Called once, right after restoreGeometry (before the first show
        the frame is not known yet: the title bar's height is allowed
        for)."""
        if self.isFullScreen() or self.isMaximized() or self.screen() is None:
            return
        avail = self.screen().availableGeometry()
        frame = self.frameGeometry()
        extra = frame.size() - self.size()
        if not self.isVisible() and extra.height() <= 0:
            extra.setHeight(28)          # the title bar the show will add
        self.resize(min(self.width(), avail.width() - max(0, extra.width())),
                    min(self.height(), avail.height() - max(0, extra.height())))
        frame = self.frameGeometry()
        self.move(max(avail.left(), min(frame.left(), avail.right() - frame.width() + 1)),
                  max(avail.top(), min(frame.top(), avail.bottom() - frame.height() + 1)))

    def _on_manage_extensions(self) -> None:
        from ingetrazo_render import show_report_sheet
        show_report_sheet(self, "extensions")

    @staticmethod
    def example_extensions() -> list:
        """``(path, title, blurb)`` of each extension shipped with the app
        (``examples/extensions``); the extension manager installs them."""
        from core.extension_manager import example_extension_files
        return example_extension_files()

    # ---- Extension panels (views.extension_api.ExtensionApp.add_panel) -------
    def _register_extension_dock(self, dock) -> None:
        """Give an extension's panel what the built-in trays get for free.

        ``setup(app)`` may add a panel after ``restoreState`` laid the window
        out, and an extension may add one later still (the first time its
        tool runs), so here a panel gets:

        - **its saved place.** ``restoreDockWidget`` puts it back where the
          user left it last session (area, size, floating, tabbed) when the
          window state knows its ``objectName`` — which is why that name
          must be stable across versions;
        - **a first place** when there is none: tabbed with the trays;
        - **a way back** when closed: Window ▸ Panels lists every tab."""
        if self.isVisible() and not dock.isFloating():
            # Shown before the dock layout places it, a new dock sat at the
            # window's (0, 0) until the next layout pass parked it behind
            # its tab. Under the Metal view every dock is a native view, so
            # that spot was drawn: the Solid Inspector flashed over the
            # toolbar for ~300 ms before Home came up. Start off-screen;
            # the layout (or restoreDockWidget for a floating one) moves it.
            dock.move(-32000, -32000)
        if not self.restoreDockWidget(dock):
            # Tabbed with Properties, on whichever side it is: the last
            # tray of the list may have been moved to the other side, and
            # tabbing with it put the extensions in a group of their own.
            area = self.dockWidgetArea(self.tray)
            if area == Qt.NoDockWidgetArea:
                area = Qt.RightDockWidgetArea
            self.addDockWidget(area, dock)
            self.tabifyDockWidget(self.tray, dock)
        self._extension_docks.append(dock)
        self._keep_docked(dock)
        # A panel added after start-up (the first time its tool runs) gets
        # the same rule as the others: closed if the user closed it, or if
        # it asks to start hidden and has not been seen yet.
        st = QSettings()
        hidden = self._hidden_tray_names(st)
        if dock.objectName() in hidden:
            dock.hide()
        elif self.isVisible():
            # Made after the window showed, a widget starts hidden until
            # shown: the tab the extension asked for would not appear.
            dock.show()
        if st.contains("ui/hidden_trays"):
            st.setValue("ui/hidden_trays", sorted(hidden))
        self._mark_trays_known(st, [dock.objectName()])

    def extension_panels(self) -> dict:
        """``objectName`` → dock for every panel an extension added."""
        return {d.objectName(): d for d in self._extension_docks}

    def _activate_plugin_tool(self, key: str) -> None:
        """Run a plugin tool from the Extensions menu.

        Plugin tools are one-shot (they open a dialog and return): the
        viewport's active tool is left untouched, so the status bar keeps
        telling the truth about which drawing tool is current."""
        tool = self._tools[key]
        self._remember_command(tool.name, lambda: tool.on_activate(self.viewport))
        tool.on_activate(self.viewport)

    def _on_cut_list(self) -> None:
        from ingetrazo_render import show_report_sheet as show
        show(self, "cutlist")

    def _on_about(self) -> None:
        from ingetrazo_render import show_report_sheet as show
        show(self, "about")

    def _file_actions(self) -> list[QAction]:
        actions = []
        home_action = QAction('Home', self)
        home_action.triggered.connect(self.show_home)
        actions.append(home_action)

        new_action = QAction(tr("New"), self)
        new_action.setStatusTip(tr(
            "Start a new, empty project in a tab of its own."))
        new_action.setShortcut(QKeySequence.New)
        new_action.triggered.connect(self._on_new)
        actions.append(new_action)

        open_action = QAction(tr("Open…"), self)
        open_action.setStatusTip(tr(
            "Open a document in a tab of its own."))
        open_action.setShortcut(QKeySequence.Open)
        open_action.triggered.connect(self._on_open)
        actions.append(open_action)

        # The last documents, one click away (asked for since the 0.4.x
        # triage; the usual File ▸ Open Recent). Filled when shown, so a
        # file deleted meanwhile just drops off the list.
        self._recent_menu = QMenu(tr("Open Recent"), self)
        self._recent_menu.aboutToShow.connect(self._fill_recent_menu)
        actions.append(self._recent_menu.menuAction())

        recover_action = QAction(tr("Recover a discarded auto-save…"), self)
        recover_action.setStatusTip(tr(
            "Auto-saved copies retired when a session was closed without "
            "saving — the last ones are kept here for a second chance."))
        recover_action.triggered.connect(self._on_recover_discarded)
        actions.append(recover_action)

        save_action = QAction(tr("Save"), self)
        save_action.setStatusTip(tr(_SAVE_TIP))
        save_action.setShortcut(QKeySequence.Save)
        save_action.triggered.connect(self._on_save)
        actions.append(save_action)

        save_as_action = QAction(tr("Save As…"), self)
        save_as_action.setStatusTip(tr(
            "Save the document under another name or in another folder."))
        save_as_action.setShortcut(QKeySequence.SaveAs)
        save_as_action.triggered.connect(self._on_save_as)
        actions.append(save_as_action)

        copy_action = QAction(tr("Save a Copy as IngeTrazo (.igz)…"), self)
        copy_action.triggered.connect(self._on_export_igz)
        actions.append(copy_action)

        actions.append(self._separator())

        # Linking is not an import — the file stays a document of its own —
        # so it has its own place in File, next to its management menu
        # (built further down, after Import). Maxence, 2026-11-07: «should
        # there be a File ▸ Link option?».
        link_action = QAction(tr("Link Document…"), self)
        link_action.setStatusTip(tr(
            "Place another IngeTrazo document as a live link: it stays its "
            "own file, and every change saved to it shows here."))
        link_action.setShortcut(QKeySequence("Ctrl+Shift+L"))
        link_action.triggered.connect(self._on_link_igz)
        actions.append(link_action)
        self._act_link_document = link_action
        # Linked documents (core/links.py): built when it opens, one entry
        # per source — open, reload, pack, tags as the source has them.
        links_menu = QMenu(tr("Linked Documents"), self)
        links_menu.aboutToShow.connect(
            lambda m=links_menu: self._fill_links_menu(m))
        actions.append(links_menu)
        actions.append(self._separator())

        # One home for everything that comes in, one for everything that
        # goes out — the flat list had import/export items scattered.
        import_menu = QMenu(tr("Import"), self)
        for label, tip, handler in (
            (tr("Document as component…"),
             tr("Bring another IngeTrazo document in as one component, "
                "placed with a click."),
             self._on_import_igz),
            (tr("Revit family (.rfa)…"),
             tr("Bring in Revit family types, materials and parameters."),
             self._on_import_rfa),
            (tr("SKP (.skp)…"),
             tr("Bring in a .skp model with its groups, materials and "
                "textures."),
             self._on_import_skp),
            (tr("COLLADA (.dae)…"),
             tr("Bring in a COLLADA model with its materials and "
                "textures."),
             self._on_import_dae),
            (tr("glTF/GLB (.glb)…"),
             tr("Bring in a glTF binary model with its materials and "
                "textures."),
             self._on_import_glb),
            (tr("Wavefront OBJ (.obj)…"),
             tr("Bring in a Wavefront OBJ model with its materials."),
             self._on_import_obj),
            (tr("STL mesh (*.stl)…"),
             tr("Bring in an STL mesh — a 3D print or a scan — at the "
                "scale you choose."),
             self._on_import_stl),
            (tr("Image (PNG / JPG)…"),
             tr("Place a picture in the model to trace over."),
             self._on_import_image),
            (tr("Orthomosaic (GeoTIFF)…"),
             tr("Lay a GeoTIFF orthomosaic under the model at its true "
                "place and size, to trace over."),
             self._on_import_orthophoto),
            (tr("AutoCAD DWG (.dwg)…"),
             tr("Bring in the linework of an AutoCAD drawing, one group per "
                "layer."),
             self._on_import_dwg),
            (tr("AutoCAD DXF (.dxf)…"),
             tr("Bring in the linework of an AutoCAD drawing, one group per "
                "layer."),
             self._on_import_dxf),
            (tr("Georeference (KML / GeoJSON)…"),
             tr("Bring in a KML, KMZ or GeoJSON alignment as paths in their "
                "true place, ready to profile or measure."),
             self._on_import_georef),
            (tr("Survey points CSV (UTM)…"),
             tr("Bring in surveyed points from a CSV file with UTM "
                "coordinates."),
             self._on_import_survey_points),
            (tr("Photogrammetric mesh (WebODM)…"),
             tr("Bring in a WebODM drone survey in its true place, as a "
                "mesh to trace over."),
             self._on_import_photomesh),
        ):
            act = QAction(label, self)
            act.setStatusTip(tip)
            act.triggered.connect(handler)
            import_menu.addAction(act)
        import_menu.addSeparator()
        clear_tex = QAction(tr("Clear imported texture cache…"), self)
        clear_tex.setStatusTip(tr(
            "Delete the images extracted from imported .skp files."))
        clear_tex.triggered.connect(self._on_clear_texture_cache)
        import_menu.addAction(clear_tex)
        actions.append(import_menu)


        from ingetrazo_render import delivery_export_form as export_form
        delivery_export = QAction(tr("Export…"), self)
        delivery_export.triggered.connect(lambda: export_form(self, one_off=True))
        actions.append(delivery_export)
        export_menu = QMenu(tr("Export"), self)
        for label, tip, handler in (
            (tr("IFC (BIM)…"),
             tr("Save the model as IFC for BIM software, with what the BIM "
                "panel tagged."),
             self._on_export_ifc),
            (tr("glTF / GLB (3D, single file)…"),
             tr("Save the model as one .glb file with its materials and "
                "textures, for Blender and web viewers."),
             self._on_export_glb),
            (tr("COLLADA (.dae)…"),
             tr("Save the model as COLLADA, which most 3D programs open; "
                "the texture images go beside the file."),
             self._on_export_dae),
            (tr("STL (3D printing)…"),
             tr("Save the model as an STL mesh for 3D printing."),
             self._on_export_stl),
            (tr("Wavefront OBJ (.obj)…"),
             tr("Save the model as Wavefront OBJ with its materials."),
             self._on_export_obj),
            (tr("Current view as DXF…"),
             tr("Save the view on screen as a 2D line drawing for CAD, with "
                "the hidden lines removed."),
             self._on_export_view_dxf),
            (tr("Image (PNG / JPG)…"),
             tr("Save the view on screen as a PNG or JPG picture at the "
                "width you choose."),
             self._on_export_image),
            (tr("Scenes as Video…"),
             tr("Save the scenes as a video: the camera glides from one to "
                "the next, with the times of Camera ▸ Animation."),
             self.export_video),
        ):
            act = QAction(label, self)
            act.setStatusTip(tip)
            act.triggered.connect(handler)
            export_menu.addAction(act)
        actions.append(export_menu)

        # Components moved to the Properties tray (a panel with
        # static thumbnails) — see views/tray.py::ComponentsPanel.

        actions.append(self._separator())

        composer_action = QAction(tr("Sheet composer…"), self)
        composer_action.setStatusTip(tr(
            "Lay out the model on paper at exact scale and export a PDF plan."))
        composer_action.triggered.connect(self._on_open_composer)
        actions.append(composer_action)

        actions.append(self._separator())

        # Projects in tabs (#1066): close the one in front, walk the tabs.
        close_doc = QAction(tr("Close Project"), self)
        close_doc.setStatusTip(tr(
            "Close the project in front, offering to save it; the others "
            "stay open."))
        close_doc.setShortcut(QKeySequence("Ctrl+W"))
        close_doc.triggered.connect(lambda _c=False: self.close_document())
        actions.append(close_doc)
        self._act_close_project = close_doc
        for label, tip, keys, step in (
                (tr("Next Project"), tr("Bring the next project tab to the "
                                        "front."), "Ctrl+PgDown", 1),
                (tr("Previous Project"), tr("Bring the previous project tab "
                                            "to the front."), "Ctrl+PgUp", -1)):
            act = QAction(label, self)
            act.setStatusTip(tip)
            act.setShortcut(QKeySequence(keys))
            act.triggered.connect(lambda _c=False, k=step: self._step_document(k))
            actions.append(act)

        actions.append(self._separator())

        quit_action = QAction(tr("Quit"), self)
        quit_action.setStatusTip(tr(
            "Close Built, offering to save what has not been saved."))
        quit_action.setShortcut(QKeySequence.Quit)
        quit_action.triggered.connect(self.close)
        actions.append(quit_action)

        return actions

    def _separator(self) -> QAction:
        sep = QAction(self)
        sep.setSeparator(True)
        return sep

    def _build_statusbar(self) -> None:
        # The status bar carries the Model | Sheet 1 | … strip at its left,
        # on the same row as the measurements box (AutoCAD's Model / Layout
        # tabs): one click from the model to any sheet.
        bar = self.statusBar()
        self._sheet_tabs = None
        # Ctrl+Tab: to the sheets and back (#91, @pacaeiro) — the last sheet
        # shown, as the strip's own tab; Ctrl+Shift+Tab too, with only two
        # places to go. A document without sheets gets its first one.
        # With several projects open (#1066) the same keys walk the project
        # tabs instead, as in a browser; Ctrl+PgDown / Ctrl+PgUp always do.
        from PySide6.QtGui import QShortcut
        for seq, step in (("Ctrl+Tab", 1), ("Ctrl+Shift+Tab", -1)):
            QShortcut(QKeySequence(seq), self,
                      activated=lambda k=step: self._ctrl_tab(k))
        self._tool_label = QLabel(tr("Tool: none"))
        bar.addPermanentWidget(self._tool_label)
        # Not shown any more: the tool's name leads the hint instead, and the
        # 250 px it held go to the hint (Marco, 23-09 — the icon of the
        # tool is highlighted anyway). Kept as the record other code reads.
        self._tool_label.hide()
        self._refresh_sheet_tabs()

        # Live UTM readout, the way a CAD shows coordinates. Local scene metres
        # are meaningless to anyone outside the file; easting/northing is what
        # goes on a plan, into a GPS, and into a report. Only shown once the
        # scene has a datum — there is no coordinate without one.
        self._coord_label = QLabel("")
        theme_style(self._coord_label, "color:{muted}; padding:0 8px;")
        bar.addPermanentWidget(self._coord_label)

        # Measurements box (VCB), pinned bottom-right: a caption
        # ("Length" / "Dimensions" / "Distance") plus a boxed field showing the
        # live measurement, or what you're typing (highlighted while typing).
        self._vcb_buffer = ""
        self._vcb_live = ""
        self._vcb_name = QLabel("")
        theme_style(self._vcb_name, "color:{muted}; padding:0 4px;")
        self._vcb_value = QLabel("")
        self._vcb_value.setMinimumWidth(130)
        self._vcb_value.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
        self._vcb_value.setStyleSheet(self._VCB_IDLE_STYLE)
        bar.addPermanentWidget(self._vcb_name)
        bar.addPermanentWidget(self._vcb_value)
        # Numbers and coordinates read left to right in a mirrored (right-to-
        # left, #1067) interface too: what is typed here goes to the tools.
        from views.rtl import keep_ltr
        keep_ltr(self._vcb_value, self._coord_label)

        # (valueBufferChanged → _on_value_buffer: see _wire_viewport.)
        # The coordinate and measurement texts change on EVERY mouse move;
        # each setText dirties the status bar and Qt flushes the top-level
        # window's backing store (3072×1920 px at scale 2 on Marco's
        # laptop) — measured 2026-09-14 on the plaza: frames p90 190 ms with
        # the labels live, 57 ms with them frozen. They now settle at most
        # ~12 times a second, and only when the text actually changed.
        self._status_pending: dict = {}
        self._status_timer = QTimer(self)
        self._status_timer.setSingleShot(True)
        self._status_timer.setInterval(self.STATUS_TEXT_MS)
        self._status_timer.timeout.connect(self._flush_status_texts)
        # (measurementChanged / coordinateChanged: see _wire_viewport.)

    _VCB_IDLE_STYLE = (
        "color:#0F141B; background:#FFFFFF; border:1px solid #9aa3ad;"
        "border-radius:3px; padding:2px 8px;"
    )
    _VCB_ACTIVE_STYLE = (
        "color:#0F141B; background:#FFF3C4; border:1px solid #E0A800;"
        "border-radius:3px; padding:2px 8px;"
    )

    def _on_value_buffer(self, text: str) -> None:
        self._vcb_buffer = text
        self._refresh_vcb()

    #: How often the per-hover status texts may repaint (ms).
    STATUS_TEXT_MS = 80

    def _queue_status_text(self, which: str, text: str) -> None:
        self._status_pending[which] = text
        if not self._status_timer.isActive():
            self._status_timer.start()

    def _flush_status_texts(self) -> None:
        pending, self._status_pending = self._status_pending, {}
        coord = pending.get("coordinate")
        if coord is not None and coord != self._coord_label.text():
            self._coord_label.setText(coord)
            # The compact readout above; the full one, zone included, on hover.
            self._coord_label.setToolTip(
                getattr(self.viewport, "_last_coordinate_full", "") or "")
        meas = pending.get("measurement")
        if meas is not None and meas != getattr(self, "_vcb_live", None):
            self._on_measurement(meas)

    def _on_measurement(self, text: str) -> None:
        self._vcb_live = text
        self._refresh_vcb()

    def _refresh_vcb(self) -> None:
        tool = self.viewport.active_tool
        # A tool may vary its caption with state (Circle: "Lados" before the
        # centre, "Radio" after); fall back to the static label.
        dynamic = getattr(tool, "vcb_caption", None) if tool is not None else None
        caption = (dynamic() if callable(dynamic)
                   else getattr(tool, "vcb_label", None)) if tool is not None \
            else None
        live = self._vcb_live
        if tool is None and getattr(self.viewport, "nav_mode", None) == "zoom":
            # Zoom takes the lens typed in the box (35mm, 60deg): the box
            # shows, reading the focal length the view has now.
            caption = "Field of view"
            cam = self.viewport.camera
            live = (f"{cam.focal_mm:.0f} mm" if cam.perspective else "—")
        self._vcb_name.setVisible(caption is not None)
        self._vcb_value.setVisible(caption is not None)
        if caption is None:
            return
        self._vcb_name.setText(tr(caption))
        if self._vcb_buffer:
            self._vcb_value.setText(f"{self._vcb_buffer}")
            self._vcb_value.setStyleSheet(self._VCB_ACTIVE_STYLE)
        else:
            self._vcb_value.setText(live)
            self._vcb_value.setStyleSheet(self._VCB_IDLE_STYLE)

    # ---- Tool routing -------------------------------------------------------
    def _activate_tool(self, key: str) -> None:
        if self._tool_filter is not None and key not in self._tool_filter:
            self.statusBar().showMessage(
                tr("{name} is not available here.", name=tr(self._tools[key].name)), 3000)
            return
        tool = self._tools[key]
        self.viewport.set_active_tool(tool)
        display = getattr(self.viewport, "keys_stand_in", None)
        if display is not None:
            display.tools.pick_up()
            display.set_scene(self.viewport.scene)
            display.update()
        action = self._tool_actions.get(key)
        if action is not None:
            action.setChecked(True)
        self._tool_label.setText(tr("Tool: {name}", name=tr(tool.name)))
        self._refresh_vcb()
        # Select is where Repeat is pressed FROM, and the two tools entered
        # from a right-click need the face or group that click was on.
        if key not in ("select", "texture_position", "change_axes",
                       "scale_manager"):
            self._remember_command(tool.name,
                                   lambda k=key: self._activate_tool(k))

    # ---- Repeat last command (Blender Shift+R, right-click ▸ Repeat) -------
    def _remember_command(self, label: str, run) -> None:
        self._last_command = (label, run)

    def _activate_nav(self, key: str) -> None:
        self.viewport.set_nav_mode(key)
        # The keys typed next are the view's (Zoom takes «35mm»), wherever
        # the keyboard was — a panel's list, a field.
        self.viewport.setFocus(Qt.OtherFocusReason)
        action = self._nav_actions.get(key)
        if action is not None:
            action.setChecked(True)
        self._tool_label.setText(
            tr("Nav: {name}", name=tr(key.capitalize())))
        self._refresh_vcb()

    def _on_scale_dialog(self, group) -> None:
        tool = self._tools["scale_manager"]
        tool.target = group
        self._activate_tool("scale_manager")

    def _on_components_from_file(self) -> None:
        start = (str(self._current_path.parent)
                 if getattr(self, "_current_path", None) else "")
        path, _ = file_dialogs.getOpenFileName(
            self, tr("Components from File"), start,
            built.DOCUMENT_FILTER)
        if not path:
            return
        try:
            n = self.import_kept_components(path)
        except Exception as exc:  # noqa: BLE001 - any unreadable file
            _dialogs.warning(self, tr("Components from File"),
                                tr("Could not read {p}: {e}", p=path, e=exc))
            return
        if n:
            text = tr("{n} components added to the list, ready to place",
                      n=n)
        else:
            text = tr("That file holds no components")
        self.statusBar().showMessage(text, 5000)

    def import_kept_components(self, path) -> int:
        """Bring every component definition of another document into this
        one as KEPT components — in the list, none placed: a library to
        draw from. Its materials and layers come along by name, as Insert
        does. Returns how many; nothing changes for a file without one."""
        from core.history import AddKeptComponentsCommand
        from core.insert import ensure_layers, merge_materials
        from core.kept_components import definitions_of
        from core.scene import Scene as _Scene
        from formats import igz as _igz
        temp = _Scene()
        _igz.load_into(temp, Path(path))       # raises before anything moves
        templates = definitions_of(temp)
        if not templates:
            return 0
        scene = self.viewport.scene
        for t in templates:
            merge_materials(getattr(temp, "materials", {}) or {},
                            scene.materials, t)
            ensure_layers(scene, t)
        self.viewport.history.execute(AddKeptComponentsCommand(templates))
        self.viewport.notify_scene_changed()
        return len(templates)

    def _fill_intersect_menu(self, menu) -> None:
        from core.intersect import WITH_CONTEXT, WITH_MODEL, WITH_SELECTION
        for mode, label, tip in (
                (WITH_MODEL, tr("With Model"),
                 tr("Add edges wherever the selected faces cross the rest "
                    "of the model, groups included.")),
                (WITH_SELECTION, tr("With Selection"),
                 tr("Add edges where the selected faces cross one "
                    "another.")),
                (WITH_CONTEXT, tr("With Context"),
                 tr("Add edges where the selected faces cross the rest of "
                    "the group being edited, or of the model."))):
            act = menu.addAction(
                label, lambda m=mode: self._on_intersect_faces(m))
            act.setStatusTip(tip)

    # ---- Sections (section planes) -----------------------------------------
    def _set_scene_visibility(self, attr: str, on: bool) -> None:
        setattr(self.viewport.scene, attr, bool(on))
        self.viewport.update()
        browser = getattr(self, "project_browser", None)
        if browser is not None:
            browser.outliner.on_scene_changed()

    def _sync_section_menu(self) -> None:
        scene = self.viewport.scene
        for act, value in (
                (self._act_show_splanes,
                 getattr(scene, "show_section_planes", True)),
                (self._act_show_scuts,
                 getattr(scene, "show_section_cuts", True)),
                (getattr(self, "_act_hidden_objects", None),
                 getattr(scene, "show_hidden_objects", False)),
                (getattr(self, "_act_hidden_geometry", None),
                 getattr(scene, "show_hidden_geometry", False))):
            if act is None:
                continue
            act.blockSignals(True)
            act.setChecked(value)
            act.blockSignals(False)

    def _sync_component_edit_menu(self) -> None:
        """The Component Edit checkmarks and the rest-mode radios follow
        the viewport: Preferences changes the same mode."""
        mode = self.viewport.edit_rest_mode
        if mode != "hide":
            self._rest_shown_mode = mode
        self._act_hide_rest.setChecked(mode == "hide")
        act = self._rest_actions.get(mode)
        if act is not None:
            act.setChecked(True)
        fade = round(self.viewport.edit_rest_fade * 100)
        self._rest_fade_slider.blockSignals(True)
        self._rest_fade_slider.setValue(fade)
        self._rest_fade_slider.setEnabled(mode == "fade")
        self._rest_fade_slider.blockSignals(False)
        self._rest_fade_value.setText(f"{fade} %")
        self._act_hide_similar.setChecked(bool(getattr(
            self.viewport.scene, "hide_similar_components", False)))

    def _on_edit_rest_fade_changed(self, value: int) -> None:
        """Apply the Camera-menu fade slider immediately and persist it."""
        self._rest_fade_value.setText(f"{value} %")
        self.viewport.set_edit_rest_fade(value / 100.0)

    def _set_hidden_view(self, attr: str, on: bool) -> None:
        """View ▸ Hidden Objects / Geometry: a scene flag (it travels in the
        document and in scenes), a version bump so the pick index learns
        what became selectable, and a repaint."""
        scene = self.viewport.scene
        if bool(getattr(scene, attr, False)) == bool(on):
            return
        setattr(scene, attr, bool(on))
        if not on:
            # What the view no longer shows cannot stay selected: the ghost
            # group's orange outline outlived the switch (Marco,
            # 2026-09-21, issue #53).
            for ent in list(scene.selection):
                if scene.entity_hidden(ent) or getattr(ent, "hidden", False):
                    scene.selection.discard(ent)
        scene.bump_view()
        self.viewport.update()

    def _selected_section_planes(self) -> list:
        from core.section import SectionPlane
        return [p for p in self.viewport.scene.selection
                if isinstance(p, SectionPlane)]

    # ---- Display styles ----------------------------------------------------
    def _active_view(self):
        return self.viewport

    def active_style(self):
        return self.viewport.scene.display_style

    own_style = active_style

    def set_active_style(self, style):
        self.viewport.scene.display_style = style
        self.repaint_styled()

    def repaint_styled(self):
        self.viewport.update()

    def _apply_display_style(self, preset) -> None:
        """Activate a built-in style (a COPY — presets stay pristine) in the
        active view."""
        self.set_active_style(preset.copy())
        self._sync_style_menu()
        self.statusBar().showMessage(
            tr("Style: {name}", name=tr(preset.name)), 2000)

    def _toggle_xray(self) -> None:
        """Alt+X: into X-ray, remembering the style you leave; out of it,
        back to that style — or to Default when there is none to go back to
        (X-ray picked from the menu, or another document since)."""
        from core.style import style_by_name
        vp = self._active_view()
        style = self.active_style()
        # Each view of several goes in and out on its own: the window's view
        # remembers in ``_style_before_xray``, the others on themselves.
        mine = vp is self.viewport
        if style is not None and style.face_mode == "xray":
            if mine:
                prev_scene, prev = self._style_before_xray
                self._style_before_xray = (None, None)
            else:
                prev_scene, prev = getattr(vp, "_style_before_xray",
                                           (None, None))
                vp._style_before_xray = (None, None)
            if prev_scene is not vp.scene or prev is None:
                prev = style_by_name("Default")
            self._apply_display_style(prev)
        else:
            kept = (vp.scene, style.copy() if style is not None else None)
            if mine:
                self._style_before_xray = kept
            else:
                vp._style_before_xray = kept
            self._apply_display_style(style_by_name("X-ray"))

    def _set_style_field(self, name: str, value: bool) -> None:
        style = self.active_style()
        if style is None or getattr(style, name) == bool(value):
            return
        setattr(self.own_style(), name, bool(value))
        self.repaint_styled()
        browser = getattr(self, "project_browser", None)
        if browser is not None and name == "section_fill":
            browser.outliner.on_scene_changed()

    def _sync_style_menu(self) -> None:
        """Reflect the active view's style in the menu (loads, scene recall,
        another of several views clicked)."""
        style = self.active_style()
        if style is None:
            return
        for name, act in self._style_actions.items():
            act.setChecked(name == style.name)
        for act, value in ((self._act_style_edges, style.edges),
                           (self._act_style_profiles, style.profiles),
                           (getattr(self, "_act_style_back_edges", None),
                            getattr(style, "back_edges", False)),
                           (getattr(self, "_act_section_fill", None),
                            getattr(style, "section_fill", True))):
            if act is None:      # menu still under construction
                continue
            act.blockSignals(True)
            act.setChecked(value)
            act.blockSignals(False)
        # The Styles/Shadows docks mirror the same state (guarded: the menu
        # builds before the docks on startup). Loads and scene recalls land
        # here too.
        sh = getattr(self.viewport.scene, "shadows", None)
        act = getattr(self, "_act_shadows", None)
        if act is not None and sh is not None:
            act.blockSignals(True)
            act.setChecked(sh.enabled)
            act.blockSignals(False)
        panel = getattr(self, "styles_panel", None)
        if panel is not None:
            panel.refresh()
        panel = getattr(self, "shadows_panel", None)
        if panel is not None:
            panel.refresh()

    def release_eyedropper(self) -> None:
        """Called back by the tool once it has sampled: the button pops out
        on its own, so the state you see is the state you are in."""
        PaintTool.sample_armed = False      # the sample is done either way
        act = getattr(self, "_act_eyedropper", None)
        if act is not None and act.isChecked():
            act.blockSignals(True)
            act.setChecked(False)
            act.blockSignals(False)
        self.viewport._apply_tool_cursor()

    def _on_toggle_shadows(self, on: bool) -> None:
        """Camera ▸ Shadows: flip the scene's sun on/off (the tray panel
        holds the date/time/darkness detail)."""
        sh = getattr(self.viewport.scene, "shadows", None)
        if sh is None or sh.enabled == bool(on):
            return
        sh.enabled = bool(on)
        self.viewport.scene.version += 1
        panel = getattr(self, "shadows_panel", None)
        if panel is not None:
            panel.refresh()
        self.viewport.update()

    def _sync_foreign_clipboard(self) -> None:
        # A copy made in ANOTHER IngeTrazo window wins over this window's
        # older one, as a system clipboard does (issue #76).
        from formats import clip as clip_transfer
        try:
            other = clip_transfer.foreign()
        except Exception:  # noqa: BLE001 - a bad clipboard must not break Paste
            other = None
        source = None
        if other is None:
            # A copy made in another project tab of this window (#1066).
            vp = self.viewport
            try:
                from PySide6.QtWidgets import QApplication
                md = QApplication.clipboard().mimeData()
                source = getattr(md, "clip", None)
                # Only a copy that one of the OTHER tabs still holds: a
                # stale offer (a closed window's, a clipboard set by code)
                # must not replace this project's own.
                if source is not None and any(
                        d.viewport.clipboard is source
                        for d in self._documents if d is not self._doc):
                    other = clip_transfer.own(
                        vp.clipboard,
                        getattr(vp, "_clip_adopted_from", None))
            except Exception:  # noqa: BLE001 - a bad clipboard must not break Paste
                other = None
        if other is not None:
            old = self.viewport.clipboard
            drop = getattr(self.viewport, "_drop_clip_protos", None)
            if old and callable(drop):
                drop(old)
            self.viewport.clipboard = other
            self.viewport._clip_adopted_from = source

    def _cancel_tool(self) -> None:
        """Esc, escalating like the viewport: release a sticky constraint
        (axis lock / reference) first, then cancel an in-progress action;
        with nothing in progress, clear the selection."""
        pending = self.__dict__.get("_pending_open")
        if pending:
            pending.job.cancel()
            return
        # One cascade, the viewport's — this action's shortcut fires before
        # the viewport ever sees the key, and its own copy lacked the
        # «step out of the group» stop.
        self.viewport.escape()

    # ---- View navigation ----------------------------------------------------
    def _on_zoom_extents(self) -> None:
        bounds = self._extents_bounds()
        if bounds is None:
            return
        from views.wgpu_ui import free_area   # the panels' free area (B8)
        self.viewport.camera.fit_box(bounds[0], bounds[1], free=free_area(self))
        self.viewport.update()

    def _on_smart_zoom(self) -> None:
        from ingetrazo_render import smart_zoom
        smart_zoom(self)

    def _on_zoom_selection(self) -> None:
        """Zoom Extents over the selection. Nothing selected: nothing."""
        lo, hi = self.viewport.scene.selection_bounds()
        if lo is None:
            self.statusBar().showMessage(tr("Nothing selected"), 2500)
            return
        from views.wgpu_ui import free_area   # the panels' free area (B8)
        self.viewport.camera.fit_box(lo, hi, free=free_area(self))
        self.viewport.update()

    def _sheet_tab_menu(self, index: int, global_pos) -> None:
        self._ensure_composer().sheet_tab_menu(index, global_pos, self)
        self._refresh_sheet_tabs()

    # ---- Model / sheet tabs (the strip at the bottom) -----------------------
    def _refresh_sheet_tabs(self) -> None:
        """This window shows the model, so its strip always marks «Model».
        Beside it, ONE sheet: the last one opened — the model window only
        goes model ↔ sheet, and all the sheets are tabs in the composer
        (Marco, 23-09: «en el modelo solo alternar entre el modelo y la
        lámina… cuando regrese a lámina, que regrese a la lámina 2»). A
        document without sheets offers «+» instead."""
        composer = getattr(self, "_composer", None)
        if composer is not None and hasattr(composer, "views_panel"):
            composer.views_panel.on_scene_changed()
        tabs = getattr(self, "_sheet_tabs", None)
        if tabs is None:
            return
        comps = getattr(self.viewport.scene, "compositions", None) or []
        if not comps:
            self._strip_sheets = []
            tabs.set_show_new(True)
            tabs.refresh([], None)
            return
        comp = getattr(getattr(self, "_composer", None), "comp", None)
        if comp is not None and comp in comps:
            last = comps.index(comp)
        else:
            last = min(max(getattr(self, "_last_sheet", 0), 0), len(comps) - 1)
        self._last_sheet = last
        self._strip_sheets = [last]
        tabs.set_show_new(False)
        tabs.refresh([comps[last].name], None)

    def _to_sheets(self) -> None:
        comps = getattr(self.viewport.scene, "compositions", None) or []
        if not comps:
            self._show_new_sheet()
            return
        last = min(max(getattr(self, "_last_sheet", 0), 0), len(comps) - 1)
        self._show_sheet(last)

    def _show_strip_sheet(self, k: int) -> None:
        """A sheet tab of THIS strip (it shows only the last sheet)."""
        sheets = getattr(self, "_strip_sheets", None) or []
        self._show_sheet(sheets[k] if 0 <= k < len(sheets) else k)

    def _strip_sheet_menu(self, k: int, pos) -> None:
        sheets = getattr(self, "_strip_sheets", None) or []
        self._sheet_tab_menu(sheets[k] if 0 <= k < len(sheets) else k, pos)

    def _show_model(self) -> None:
        from views.window_front import bring_to_front
        comp = getattr(self, "_composer", None)
        if comp is not None and getattr(comp, "EMBEDDED", False):
            comp.hide()                     # the Sheets workspace gives the frame back
        else:
            bring_to_front(self)
        self._refresh_sheet_tabs()

    def _show_new_sheet(self) -> None:
        """The «+» tab: the composer on a NEW sheet. A document without
        sheets gets its first one just by opening the composer."""
        comps = getattr(self.viewport.scene, "compositions", None) or []
        had = len(comps)
        self._on_open_composer()
        if len(self.viewport.scene.compositions) == had:
            self._composer._on_comp_add()
        self._composer.show_sheet(len(self.viewport.scene.compositions) - 1)
        self._refresh_sheet_tabs()

    def _on_standard_view(self, key: str) -> None:
        """A standard view (Views bar, Ctrl+1…7) in the view being worked
        in; one of several extra views takes it as its cell's view."""
        if key == "iso":
            self.viewport.camera.snap_iso()
        else:
            self.viewport.camera.set_view(key)
        self.viewport.update()

    # ---- Terrain-surface fill (Track G) -------------------------------------
    def _rebuild_surfaces(self):
        from ingetrazo_render import rebuild_path_surfaces
        rebuild_path_surfaces(self.viewport)

    def _on_surfaces_scene_changed(self) -> None:
        """Re-drape surfaced paths when their nodes move (version bump)."""
        if any(getattr(p, "surface", None) for p in self.viewport.scene.geo_paths):
            self._rebuild_surfaces()

    # ---- 3D terrain (Track G, G2 full) --------------------------------------
    @property
    def _terrain_on(self) -> bool:
        """Whether the 3D terrain is on — the document's say, kept with its
        base map (``TileLayer.terrain3d``) and saved with it, so a project
        reopens as it was left and each tab keeps its own. Without a base
        map yet, the window remembers the wish for the one to come."""
        layer = self._terrain_layer()
        if layer is not None:
            return bool(getattr(layer, "terrain3d", False))
        return bool(self.__dict__.get("_terrain_wish", False))

    def _terrain_layer(self):
        return getattr(self.viewport.scene, "tile_layer", None)

    def set_terrain_enabled(self, on):
        from ingetrazo_render import set_terrain_enabled
        self._terrain_wish = bool(on)
        set_terrain_enabled(self.viewport, on)

    def _build_terrain(self):
        from ingetrazo_render import restore_terrain
        restore_terrain(self.viewport)

    # ---- Undo / redo --------------------------------------------------------
    # ---- File handling ------------------------------------------------------
    # ---- The document's camera (issue #60) ---------------------------------
    def _show_camera_settings(self):
        from ingetrazo_render import open_camera
        return open_camera(self)

    # ---- Documents: one project per tab (#1066, views/documents.py) ---------
    # What the window kept for its single document now lives on the active
    # Document; these properties keep the old names reading and writing it,
    # so the code that predates tabs — and the extensions — act on the
    # project in front without knowing tabs exist.
    def _doc_attr(name):  # noqa: N805 — a helper run in the class body
        def get(self):
            doc = self.__dict__.get("_doc")
            return getattr(doc, name) if doc is not None else None

        def put(self, value):
            setattr(self._doc, name, value)
        return property(get, put)

    _current_path = _doc_attr("path")
    _import_name = _doc_attr("import_name")
    _saved_content = _doc_attr("saved_content")
    _autosaved_version = _doc_attr("autosaved_version")
    _composer = _doc_attr("composer")
    _last_sheet = _doc_attr("last_sheet")
    del _doc_attr

    #: The active project changed (another tab, or the one in front was
    #: closed); panels that keep something of the document refresh.
    documentChanged = Signal()

    def documents(self) -> list:
        """The open projects, in tab order."""
        bar = self.__dict__.get("_project_tabs")
        if bar is None:
            return list(self._documents)
        return bar.documents()

    def active_document(self):
        return self._doc

    def all_viewports(self) -> list:
        return [d.viewport for d in self._documents if d.viewport is not None]

    def _choose_document_renderer(self, doc):
        """Choose once per new document; an existing document never swaps."""
        doc._renderer_wgpu = doc.viewport.keys_stand_in is not None
        if doc._renderer_wgpu:
            self._install_wgpu_display(doc)
        if doc is self._doc:
            self._show_renderer_note(doc)

    def _show_renderer_note(self, doc):
        if not hasattr(self, "_renderer_label"):
            self._renderer_label = QLabel(self)
            self.statusBar().addPermanentWidget(self._renderer_label)
        from core import renderer_settings
        self._renderer_label.setText(renderer_settings.status_text())

    def _document_first_frame(self, doc):
        """Release optional work only after the renderer reports presentation.

        GL builds the data it needs on demand; the old eager preparation also
        baked unused smooth streams and serialized caches for the hidden GL
        viewport under Metal. Do not run that duplicate whole-scene pass.
        """
        if doc is None or doc.viewport is None:
            return
        if getattr(self, "_opening_document", False):
            return
        # Progress callbacks can pump a presentation of the previous scene.
        # Only a frame after this document's resident upload releases work.
        if (getattr(doc, "_renderer_wgpu", False)
                and not getattr(doc, "_resident_scene_ready", True)):
            return
        # A presented frame from either renderer satisfies the Rust engine's
        # start guard (main.py arms it; the GL viewport's frameSwapped never
        # fires under the Metal view).
        if not getattr(self, "_engine_frame_seen", False):
            from core import _native
            _native.start_ok()
            self._engine_frame_seen = True
        if not getattr(doc, "_awaiting_picture", False):
            return
        doc._awaiting_picture = False
        self._drain_document_startup(doc)

    def _drain_document_startup(self, doc):
        """One optional callback per event-loop slice, paused in background tabs."""
        from PySide6.QtCore import QTimer
        if getattr(doc, "_startup_draining", False):
            return
        doc._startup_draining = True

        def step():
            doc._startup_draining = False
            if doc not in self._documents or doc is not self._doc:
                return
            jobs = getattr(doc, "_after_picture", [])
            if jobs:
                doc._startup_draining = True
                try:
                    jobs.pop(0)()
                finally:
                    QTimer.singleShot(16, self, step)
        QTimer.singleShot(0, self, step)

    def _install_wgpu_display(self, doc):
        """Install the native input/presentation view over shared document state.

        Scene changes coalesce before updating resident arrays.
        """
        from views.wgpu_view import WgpuView
        from PySide6.QtCore import QTimer
        display = doc.viewport.keys_stand_in or WgpuView(self._doc_stack)
        display.frameReady.connect(lambda *_: self._document_first_frame(doc))
        display.source_viewport = doc.viewport
        display.camera = doc.viewport.camera
        # Whoever gives the (hidden) GL viewport the keyboard gives it to
        # the view the user sees (Viewport.setFocus).
        doc.viewport.keys_stand_in = display
        doc.stand_in = display
        if self._doc_stack.indexOf(display) < 0:
            self._doc_stack.addWidget(display)
        refresh = QTimer(display)
        refresh.setSingleShot(True)
        resident_version = None

        def rebuild(progress=None, prepared=None):
            # ``progress``: the opening document's bar is still up and this
            # is its last stage (MainWindow._open_path_here).
            nonlocal resident_version
            if doc.viewport is None or getattr(display, "_closed", False):
                return
            if getattr(self, "_opening_document", False) and progress is None:
                return
            scene = doc.viewport.scene
            version = scene.version
            if prepared is None:
                display.set_scene(scene, progress=progress, opening=progress is not None)
            else:
                display.set_scene(scene, opening=True, prepared=prepared)
            resident_version = version
            doc._resident_scene_ready = True
            if display.packed is not None and not len(display.packed.instances):
                self._document_first_frame(doc)
        refresh.timeout.connect(lambda: rebuild())
        display._rebuild = rebuild
        doc.viewport.sceneVersionChanged.connect(lambda *_: refresh.start(0))
        display._refresh_timer = refresh
        def present():
            if (doc.viewport is None or getattr(display, "_closed", False)
                    or not display._presentable()):
                return
            # Outliner and editing commands can request a paint on the hidden
            # GL viewport. Its paint would normally announce the new version.
            scene = doc.viewport.scene
            if (not refresh.isActive() and not getattr(self, "_opening_document", False)
                    and (scene is not getattr(display, "scene", None)
                         or scene.version != resident_version)):
                doc.viewport.notify_scene_changed()
            elif display._can_create_surface():
                display.update()  # camera presets also use the shared viewport
                for peer in display.peers():
                    if peer.isVisible():
                        peer.update()

        repaint = QTimer(display)
        repaint.timeout.connect(present)
        repaint.start(33)
        display._repaint_timer = repaint
        doc.viewport.set_active_tool(None)
        doc.viewport.setEnabled(False)
        if doc is self._doc:
            self._doc_stack.setCurrentWidget(display)
        refresh.start(0)

    def _make_viewport(self):
        """A viewport for a project, set up the way the window's first one
        always was."""
        from views.document_controller import DocumentController
        from views.wgpu_view import WgpuView
        vp = DocumentController.for_widget(WgpuView(self._doc_stack))
        try:                                        # issue #56
            vp.history.max_steps = int(_gs.value("undo_steps", 200))
        except (TypeError, ValueError):
            vp.history.max_steps = 200
        from ingetrazo_render import attach_viewport_extensions
        attach_viewport_extensions(self, vp)
        return vp

    def _wire_viewport(self, vp) -> None:
        """Connect a project's viewport to the window's panels and status
        bar. Every viewport gets the same connections; each answers only
        while its project is the one in front."""
        def live(fn, *, after_picture=False):
            def dispatch(*args):
                if vp is not self.viewport:
                    return
                doc = self._doc
                if after_picture and getattr(doc, "_awaiting_picture", False):
                    doc._after_picture.append(lambda: fn(*args))
                else:
                    fn(*args)
            return dispatch
        vp.projectionChanged.connect(live(self._sync_projection_buttons))
        vp.smartZoomRequested.connect(live(self._on_smart_zoom))
        for name in ("tray", "bim_tray", "georef_tray", "project_browser"):
            panel = getattr(self, name, None)
            if panel is not None:
                vp.sceneVersionChanged.connect(
                    live(lambda _v, p=panel: p.on_scene_changed(),
                         after_picture=True))
        vp.sceneVersionChanged.connect(
            live(lambda _v: self._on_surfaces_scene_changed(),
                 after_picture=True))
        vp.valueBufferChanged.connect(live(self._on_value_buffer))
        vp.measurementChanged.connect(
            live(lambda text: self._queue_status_text("measurement", text)))
        vp.coordinateChanged.connect(
            live(lambda text: self._queue_status_text("coordinate", text)))
        vp.sceneVersionChanged.connect(live(self._on_scene_version_changed))
        # The extensions' «the document changed» calls (ExtensionApp).
        for fn in self.__dict__.get("_doc_change_hooks", ()):
            vp.sceneVersionChanged.connect(live(lambda _v, f=fn: f()))
        # The view's edges place the sidebar handles; a tray tab is dropped
        # on it.
        vp.setAcceptDrops(True)

    def _new_document(self, activate: bool = True):
        """A new, empty project in a tab of its own, right of the active
        one; shown at once unless ``activate`` is False."""
        from views.documents import Document
        doc = Document()
        doc.untitled_no = self._free_untitled_no()
        doc.viewport = self._make_viewport()
        self._wire_viewport(doc.viewport)
        self._doc_stack.addWidget(doc.viewport.keys_stand_in or doc.viewport)
        self._documents.append(doc)
        self._choose_document_renderer(doc)
        bar = self._project_tabs
        bar.add(doc, bar.index_of(self._doc) + 1)
        doc.saved_content = doc.viewport.scene.content_version
        # A new viewport binds the units to its own scene: the one in front
        # keeps them until the switch.
        from core import units as _units
        _units.bind_scene(self.viewport.scene)
        self._update_tab(doc)
        if activate:
            self.activate_document(doc)
        return doc

    def _step_document(self, step: int) -> None:
        nxt = self._project_tabs.neighbour(self._doc, step)
        if nxt is not None:
            self.activate_document(nxt)

    def _ctrl_tab(self, step: int) -> None:
        if len(self._documents) > 1:
            self._step_document(step)
        else:
            self._to_sheets()

    def activate_document(self, doc, closing: bool = False) -> None:
        """Bring ``doc``'s tab to the front. The tool in hand is dropped and
        an open group is left in the project left behind; the panels, the
        menus and the title follow the one in front."""
        if doc is None or doc not in self._documents:
            return
        bar = self._project_tabs
        if doc is self._doc:
            bar.set_current(doc)
            return
        if self._workspace is not None:
            # An extension's workspace holds the window: stay on it.
            bar.set_current(self._doc)
            return
        self._leave_document(self._doc)
        self._doc = doc
        self._doc_stack.setCurrentWidget(doc.slot_widget())
        bar.set_current(doc)
        self._enter_document(doc)

    def _leave_document(self, doc) -> None:
        from ingetrazo_render import leave_camera
        leave_camera(doc)
        vp = doc.viewport
        if vp is None:
            return
        try:
            if doc.front_view is not None:
                doc.front_view.set_active_tool(None)
            vp.set_active_tool(None)
            vp.end_group_edit()
        except RuntimeError:            # its widget is already gone
            return
        vp._last_pos = None
        comp = doc.composer
        doc.composer_shown = bool(comp is not None and comp.isVisible())
        if doc.composer_shown:
            comp.hide()

    #: Fallback width for the document archive thumbnail.
    ARCHIVE_PREVIEW_PX = 280

    def _archive_thumbnail(self, vp):
        """Render a small archive picture from the document viewport."""
        try:
            if vp.keys_stand_in.renderer is None:
                return None
            img = vp.render_image(self.ARCHIVE_PREVIEW_PX, overlays=False)
        except Exception:  # noqa: BLE001 — a missing picture only
            return None
        if img.isNull():
            return None
        from PySide6.QtCore import Qt as _Qt
        return img.scaledToWidth(self.ARCHIVE_PREVIEW_PX,
                                 _Qt.SmoothTransformation)

    #: Width of the picture a save keeps in the file (px): the start page's
    #: tiles and Continue card are ~200 pt wide, so twice that on Retina.

    def _document_picture(self, doc):
        """The picture a save keeps in the file (``thumbnail.png``): the
        model as its view shows it, without the chrome. None when there is
        nothing worth keeping (the file then keeps the picture it had).

        Under the Metal view the GL viewport is hidden and never draws, so
        its framebuffer is black, and a grab of the Metal widget is the
        chrome at best. The Metal renderer draws one frame offscreen at
        this small size instead (``render_image``: resident geometry, no
        egui, the surface untouched): no repack and no full-size render,
        so it stays cheap on a huge model."""
        from views.wgpu_start import blank_picture
        view = getattr(doc, "stand_in", None)
        if view is not None and getattr(view, "renderer", None) is not None:
            image = None
            if getattr(view, "packed", None) is not None and view.width() > 0 and view.height() > 0:
                try:
                    image = view.render_image(self.SAVE_PICTURE_PX)
                except Exception:  # noqa: BLE001 — a missing picture only
                    image = None
            return None if blank_picture(image) else image
        image = self._archive_thumbnail(doc.viewport)
        return None if blank_picture(image) else image

    #: Panel caches keyed by the scene's content version: two projects can
    #: be at the same number, so a switch forgets them all.
    _VERSION_CACHES = ("_in_model_cv", "_counted_version", "_drawn_signature",
                       "_swatch_sig", "_pending_refresh", "_count_job",
                       # …and what Materials remembers of each mesh: it held
                       # a closed project's 2 722 meshes (the SPR, ~2.5 GB).
                       "_worn_by_mesh")

    def _forget_panel_caches(self) -> None:
        from PySide6.QtCore import QObject
        for obj in self.findChildren(QObject):
            d = getattr(obj, "__dict__", None)
            if not d:
                continue
            for name in self._VERSION_CACHES:
                if name in d:
                    d[name] = None

    def _enter_document(self, doc) -> None:
        self._show_renderer_note(doc)
        if not getattr(doc, "_awaiting_picture", False):
            self._drain_document_startup(doc)
        vp = doc.viewport
        from core import axes as _axes
        from core import units as _units
        _units.bind_scene(vp.scene)
        _axes.sync(getattr(vp.scene, "drawing_frame", None))
        self._forget_panel_caches()
        # View preferences changed while this tab was in the back.
        from views.document_interaction import _load_edit_rest_mode
        vp.set_edit_rest_mode(_load_edit_rest_mode())
        self._activate_tool("select")
        self._forget_coordinate()
        self._sync_projection_buttons(vp.camera.projection_mode())
        self._sync_style_menu()
        self._sync_section_menu()
        georef = getattr(self, "georef_tray", None)
        if georef is not None:
            try:
                georef.base_map._sync_from_scene()
                georef.base_map.sync_photo_mesh()
            except Exception:  # noqa: BLE001 — a panel never blocks the switch
                pass
        self.viewport.notify_scene_changed()    # panels, title, extensions
        if doc.composer is not None and doc.composer_shown:
            doc.composer.show()
        self.documentChanged.emit()
        self._refresh_scene_tabs()
        vp.setFocus()

    # ---- every project closed: the empty area --------
    #: What a file dropped on the empty area may be, besides an extension's
    #: own types (``open_path``).
    OPENABLE = (*built.DOCUMENT_SUFFIXES, ".dae", ".skp", ".rfa", ".dxf", ".dwg")

    def is_empty(self) -> bool:
        """No project open: the empty area is shown."""
        return bool(self.__dict__.get("_empty_on"))

    def _show_empty_area(self) -> None:
        """Every project closed: the hatched area with only «+» above —
        the tabs, the views button, the scene row, the panels and their
        handles and the sheets strip go (the panels as the user had them,
        to come back the same). The blank project waits behind."""
        if self._empty_on:
            return
        self._empty_on = True
        self._activate_tool("select")
        bar = self._project_tabs
        bar.tabs.setVisible(False)
        self._hide_panels()
        strip = getattr(self, "_sheet_tabs", None)
        if strip is not None:
            strip.setVisible(False)
        self._doc_stack.setVisible(False)
        self.set_tool_filter(set())
        self._refresh_scene_tabs()
        bar.plus.setFocus()
        self._update_title()

    def _leave_empty_area(self) -> None:
        """A project comes in (+, New, Open, a dropped file): everything
        the empty area hid comes back."""
        if not self.__dict__.get("_empty_on"):
            return
        self._empty_on = False
        bar = self._project_tabs
        bar.tabs.setVisible(True)
        self.set_tool_filter(None)
        self._doc_stack.setVisible(True)
        self._show_panels_again()
        strip = getattr(self, "_sheet_tabs", None)
        if strip is not None and self._workspace is None:
            strip.setVisible(True)
        self._refresh_scene_tabs()
        self.viewport.setFocus()
        self._update_title()

    def _hide_panels(self) -> None:
        """The docks open now (either side, not floating) go, remembered
        with the tab that was in front."""
        from PySide6.QtWidgets import QDockWidget
        docks = [d for d in self.findChildren(QDockWidget)
                 if d.parent() is self and not d.isFloating()
                 and d.toggleViewAction().isChecked()]
        self._panels_hidden = (
            docks, [d for d in docks if d.isVisible()
                    and not d.visibleRegion().isEmpty()])
        for d in docks:
            d.hide()

    def _show_panels_again(self) -> None:
        saved = self.__dict__.get("_panels_hidden")
        if not saved:
            return
        self._panels_hidden = None
        from shiboken6 import isValid
        docks, on_top = saved
        for d in docks:
            if isValid(d):
                d.show()
        for d in on_top:
            if isValid(d):
                d.raise_()

    def _update_tab(self, doc=None) -> None:
        """The tab's name, its unsaved-changes dot and its tooltip."""
        bar = self.__dict__.get("_project_tabs")
        doc = doc or self.__dict__.get("_doc")
        if bar is None or doc is None or doc.viewport is None:
            return
        if doc is self._doc:
            dirty = self._is_dirty() if self._workspace is None else False
        else:
            dirty = doc.viewport.scene.content_version != doc.saved_content
        tip = str(doc.path) if doc.path is not None else doc.display_name()
        label = doc.display_name() + (' [read-only]' if getattr(doc, 'read_only', False) else '')
        bar.set_label(doc, label, dirty, tip)

    def _doc_is_dirty(self, doc) -> bool:
        if doc is self._doc:
            return self._is_dirty()
        return doc.viewport.scene.content_version != doc.saved_content

    def close_document(self, doc=None, ask: bool = True) -> bool:
        """Close a project's tab (the active one by default), asking about
        ITS unsaved changes first. The last tab closed leaves an empty
        project, never an empty window. False when the user cancelled."""
        doc = doc or self._doc
        if doc not in self._documents:
            return True
        from ingetrazo_render import confirm_close_tab
        if not confirm_close_tab(self, doc, ask):
            return False
        last = len(self._documents) == 1
        if last:
            # The last one: an empty project takes its place first (shown
            # as the empty area below, behind which it waits).
            self._new_document(activate=False)
        if doc is self._doc:
            bar = self._project_tabs
            i = bar.index_of(doc)
            docs = bar.documents()
            nxt = docs[i + 1] if i + 1 < len(docs) else docs[i - 1]
            self.activate_document(nxt, closing=True)
        self._retire_document(doc)
        if last:
            self._show_empty_area()
        return True

    def close_documents(self, docs) -> bool:
        """Close these projects one by one, each asking about its own
        unsaved changes; a Cancel stops there. Closing them all leaves an
        empty project, as closing the last tab does."""
        for doc in list(docs):
            if doc in self._documents and not self.close_document(doc):
                return False
        return True

    def _retire_document(self, doc) -> None:
        """Release the project, its autosave slot and each presentation surface."""
        from core import autosave
        try:
            autosave.clear(doc.path, untitled=doc.untitled_no)
        except OSError:
            pass
        self._documents.remove(doc)
        self._project_tabs.remove(doc)
        doc.front_view = None
        for fn in list(self.__dict__.get("_doc_close_hooks", ())):
            try:
                fn(doc.viewport.scene)
            except Exception:  # noqa: BLE001 — an extension cannot block it
                import logging
                logging.getLogger("ingetrazo.plugins").exception(
                    "on_document_closed hook failed")
        comp, doc.composer = doc.composer, None
        if comp is not None:
            comp.hide()
            comp.deleteLater()
        vp, doc.viewport = doc.viewport, None
        display = getattr(vp, "keys_stand_in", None)
        if vp is not None:
            try:
                vp.set_active_tool(None)
            except Exception:  # noqa: BLE001 — never block closing on cleanup
                pass
            vp.clipboard = None
            if vp.keys_stand_in is None:
                self._doc_stack.removeWidget(vp)
            vp.hide()
            # Once it is really gone, give its memory back to the system.
            vp.destroyed.connect(self._schedule_reclaim)
            vp.deleteLater()
        stand_in, doc.stand_in = doc.stand_in, None
        if stand_in is not None:
            self._doc_stack.removeWidget(stand_in)
            stand_in.hide()
        from views.wgpu_view import WgpuView
        if isinstance(display, WgpuView):
            display._refresh_timer.stop()
            display._repaint_timer.stop()
            display.close()
            display.deleteLater()
        self._update_title()

    #: How long after a closed project's viewport is deleted its memory is
    #: reclaimed: the panels showing it let go on their own deferred
    #: refresh (the Browser's comes 300 ms after the switch and holds every
    #: group until then).
    RECLAIM_DELAY_MS = 1000

    def _schedule_reclaim(self, *_args) -> None:
        # The window as context: closing it within the delay drops the call.
        QTimer.singleShot(self.RECLAIM_DELAY_MS, self, self._reclaim_memory)

    def _reclaim_memory(self):
        from ingetrazo_render import reclaim_closed_documents
        reclaim_closed_documents()

    # ---- Extension workspaces (ExtensionApp.enter_workspace) ----------------
    def enter_workspace(self, workspace) -> bool:
        """Show an extension's own document instead of the model — a CAM
        job's drawing on its stock, say (docs/plugins.md, «Workspaces»).

        The model is PARKED, not closed: its scene, undo history, camera,
        file and saved state wait untouched until :meth:`leave_workspace`.
        Meanwhile File ▸ New / Open / Save / Save As, the title, the
        unsaved-changes prompts and quitting go to ``workspace``, the
        model's autosave pauses, and only the tools in
        ``workspace.allowed_tools`` (None = all) can be picked.

        ``workspace`` provides ``scene``, ``history``, ``title()``,
        ``is_dirty()``, ``save()``, ``save_as()`` and ``confirm_leave()``
        (True when it may go: saved, discarded, or nothing to lose);
        optionally ``new()``, ``open()``, ``allowed_tools``, ``camera``
        (a :meth:`_camera_dict`) and ``left()``, called once it is gone.
        Returns False when a workspace is already shown."""
        if self._workspace is not None:
            return False
        self._leave_empty_area()
        self._activate_tool("select")
        vp = self.viewport
        vp.end_group_edit()
        self._parked = {
            "clean": not self._is_dirty(),
            "path": self._current_path,
            "import_name": self._import_name,
            "camera": self._camera_dict(),
        }
        self._parked["scene"], self._parked["history"] = vp.set_document(
            workspace.scene, workspace.history)
        self._follow_in_extra_views()
        self._workspace = workspace
        self._project_tabs.setEnabled(False)   # the workspace holds the window
        self.set_tool_filter(getattr(workspace, "allowed_tools", None))
        self._apply_camera_dict(getattr(workspace, "camera", None))
        if getattr(self, "_sheet_tabs", None) is not None:
            self._sheet_tabs.setVisible(False)   # sheets belong to the model
        self._refresh_scene_tabs()               # and so do the scenes
        self._update_title()
        return True

    def leave_workspace(self) -> bool:
        """Back to the parked model; False when the workspace would not go
        (its user cancelled the unsaved-changes prompt)."""
        ws = self._workspace
        if ws is None:
            return True
        if not ws.confirm_leave():
            return False
        self._activate_tool("select")
        p = self._parked
        if hasattr(ws, "camera"):
            ws.camera = self._camera_dict()
        self.viewport.set_document(p["scene"], p["history"])
        self._follow_in_extra_views()
        self._workspace, self._parked = None, None
        self._project_tabs.setEnabled(True)
        self.set_tool_filter(None)
        self._current_path = p["path"]
        self._import_name = p["import_name"]
        # set_document moved the version on: a model that was saved stays so.
        self._saved_version = (self._doc.viewport.scene.version
                               if p["clean"] else -1)
        self._apply_camera_dict(p["camera"])
        if getattr(self, "_sheet_tabs", None) is not None:
            self._sheet_tabs.setVisible(True)
        self._refresh_scene_tabs()
        self._update_title()
        if hasattr(ws, "left"):
            ws.left()
        return True

    def _follow_in_extra_views(self) -> None:
        """The window's viewport has another document: the project's other
        views go with it, or a tool in one would edit the one left behind."""

    def workspace(self):
        """The extension workspace shown instead of the model, or None."""
        return self._workspace

    def set_tool_filter(self, allowed) -> None:
        """Only the tools keyed in ``allowed`` can be picked (toolbar,
        menus, shortcuts); None allows every tool again."""
        self._tool_filter = set(allowed) if allowed is not None else None
        for key, action in self._tool_actions.items():
            action.setEnabled(self._tool_filter is None or key in self._tool_filter)

    # ---- Recent files -------------------------------------------------------

    def _fill_recent_menu(self) -> None:
        menu = self._recent_menu
        menu.clear()
        alive = self._recent_paths()
        if not alive:
            none = menu.addAction(tr("(no recent documents)"))
            none.setEnabled(False)
            return
        for i, entry in enumerate(alive, 1):
            p = Path(entry)
            act = menu.addAction(f"&{i}  {p.name}  —  {p.parent}")
            act.setToolTip(str(p))
            act.triggered.connect(lambda _c=False, q=p: self._open_recent(q))
        menu.addSeparator()
        clear = menu.addAction(tr("Clear list"))
        from core import recent as home_recent
        clear.triggered.connect(lambda: home_recent.clear())

    def _open_recent(self, path: Path) -> None:
        if not path.exists():
            _dialogs.warning(self, tr("Open Recent"),
                                tr("{name} is no longer there.", name=path.name))
            return
        self.viewport.end_group_edit()
        self.open_path(path)            # its own tab: nothing is discarded

    def _on_open(self) -> None:
        if self._workspace is not None and hasattr(self._workspace, "open"):
            self._workspace.open()
            return
        self.viewport.end_group_edit()
        path_str, _ = file_dialogs.getOpenFileName(
            self,
            tr("Open Built document"),
            "",
            built.DOCUMENT_FILTER + ";;Built projects (*.builtproject *.igzproj);;All files (*)",
        )
        if not path_str:
            return
        self.open_path(Path(path_str))

    def open_path(self, path: Path) -> bool:
        """Open a document at ``path`` (File dialog, CLI argument, or the OS
        file association's double-click all land here).

        ``.igz`` is our native format; ``.dae``/``.skp`` are the interchange
        formats we also register in the desktop entry, so double-clicking one
        imports it rather than failing to parse it as an IngeTrazo document.
        An extension may claim a suffix of its own
        (``ExtensionApp.add_file_opener``); anything else needs the model,
        so a workspace in front of it is left first."""
        path = Path(path)
        if getattr(self, "_closed", False):
            # The window's close was already accepted while this open was
            # still queued (the deferred command-line open, #1269):
            # loading into a closed window would bring back a session
            # the user has already ended.
            return False
        if self.__dict__.get("_empty_on"):
            # The blank project behind the empty area takes it; the area
            # comes back if nothing could be opened.
            self._leave_empty_area()
            done = self.open_path(path)
            if not done and len(self._documents) == 1 \
                    and self._doc_is_blank():
                self._show_empty_area()
            return done
        suffix = path.suffix.lower()
        opener = self.file_openers.get(suffix)
        if opener is not None:
            return bool(opener(path))
        if self._workspace is not None and not self.leave_workspace():
            return False
        # Projects in tabs (#1066): a file already open is shown, not read
        # twice; anything else gets a tab of its own — unless the one in
        # front is an untouched untitled drawing, which it simply replaces.
        for doc in self._documents:
            if doc.same_file(path):
                self.activate_document(doc)
                self._dismiss_home()
                return True
        if suffix == ".skp" and self.isVisible():
            from views.document_open import install_skp, start
            return start(self, path, install_skp)
        if (suffix in built.DOCUMENT_SUFFIXES and self.isVisible()
                and QApplication.platformName() not in ("offscreen", "minimal")
                and getattr(self._doc, "_renderer_wgpu", False)):
            from views.document_open import start
            return start(self, path, MainWindow._install_open)

        previous, created = self._doc, None
        if not self._doc_is_blank():
            created = self._new_document()
        ok = self._open_path_here(path, suffix)
        if not ok and created is not None and created in self._documents \
                and self._doc_is_blank(created):
            # Nothing came in: the tab goes, the user is back where he was.
            self.activate_document(previous, closing=True)
            self._retire_document(created)
        if ok:
            self._dismiss_home()
        return ok

    def _install_open(self, path, bar, result):
        ok = self._open_path_here(path, path.suffix.lower(), result, bar)
        return ok

    def _open_path_here(self, path: Path, suffix: str, prepared=None, bar=None) -> bool:
        """Read ``path`` into the project in front (see :meth:`open_path`)."""
        if suffix == ".rfa":
            return self._import_rfa_path(path)
        if suffix == ".dae":
            return self._import_dae_path(path)
        if suffix == ".skp":
            return self.import_skp_path(path)
        if suffix == ".dxf":
            return self._import_dxf_path(path)
        if suffix == ".dwg":
            return self._import_dwg_path(path)
        # An existing recovery slot for this document means a session that
        # never got to save it (the invariant in core/autosave.py) — offer
        # the auto-saved copy before loading the file itself.
        from core import autosave
        recovered = False
        slot = autosave.pending(path)
        defer_recovery = slot is not None and (prepared is not None or _dialogs.live(self) is not None)
        if slot is not None and not defer_recovery:
            from ingetrazo_render import named_recovery
            recovered = named_recovery(self, path)
            if recovered is None:
                return False
        if prepared is None:
            dlg, cb = self._import_progress(tr("Opening {name}…", name=path.name), monotonic=True)
        else:
            dlg, cb = bar, bar.job.update
        self._open_cancelled = False
        dlg.canceled.connect(self._on_open_cancelled)
        dlg.set_cancellable(True)

        def guarded_cb(frac, text):
            # A close that landed while the file was still coming in
            # (#1269) unwinds the load at its next milestone, instead of
            # letting it finish behind the closing window.
            if self._open_aborted or self._open_cancelled:
                raise _OpenAborted()
            cb(frac, text)

        # The viewport stays still while the document comes in: each
        # progress tick lets the window repaint, and a paint over a model
        # half loaded built every drawing chunk in one go — 61 s without
        # answering the desktop on the model of #158.
        # Fresh for this open: a failed one closes its bar while the load
        # still counts as running, which would leave the flag up.
        self._open_aborted = False
        self._open_in_progress = True
        self.viewport.setUpdatesEnabled(False)
        self._opening_document = True
        self._doc._resident_scene_ready = False
        if getattr(self._doc, "_renderer_wgpu", False):
            stand_in = getattr(self._doc, "stand_in", None)
            if stand_in is not None and hasattr(stand_in, "blank"):
                stand_in.blank()        # not the previous model while this one loads
        try:
            try:
                if prepared is None:
                    igz_format.load_into(self.viewport.scene, slot if recovered else path,
                                         progress=lambda f, t: guarded_cb(0.6 * f, t))
                else:
                    scene, packed, tracker, tree = prepared
                    history = self.viewport.history
                    history.scene = scene
                    self.viewport.set_document(scene, history)
            except _OpenAborted:
                dlg.close()
                if self._open_cancelled and not self._open_aborted:
                    self._finish_cancelled_open()
                else:
                    self._finish_aborted_open()
                return False
            except Exception as exc:  # noqa: BLE001 - surface any IO/parse error to the user
                _dialogs.close_failed(dlg, exc)
                _dialogs.critical(self, tr("Open failed"), str(exc))
                return False
            # Keep the existing steady-state allocation threshold before the
            # resident pack allocates. Only the full heap walk is deferred.
            import gc
            gc.set_threshold(self.GC_THRESHOLD0, 10, 10)
            dlg.set_cancellable(False)  # it is in: packing cannot stop
            self.viewport.history.clear()
            self.viewport.reset_texture_cache()
            self._forget_coordinate()
            self._doc._awaiting_picture = True
            self._doc._after_picture = []
            self.viewport.inference.reset()
            self._apply_camera_home()
            self._current_path = path
            self._remember_recent(path)
            self._doc._after_picture.append(self._after_links_loaded)
            self._import_name = None
            self._autosaved_version = None
            # A recovered drawing is NOT the file on disk yet: keep it dirty so
            # the title shows * and Ctrl+S makes it real (the slot stays until
            # that clean save).
            self._saved_version = (-1 if recovered
                                   else self.viewport.scene.version)
            self._sync_style_menu()      # the document may carry its own style
            self._sync_section_menu()
            # A stored survey (Track G, G6) arrives as plain arrays + images; the
            # GL upload only happens here, where there's a context.
            survey = getattr(self.viewport.scene, "photo_mesh", None)
            if survey is not None:
                self.viewport.upload_photo_mesh(survey, getattr(survey, "images", None))
                # Frame it. The default camera sits ~20 m out (sized for the scale
                # figure) and a survey is hundreds of metres across, so without this
                # the whole thing falls outside the far plane and the document opens
                # looking empty — with the mesh loaded and invisible.
                mn, mx = survey.bounds()
                if mn is not None:
                    self.viewport.camera.set_view("iso")
                    self.viewport.camera.fit_to(mn, mx)
            else:
                self.viewport.release_photo_textures()
            # Mirror the document's base map and survey into the tray, and refetch
            # the tiles for the capture the document carries — otherwise the panel
            # shows stale defaults over a scene that has its own.
            self._doc._after_picture.extend([
                self.georef_tray.base_map.sync_from_document,
                self.georef_tray.base_map.sync_photo_mesh])
            repaired = getattr(self.viewport.scene, "load_repairs", 0)
            if repaired:
                # Pieces with a coordinate that is not a number (NaN / inf) were
                # left out rather than refuse the whole document (#185). Keep it
                # unsaved, so Ctrl+S writes the cleaned file, and say so.
                self._saved_version = -1
                text = tr("{n} damaged pieces (a coordinate that is not a number) "
                          "were left out so the rest of “{name}” could open. Save "
                          "it to keep the repaired document.",
                          n=repaired, name=path.name)
                _dialogs.warning(self, tr("Document repaired"), text)
            self.viewport.notify_scene_changed()
            # The panels wait for the first picture; the scene tabs row
            # reads the scenes itself and comes up with the document.
            self._refresh_scene_tabs()
            self._update_title()
            if prepared is None:
                self._doc._after_picture.append(self.settle_heap)
            display = getattr(self._doc, "stand_in", None)
            if (getattr(self._doc, "_renderer_wgpu", False)
                    and getattr(display, "_rebuild", None) is not None):
                # The bar used to close here, and the window then showed an
                # empty view while the model was packed and sent to the
                # graphics card — 23 s on the ride with nothing saying so.
                # That is the last stage of the opening now, under the bar.
                display._refresh_timer.stop()
                if prepared is None:
                    display._rebuild(progress=lambda f, text: cb(0.6 + 0.38 * f, text))
                else:
                    display._rebuild(progress=cb, prepared=(packed, tracker, tree))
                # The snap index and the containers' boxes, left for after
                # the first picture.
                self._doc._after_picture.extend(getattr(display, "after_open", ()))
                display.after_open = []
            if self._open_aborted:
                # The close landed while the model was being packed, too
                # late to unwind the load itself — the document is in, but
                # the user's close still wins over keeping it (#1269).
                dlg.close()
                self._finish_aborted_open()
                return False
            # The heavy parts of the pick index, once the document is on
            # screen.
            self.viewport._pick_warm_due = True
            cb(1.0, "Document loaded")
            _dialogs.opened(dlg, self, path)
            if defer_recovery:
                _dialogs.offer_recovery(self, path, slot)
            # Returning to Qt lets the selected renderer present before any
            # optional panel/cache work. A timer alone is not a frame barrier.
            self._opening_document = False
            display = self._doc.stand_in
            if getattr(self._doc, "_renderer_wgpu", False):
                display._refresh_timer.start(0)
            return True
        finally:
            self._opening_document = False
            self.viewport.setUpdatesEnabled(True)
            self._open_in_progress = False
            self.viewport.update()
            dlg.close()

    def _finish_aborted_open(self) -> None:
        """The user closed the window while a file was loading (#1269)
        and the load has now unwound. What is on screen is the loader's
        half-finished work, not the user's: adopt it as the saved
        baseline so the close below does not stop to ask about
        discarding it, then close for real — deferred, so the open's
        own call stack has fully left the window first."""
        self._open_aborted = False
        self._saved_version = self.viewport.scene.version
        QTimer.singleShot(0, self.close)

    def _on_save(self) -> None:
        if self._workspace is not None:
            self._workspace.save()
            self._update_title()
            return
        self._end_group_edits()
        if self._current_path is None or not built.is_built(self._current_path):
            self._on_save_as()
            return
        self._do_save(self._current_path)

    def _on_save_as(self) -> None:
        if self._workspace is not None:
            self._workspace.save_as()
            self._update_title()
            return
        self._end_group_edits()
        default_name = (
            str((self._current_path or Path("untitled")).with_suffix(built.BUILT_SUFFIX))
        )
        path_str, _ = file_dialogs.getSaveFileName(
            self,
            tr("Save Built document"),
            default_name,
            built.BUILT_FILTER,
        )
        if not path_str:
            return
        path = Path(path_str)
        if not built.is_built(path):
            path = path.with_suffix(built.BUILT_SUFFIX)
        self._do_save(path)

    def _on_export_igz(self):
        self._end_group_edits()
        doc = self._doc
        default = (doc.path or Path("untitled")).with_suffix(".igz")
        chosen, _ = file_dialogs.getSaveFileName(
            self, tr("Save a Copy as IngeTrazo"), str(default), "IngeTrazo (*.igz)")
        if not chosen:
            return
        path = Path(chosen).with_suffix(".igz")

        def confirm(mm):
            return _dialogs.question(
                self, tr("Save the rounded copy?"),
                tr("Float32 rounding moves points by up to {mm} mm. Save the copy?",
                   mm=f"{mm:.6g}")) == QMessageBox.Yes

        try:
            import copy
            scene = copy.copy(doc.viewport.scene)
            scene.camera_home = self._camera_dict(doc.viewport)
            stats = igz_format.export_igz(scene, path, confirm)
            if stats and stats["missing"]:
                _dialogs.warning(self, tr("Saved with missing textures"),
                                    tr("Some texture images could not be packed."))
        except Exception as exc:
            _dialogs.save_failed(self, exc, self._on_export_igz)

    def _do_save(self, path: Path) -> None:
        """Write the active project to ``path``: its OWN scene and camera,
        whatever view an extension has put in front (#1106). The previous
        file is kept as ``.bak`` by the save itself (formats.igz)."""
        doc = self._doc
        vp = doc.viewport
        if self.viewport.scene is not vp.scene:
            import logging
            logging.getLogger("ingetrazo.plugins").warning(
                "Saving the project's own scene: the view in front (%s) holds "
                "another one.", type(self.viewport).__name__)
        if (getattr(doc, 'read_only', False)
                and not (self._doc_is_dirty(doc) and doc.same_file(path))):
            self.statusBar().showMessage('Choose this file in Worksession to edit or save it.', 5000)
            return
        try:
            vp.scene.camera_home = self._camera_dict(vp)
            # Linked files: their paths relative to where this is saved.
            from core import links as _links
            _links.refresh_paths(vp.scene, path)
            image = self._document_picture(doc)
            if image is not None:
                from PySide6.QtCore import QBuffer, QByteArray, QIODevice
                data = QByteArray()
                buffer = QBuffer(data)
                buffer.open(QIODevice.WriteOnly)
                image.save(buffer, 'PNG')
                vp.scene._home_thumbnail_png = bytes(data)
            # The Built UI: «Saving …» in the top bar for the time it takes.
            with _dialogs.busy(self, tr("Saving {name}…", name=path.name), output_path=path):
                stats = igz_format.save_scene(vp.scene, path) or {}
        except Exception as exc:  # noqa: BLE001
            _dialogs.save_failed(self, exc, self._on_save_as)
            return
        # A clean save ends the recovery window for this document — and for
        # the slot of what it was called until now (untitled, or the old
        # name of a Save As).
        from core import autosave
        autosave.clear(path)
        autosave.clear(doc.path, untitled=doc.untitled_no)
        # Textures travel inside the document — say so, and say it loudly when
        # an image could not be read (that face's texture will NOT travel).
        embedded = int(stats.get("embedded", 0))
        missing = int(stats.get("missing", 0))
        if missing:
            _dialogs.warning(
                self, tr("Saved with missing textures"),
                tr("{n} texture image(s) could not be read, so they were not "
                   "packed into the document — those faces will lose their "
                   "texture on another computer.", n=missing))
        elif stats.get("backup_failed"):
            self.statusBar().showMessage(tr("Could not write the backup copy."), 4000)
        elif embedded:
            self.statusBar().showMessage(
                tr("Saved — {n} texture(s) packed into the document.",
                   n=embedded), 4000)
        doc.path = path
        self._remember_recent(path)
        self._saved_version = vp.scene.version
        self._update_title()

    def _on_open_library(self) -> None:
        from views.wgpu_library import open_library
        open_library(self, "components", scope="s:online")

    _pump_file_command = staticmethod(QApplication.processEvents)

    def _import_progress(self, title, *, monotonic=False):
        """Publish an import job and return its loader progress callback."""
        from PySide6.QtWidgets import QApplication
        old = getattr(self, "_progress_dlg", None)
        if old is not None:
            try:
                old.deleteLater()
            except RuntimeError:
                pass
        dlg = self._progress_dlg = _dialogs.ChromeBar(self, title)
        last = 0

        def cb(frac, text):
            nonlocal last
            value = max(0, min(100, int(frac * 100)))
            last = max(last, value) if monotonic else value
            dlg.setValue(last)
            dlg.setLabelText(tr(text))
            QApplication.processEvents()

        return dlg, cb

    def _parse_stl_threaded(self, path, scale, simplify_mode, cb,
                            report=None):
        """Parse an STL on a worker while delivering progress on the UI thread.
        ``report`` receives the survey offset (``stl.parse_stl``)."""
        from PySide6.QtCore import QEventLoop, QObject, QThread, Qt, Signal
        from formats import stl as stl_format

        class _Worker(QObject):
            progressed = Signal(float, str)
            finished = Signal(object, object)

            def run(self):
                try:
                    mesh = stl_format.parse_stl(
                        path, progress=lambda f, t: self.progressed.emit(f, t),
                        scale=scale, simplify_mode=simplify_mode,
                        report=report)
                    self.finished.emit(mesh, None)
                except Exception as exc:  # noqa: BLE001 — reported to caller
                    self.finished.emit(None, exc)

        thread = QThread(self)
        worker = _Worker()
        worker.moveToThread(thread)
        result = {}
        loop = QEventLoop()

        class _Relay(QObject):
            def on_progress(self, fraction, text):
                cb(fraction, text)

            def on_finished(self, mesh, exc):
                result["mesh"] = mesh
                result["exc"] = exc
                loop.quit()

        relay = _Relay(self)
        worker.progressed.connect(relay.on_progress, Qt.QueuedConnection)
        worker.finished.connect(relay.on_finished, Qt.QueuedConnection)
        thread.started.connect(worker.run)
        thread.start()
        loop.exec()
        thread.quit()
        thread.wait()
        worker.deleteLater()
        relay.deleteLater()
        return result.get("mesh"), result.get("exc")

    def _on_import_rfa(self) -> None:
        path, _ = file_dialogs.getOpenFileName(
            self, tr("Import Revit family"), "",
            tr("Revit family (*.rfa);;All files (*)"))
        if path:
            self._import_rfa_path(Path(path))

    def _on_import_dae(self) -> None:
        path_str, _ = file_dialogs.getOpenFileName(
            self, tr("Import DAE"), "",
            tr("COLLADA (*.dae);;All files (*)"))
        if not path_str:
            return
        self._import_dae_path(Path(path_str))

    def _on_import_glb(self) -> None:
        path_str, _ = file_dialogs.getOpenFileName(
            self, tr("Import glTF/GLB"), "",
            tr("glTF binary (*.glb *.gltf);;All files (*)"))
        if not path_str:
            return
        self._import_glb_path(Path(path_str))

    # ---- SKP import: IngeTrazo's own reader (formats/skp.py) -----------------
    def _on_import_skp(self) -> None:
        path_str, _ = file_dialogs.getOpenFileName(
            self, tr("Import SKP"), "",
            tr("SKP (*.skp);;All files (*)"))
        if not path_str:
            return
        self.import_skp_path(Path(path_str))

    # ---- linked documents (core/links.py) ------------------------------------
    def _on_link_igz(self) -> None:
        from app_shell.files import link_document
        link_document(self)

    def _fill_links_menu(self, menu) -> None:
        """File ▸ Linked Files: one submenu per source — open it, reload
        it, pack it into this document, show its tags as it has them."""
        from core import links
        menu.clear()
        scene = self.viewport.scene
        paths = links.sources(scene)
        if not paths:
            act = menu.addAction(tr("No linked documents"))
            act.setEnabled(False)
            return
        for path in paths:
            name = Path(path).name
            sub = menu.addMenu(name if Path(path).is_file()
                               else tr("{name} (missing)", name=name))
            sub.setToolTipsVisible(True)
            sub.addAction(tr("Open"), lambda p=path: self.open_path(Path(p)))
            sub.addAction(tr("Reload"), lambda p=path: self._reload_link(p))
            placements = links.linked_groups(scene, path)
            for index, group in enumerate(placements, 1):
                target = sub if len(placements) == 1 else sub.addMenu(
                    f"{index}. {group.name or name}")
                nested = target.addAction(tr("Include nested linked files"))
                nested.setCheckable(True)
                nested.setChecked(links.link_of(group).get("include_nested", True))
                nested.toggled.connect(lambda on, g=group: self._set_link_nested(g, on))
            pack = sub.addAction(tr("Pack into this document"))
            pack.setCheckable(True)
            pack.setChecked(links.is_packed(scene, path))
            pack.setToolTip(tr(
                "Keep the linked geometry inside this file, so it opens "
                "without the source. Unchecked, only the link is saved."))
            pack.toggled.connect(lambda on, p=path: self._pack_link(p, on))
            sub.addAction(tr("Show tags as the source has them"),
                          lambda p=path: self._reset_link_tags(p))
        menu.addSeparator()
        menu.addAction(tr("Reload all"), self._reload_all_links)

    def _on_import_igz(self) -> None:
        """Bring another IngeTrazo document in as ONE component, placed
        with a click — the classic import of a .skp. Furniture drawn in its
        own file (a pergola, an arch, a lamp post) lands in the plaza with
        its groups, materials and layers intact (see :mod:`core.insert`)."""
        start = (str(self._current_path.parent)
                 if self._current_path is not None else "")
        path_str, _ = file_dialogs.getOpenFileName(
            self, tr("Import IngeTrazo document"), start, IGZ_FILE_FILTER)
        if not path_str:
            return
        path = Path(path_str)
        if (self._current_path is not None
                and path.resolve() == self._current_path.resolve()):
            _dialogs.warning(
                self, tr("Import IngeTrazo document"),
                tr("That is the document you are editing."))
            return
        try:
            comp = self.import_igz_path(path)
        except Exception as exc:  # noqa: BLE001
            _dialogs.critical(
                self, tr("Import IngeTrazo document failed"), str(exc))
            return
        if comp is None:
            _dialogs.warning(
                self, tr("Import IngeTrazo document"),
                tr("“{name}” has no geometry.", name=path.name))

    def _on_import_orthophoto(self):
        from views.survey_import import import_orthophoto
        import_orthophoto(self)

    def _set_survey_capture(self, datum, lo, hi):
        self.georef_tray.base_map.setup_for_bounds(datum, lo, hi)


    def add_appimage_to_menu(self) -> None:
        """Write the launcher + icon for the running AppImage."""
        from core.appimage import appimage_path, integrate
        img = appimage_path()
        if img is None:
            return
        try:
            f = integrate(img)
        except OSError as exc:
            _dialogs.warning(self, tr("Add to the applications menu"),
                                str(exc))
            return
        self.statusBar().showMessage(
            tr("Launcher added: {path}", path=str(f)), 6000)

    def remove_appimage_from_menu(self) -> None:
        from core.appimage import remove
        remove()
        self.statusBar().showMessage(
            tr("Launcher removed from the applications menu"), 5000)

    def _on_import_dwg(self) -> None:
        path_str, _ = file_dialogs.getOpenFileName(
            self, tr("Import DWG"), "",
            tr("AutoCAD DWG (*.dwg);;All files (*)"))
        if path_str:
            self._import_dwg_path(Path(path_str))

    def _on_import_dxf(self) -> None:
        path_str, _ = file_dialogs.getOpenFileName(
            self, tr("Import DXF"), "",
            tr("AutoCAD DXF (*.dxf);;All files (*)"))
        if path_str:
            self._import_dxf_path(Path(path_str))

    def _on_import_photomesh(self):
        from views.survey_import import import_photomesh
        import_photomesh(self)

    def _on_photo_progress(self, fraction: float, text: str) -> None:
        """Progress from the import worker, delivered on the UI thread."""
        callback = getattr(self, "_photo_progress", None)
        if callback is not None:
            callback(fraction, text)

    def _load_photomesh_threaded(self, path, datum, cb):
        """Parse the OBJ and decode its atlases off the UI thread.

        Both halves are slow enough to freeze the window — the real survey this
        was built against is 40 MB of OBJ plus 455 MB of PNG, and one atlas
        alone takes ~9 s to decode the first time. Neither touches ``Scene``,
        so both are safe off-thread; the GL upload stays with the caller.
        """
        from PySide6.QtCore import QEventLoop, QObject, QThread, Qt, Signal

        from ingetrazo_render import read_survey_mesh

        gl_max = self.viewport.max_texture_size()

        class _Worker(QObject):
            progressed = Signal(float, str)
            finished = Signal(object, object, object)   # (mesh, images, exc)

            def run(self):
                try:
                    mesh, images = read_survey_mesh(path, datum, gl_max, self.progressed.emit)
                    self.finished.emit(mesh, images, None)
                except Exception as exc:  # noqa: BLE001 — reported to caller
                    self.finished.emit(None, None, exc)

        thread = QThread(self)
        worker = _Worker()
        worker.moveToThread(thread)
        result = {}

        # A BOUND METHOD, not a lambda. A queued connection to a bare lambda has
        # no receiver QObject to marshal into, so Qt ends up running it on the
        # *worker* thread and the progress dialog's timer gets stopped
        # cross-thread ("Timers cannot be stopped from another thread"). Bound
        # to ``self``, delivery lands on the UI thread where the dialog lives.
        self._photo_progress = cb
        worker.progressed.connect(self._on_photo_progress, Qt.QueuedConnection)

        loop = QEventLoop()

        def _done(mesh, images, exc):
            result.update(mesh=mesh, images=images, exc=exc)
            loop.quit()

        worker.finished.connect(_done, Qt.QueuedConnection)
        thread.started.connect(worker.run)
        thread.start()
        loop.exec()
        thread.quit()
        thread.wait()
        worker.deleteLater()
        return result.get("mesh"), result.get("images"), result.get("exc")

    def _on_import_georef(self):
        from views.survey_import import import_alignment
        import_alignment(self)

    def _on_import_survey_points(self):
        from views.survey_import import import_points
        import_points(self)

    def _on_export_ifc(self) -> None:
        self.viewport.end_group_edit()
        from core.bim import collect_objects
        if not collect_objects(self.viewport.scene):
            _dialogs.information(
                self, tr("Export IFC"),
                tr("Nothing to export: tag geometry in the BIM panel first "
                   "(only tagged objects go to IFC)."))
            return
        path_str, _ = file_dialogs.getSaveFileName(
            self, tr("Export IFC"), "model.ifc",
            tr("IFC4 (*.ifc);;All files (*)"))
        if not path_str:
            return
        from core.bim_registry import get as get_registry
        from PySide6.QtWidgets import QInputDialog
        profiles = list(get_registry(self.viewport.scene).get("export_profiles", {}))
        profile = profiles[0] if profiles else None
        if len(profiles) > 1:
            profile, accepted = QInputDialog.getItem(
                self, tr("Export IFC"), tr("Export profile"), profiles, 0, False)
            if not accepted:
                return
        try:
            count = ifc_format.save_ifc(
                self.viewport.scene, path_str, profile=profile,
                project_directory=Path(self._current_path).parent if self._current_path else None)
        except Exception as exc:  # noqa: BLE001
            _dialogs.critical(self, tr("Export IFC failed"), str(exc))
            return
        self.statusBar().showMessage(
            tr("{n} IFC elements exported to {path}",
               n=count, path=path_str), 5000)
        if count.problems or count.ids:
            import json
            details = [
                f"{problem['name']}: " + "; ".join(problem["messages"])
                for problem in count.problems]
            if count.ids:
                details.append(json.dumps(count.ids, indent=2))
            self.statusBar().showMessage(
                tr("{n} IFC elements exported to {path}", n=count, path=path_str)
                + f" — {len(count.problems)} object problems", 10000)
            dialog = QMessageBox(self)
            dialog.setWindowTitle(tr("IFC export report"))
            dialog.setText("\n".join(details))
            dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
            dialog.setModal(False)
            dialog.show()

    def _on_export_stl(self):
        from ingetrazo_render import delivery_export_form as export_form
        return export_form(self, one_off=True, initial_format="stl")

    def _on_export_obj(self) -> None:
        self._export("OBJ", "obj", tr("Wavefront OBJ (*.obj)"), obj_format.save_obj)

    def _on_export_glb(self) -> None:
        """Single-file 3D export (geometry + materials + textures embedded).
        Best format for 'send it so a colleague can view it' — no texture folder
        to lose, opens in Blender / web viewers / Windows 3D Viewer.

        With something selected, it asks first whether the file takes the
        whole model or just the selection (#353)."""
        only = self._ask_export_scope()
        if only is False:
            return                          # cancelled
        if only:
            def writer(scene, path):
                gltf_format.save_glb(scene, path, only=only)
        else:
            writer = gltf_format.save_glb
        self._export("GLB", "glb", tr("glTF binary (*.glb)"), writer)

    def _ask_export_scope(self):
        """With a selection: ``set`` of the selected entities to export
        only them, ``None`` for the whole model, ``False`` if cancelled.
        Without one: ``None`` at once."""
        sel = set(self.viewport.scene.selection)
        if not sel:
            return None
        answer = _dialogs.ask(
            self, tr("What should be exported?"), tr("Export the whole model, or only what is selected?"),
            [("cancel", tr("Cancel")), ("whole", tr("Whole model")),
             ("part", tr("Only the selection"))],
            default="part", cancel="cancel")
        return {"part": sel, "whole": None}.get(answer, False)

    def _on_export_dae(self) -> None:
        """COLLADA export — the 'open it back in the original program' bridge. Copies the
        texture images beside the .dae (send both, or use GLB)."""
        self._export("COLLADA", "dae", tr("COLLADA (*.dae)"), dae_format.save_dae)

    def _on_export_view_dxf(self):
        from ingetrazo_render import delivery_export_form as export_form
        return export_form(self, one_off=True, initial_format="dxf")

    def _export(self, label: str, suffix: str, file_filter, writer) -> None:
        base = (self._current_path.stem if self._current_path is not None
                else "untitled")
        path_str, _ = file_dialogs.getSaveFileName(
            self, tr("Export {label}", label=label), f"{base}.{suffix}", file_filter)
        if not path_str:
            return
        path = Path(path_str)
        if path.suffix.lower() != f".{suffix}":
            path = path.with_suffix(f".{suffix}")
        try:
            writer(self.viewport.scene, path)
        except Exception as exc:  # noqa: BLE001
            _dialogs.critical(
                self, tr("Export {label} failed", label=label), str(exc))
            return
        self.statusBar().showMessage(
            tr("Exported {label} → {name}", label=label, name=path.name), 3000)

    def _on_scene_version_changed(self, _version: int) -> None:
        # Something came into the blank project behind the empty area (an
        # import, a paste, an extension, the AI bridge): it is on show.
        if self.__dict__.get("_empty_on") and not self._doc_is_blank():
            self._leave_empty_area()
        self._update_title()

    def _update_title(self) -> None:
        if self.__dict__.get("_empty_on"):
            self.setWindowTitle("Built")
            return
        marker = " *" if self._is_dirty() else ""
        if self._workspace is not None:
            self.setWindowTitle(f"Built — {self._workspace.title()}{marker}")
            return
        if self._current_path is not None:
            name = self._current_path.name
        elif self._import_name:
            name = self._import_name
        else:
            name = tr("Untitled")
        self.setWindowTitle(f"Built — {name}{marker}")
        self._update_tab()
        # The composer has a taskbar button of its own: it names the model.
        composer = getattr(self, "_composer", None)
        if composer is not None:
            composer.set_document_title(f"{name}{marker}")
        self._refresh_sheet_tabs()      # a new / opened document: its sheets

    # ---- Window lifecycle ---------------------------------------------------
    def closeEvent(self, event) -> None:
        if _dialogs.current_sheet(self) is not None:
            # A question is on the chrome's sheet: answer it first, as a
            # modal dialog would have kept the window from closing.
            event.ignore()
            return
        if getattr(self, "_open_in_progress", False):
            # A file is still loading into this window (#1269). What is
            # on screen is the loader's half-finished work, not the
            # user's, so there is nothing to ask about discarding — but
            # accepting the close here would tear the window down under
            # the loader. Refuse it for the moment and flag the load to
            # unwind at its next progress milestone; the abort path
            # then closes the window for real.
            self._open_aborted = True
            event.ignore()
            return
        # A workspace first (its own unsaved job), then the model it parked.
        if self._workspace is not None and not self.leave_workspace():
            event.ignore()
            return
        if not self._confirm_quit():
            event.ignore()
            return
        from ingetrazo_render import chrome_sheet
        chrome_sheet.close(self)
        _forms.close_all(self)
        self._release_check = None
        # A clean goodbye: whatever the user decided (save or discard), this
        # session's recovery slots no longer speak for anyone.
        from core import autosave
        for doc in self._documents:
            autosave.clear(doc.path, untitled=doc.untitled_no)
        # Quitting from the empty area: the panels it hid are the user's
        # arrangement, kept for the next session.
        self._show_panels_again()
        # The window arrangement (toolbars, docks, size) IS a preference.
        st = QSettings()
        st.setValue("ui/window_state", self.saveState())
        st.setValue("ui/window_geometry", self.saveGeometry())
        for doc in self._documents:
            if doc.composer is not None:
                doc.composer.close()
            if doc.viewport is not None:
                display = getattr(doc.viewport, "keys_stand_in", None)
                if display is not None:
                    display.close()
        # The close is really happening: an open still queued behind
        # this window (the deferred command-line open) must not load
        # into it afterwards (#1269).
        self._closed = True
        super().closeEvent(event)
