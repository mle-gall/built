"""UI spike (branch ``ui/egui-spike``): the viewport chrome drawn by egui
inside the Metal renderer, in the same frame as the model.

The Metal view draws the tool palette, command cluster and status line.
Both hosts bind the Rust view registry to their document controller.
"""
from __future__ import annotations

import os
from pathlib import Path

from PySide6.QtCore import Qt, QTimer, QSize
from PySide6.QtGui import QImage, QKeySequence

# Inter 400/500/600 (SIL OFL, resources/fonts/inter); the system font if missing.
FONT = str(Path(__file__).resolve().parents[1] / "resources" / "fonts" / "inter")
if not os.path.isdir(FONT):
    FONT = "/System/Library/Fonts/SFNS.ttf"
_ICON_PX = 48


def _icon_rgba(key: str) -> tuple[int, int, bytes]:
    """The tool's own icon as white with its alpha, so egui can tint it."""
    from views.icons import tool_icon
    icon = tool_icon(key)
    if icon.isNull():
        return 0, 0, b""
    image = icon.pixmap(QSize(_ICON_PX, _ICON_PX)).toImage().convertToFormat(
        QImage.Format_RGBA8888)
    w, h = image.width(), image.height()
    data = bytearray(bytes(image.constBits())[: w * h * 4])
    data[0::4] = data[1::4] = data[2::4] = b"\xff" * (w * h)
    return w, h, bytes(data)


def _drawn_icon(key: str) -> tuple[int, int, bytes]:
    """Icons the app has no art for yet (view-state toggles), drawn in the
    same 48-px grid and stroke as views/icons.py, white for tinting."""
    from PySide6.QtCore import QPointF, QRectF
    from PySide6.QtGui import QColor, QPainter, QPen
    image = QImage(_ICON_PX, _ICON_PX, QImage.Format_RGBA8888)
    image.fill(Qt.transparent)
    p = QPainter(image)
    p.setRenderHint(QPainter.Antialiasing, True)
    solid = QPen(QColor(255, 255, 255), 3.0, Qt.SolidLine, Qt.RoundCap, Qt.RoundJoin)
    dashed = QPen(QColor(255, 255, 255, 200), 2.2, Qt.DashLine, Qt.RoundCap, Qt.RoundJoin)
    dashed.setDashPattern([1.6, 1.8])
    front, back = QRectF(10, 18, 20, 20), QRectF(18, 10, 20, 20)
    corners = [(front.topLeft(), back.topLeft()), (front.topRight(), back.topRight()),
               (front.bottomLeft(), back.bottomLeft()), (front.bottomRight(), back.bottomRight())]
    if key == "xray":
        # A cube you see through: faces washed, every edge drawn.
        p.setPen(Qt.NoPen)
        p.setBrush(QColor(255, 255, 255, 70))
        p.drawRect(back)
        p.drawRect(front)
        p.setBrush(Qt.NoBrush)
        p.setPen(solid)
        p.drawRect(front)
        p.drawRect(back)
        for a, b in corners:
            p.drawLine(a, b)
    elif key == "back_edges":
        # The near edges solid, the ones behind dashed.
        p.setPen(dashed)
        p.drawLine(back.topLeft(), back.bottomLeft())
        p.drawLine(back.bottomLeft(), back.bottomRight())
        p.drawLine(front.bottomLeft(), back.bottomLeft())
        p.setPen(solid)
        p.drawRect(front)
        p.drawLine(back.topLeft(), back.topRight())
        p.drawLine(back.topRight(), back.bottomRight())
        p.drawLine(front.topLeft(), back.topLeft())
        p.drawLine(front.topRight(), back.topRight())
        p.drawLine(front.bottomRight(), back.bottomRight())
    elif key == "section_cut":
        # A box sliced by a plane: the near half kept, its cut face hatched,
        # the far half gone (dashed), the plane drawn through it.
        p.setPen(dashed)
        p.drawLine(QPointF(25, 15), QPointF(37, 15))
        p.drawLine(QPointF(37, 15), QPointF(37, 35))
        p.drawLine(QPointF(37, 35), QPointF(25, 35))
        p.setPen(Qt.NoPen)
        p.setBrush(QColor(255, 255, 255, 90))
        p.drawRect(QRectF(11, 15, 14, 20))
        p.setBrush(Qt.NoBrush)
        hatch = QPen(QColor(255, 255, 255, 190), 1.6)
        p.setPen(hatch)
        for k in range(0, 34, 6):
            p.drawLine(QPointF(max(11, 11 + k - 20), min(35, 15 + k)), QPointF(min(25, 11 + k), max(15, 15 + k - 14)))
        p.setPen(solid)
        p.drawLine(QPointF(25, 15), QPointF(11, 15))
        p.drawLine(QPointF(11, 15), QPointF(11, 35))
        p.drawLine(QPointF(11, 35), QPointF(25, 35))
        plane = QPen(QColor(255, 255, 255), 3.4, Qt.SolidLine, Qt.RoundCap)
        p.setPen(plane)
        p.drawLine(QPointF(25, 9), QPointF(25, 41))
    elif key == "shadows":
        # A sun, a box, and the shadow it casts on the ground.
        p.setPen(Qt.NoPen)
        p.setBrush(QColor(255, 255, 255, 110))
        from PySide6.QtGui import QPolygonF
        p.drawPolygon(QPolygonF([QPointF(24, 38), QPointF(36, 38), QPointF(44, 31), QPointF(32, 31)]))
        p.setBrush(Qt.NoBrush)
        p.setPen(solid)
        p.drawRect(QRectF(22, 22, 14, 16))
        p.drawEllipse(QPointF(12, 12), 4.5, 4.5)
        ray = QPen(QColor(255, 255, 255), 2.2, Qt.SolidLine, Qt.RoundCap)
        p.setPen(ray)
        import math
        for a in range(0, 360, 45):
            r = math.radians(a)
            p.drawLine(QPointF(12 + 7.5 * math.cos(r), 12 + 7.5 * math.sin(r)),
                       QPointF(12 + 10 * math.cos(r), 12 + 10 * math.sin(r)))
    elif key == "hidden_geometry":
        # A face whose soft diagonal and smoothed edges show dashed.
        p.setPen(solid)
        p.drawRect(QRectF(11, 11, 26, 26))
        p.setPen(dashed)
        p.drawLine(QPointF(11, 37), QPointF(37, 11))
        p.drawLine(QPointF(11, 24), QPointF(24, 11))
        p.drawLine(QPointF(24, 37), QPointF(37, 24))
    p.end()
    return _ICON_PX, _ICON_PX, bytes(image.constBits())[: _ICON_PX * _ICON_PX * 4]


def _window(view):
    win = view.window()
    return win if hasattr(win, "_tool_actions") else None


def attach(view) -> None:
    """Called once the view has its renderer."""
    renderer = view.renderer
    if getattr(view, "_ui_attached", False):
        return
    win = _window(view)
    if win is None:
        return
    view._ui_timer = view._view_timer(view.update)
    renderer.ui_enable(FONT if os.path.exists(FONT) else None)
    from views import wgpu_library
    wgpu_library.install(win)
    if hasattr(renderer, "ui_set_dock_width"):
        from core.settings import settings as QSettings
        try:
            renderer.ui_set_dock_width(float(QSettings().value("ui/dock_width", 300.0)))
        except (TypeError, ValueError):
            pass
    view._ui_tools_sent = False
    view._ui_attached = True
    if view.source_viewport is not None:
        view.source_viewport._chrome_owns_labels = True


#: Drawing and Export have room for previews and a readiness checklist.
WIDE_DOCK = 430.0
#: Drawing and Export reserve room for their columns.
WIDE_LENSES = (1, 2)


def dock_width(lens, settings=None) -> float:
    """The dock width for lens ``lens``: the user's width for
    Filing; for the wide sets the width last dragged there, at least
    ``WIDE_DOCK`` and never narrower than the user's own."""
    from core.settings import settings as QSettings
    st = settings if settings is not None else QSettings()
    try:
        own = float(st.value("ui/dock_width", 300.0))
    except (TypeError, ValueError):
        own = 300.0
    if lens not in WIDE_LENSES:
        return own
    try:
        wide = float(st.value("ui/dock_width_wide", WIDE_DOCK))
    except (TypeError, ValueError):
        wide = WIDE_DOCK
    return max(own, wide)


def apply_dock_width(view, settings=None) -> None:
    renderer = getattr(view, "renderer", None)
    if renderer is None or not hasattr(renderer, "ui_set_dock_width"):
        return
    renderer.ui_set_dock_width(dock_width(view.__dict__.get("_ui_lens", 0), settings))


def keep_dock_width(view, arg, settings=None) -> None:
    """The grip let go (or double-clicked): keep the width for the column
    set it was dragged in, so Filing comes back to the user's width."""
    from core.settings import settings as QSettings
    st = settings if settings is not None else QSettings()
    try:
        w = float(arg or 300)
    except ValueError:
        return
    lens = view.__dict__.get("_ui_lens", 0) if view is not None else 0
    st.setValue("ui/dock_width_wide" if lens in WIDE_LENSES else "ui/dock_width", w)


def _send_tools(view, win) -> None:
    """Once the window has built its tool actions (after the first view)."""
    tools = getattr(win, "_tools", {})
    groups = []
    disabled = []
    view._ui_palette_runs = {}
    from ingetrazo_render import rail_layout
    for keys in rail_layout():
        row = []
        for key, binding in keys:
            if binding:
                action = getattr(win, binding, None)
                if action is not None:
                    view._ui_palette_runs[key] = action.trigger
            else:
                action = win._tool_actions.get(key) if key in tools else None
            if action is None:
                continue
            if not action.isEnabled():
                disabled.append(key)
            shortcut = action.shortcut().toString(QKeySequence.NativeText)
            w, h, rgba = _icon_rgba(key)
            row.append((key, action.text().replace("&", ""), shortcut, w, h, rgba))
        if row:
            groups.append(row)
    if disabled:
        view.renderer.ui_set_tools(groups, disabled)
    else:
        view.renderer.ui_set_tools(groups)
    # Full bleed: the palette replaces every tool bar.
    from PySide6.QtWidgets import QToolBar
    for bar in win.findChildren(QToolBar):
        bar.hide()
    _merge_title_row(win)
    _one_theme(win)
    # Full bleed: the right column stands in for the docks; they stay
    # reachable from the Window menu until it covers all they do.
    from PySide6.QtWidgets import QDockWidget
    for dock in win.findChildren(QDockWidget):
        # Only the side panels; never a dock that holds the view itself.
        if dock.isAncestorOf(view) or win.dockWidgetArea(dock) not in (
                Qt.RightDockWidgetArea, Qt.LeftDockWidgetArea):
            continue
        dock.hide()
    # The hint and the measurements now live in the chrome: their Qt
    # copies keep their state but take no room.
    status = win.statusBar()
    for w in (getattr(status, "_msg", None), getattr(win, "_vcb_name", None),
              getattr(win, "_vcb_value", None)):
        if w is not None:
            w.setMaximumWidth(0)
    icons = []
    from views.view_commands import entries
    for group, _, _, _, key in entries():
        if group not in ("nav", "walkthrough", "Views", "Projection") or not key:
            continue
        w, h, rgba = _icon_rgba(key)
        icons.append((key, w, h, rgba))
    icons.append(("four_views", *_icon_rgba("four_views")))
    for key in ("xray", "back_edges", "hidden_geometry", "section_cut", "shadows"):
        icons.append((key, *_drawn_icon(key)))
    view.renderer.ui_set_icons(icons)
    view._ui_tools_sent = True


#: Window buttons' room at the top left (mac_glass: 14 + three buttons).
LIGHTS_ROOM = 86.0
#: The docked top bar (ui.rs TOP_H): its empty parts move the window.
TOP_H = 40.0


def _merge_title_row(win) -> None:
    """No title row at all (Maxence 2026-11-22: "can't we get rid of it?"):
    the model reaches the window's top edge; the project tabs are a glass
    capsule after the window buttons, on the top row of the chrome; the
    buttons sit centred on that row."""
    chrome = getattr(win, "_mac_chrome", None)
    bar = getattr(win, "_project_tabs", None)
    if bar is not None:
        bar.hide()
        bar.setMaximumHeight(0)
    area = getattr(win, "_doc_area", None)
    if area is not None and area.layout() is not None:
        area.layout().setContentsMargins(0, 0, 0, 0)
    if chrome is None:
        return
    from PySide6.QtCore import QPoint

    def fit():
        from views.mac_glass import _window_alive
        if not _window_alive(win):
            return
        if win.windowState() & Qt.WindowFullScreen:
            chrome._glass.place_lights(None)
            return
        view = getattr(getattr(win, "_doc", None), "stand_in", None)
        top = view.mapTo(win, QPoint(0, 0)).y() if view is not None else 0
        chrome._glass.place_lights(top + TOP_H / 2)     # the top bar's centre
    chrome.fit_lights = fit
    QTimer.singleShot(0, fit)
    QTimer.singleShot(400, fit)


def _projects(view, win) -> None:
    bar = getattr(win, "_project_tabs", None)
    if bar is None:
        return
    current = bar.current()
    docs = []
    for i in range(bar.count()):
        text = bar.tabs.tabText(i)
        dirty = text.endswith(bar.DIRTY_MARK)
        name = text[: -len(bar.DIRTY_MARK)] if dirty else text
        docs.append((name, bar.tabs.tabData(i) is current, dirty))
    if docs != getattr(view, "_ui_docs", None):
        view._ui_docs = docs
        view.renderer.ui_set_projects(LIGHTS_ROOM, docs)


def _project_action(win, arg) -> None:
    bar = getattr(win, "_project_tabs", None)
    if bar is None:
        return
    if arg == "views":
        bar.viewsRequested.emit()
    elif arg == "new":
        bar.newRequested.emit()
    elif arg.startswith("close:"):
        doc = bar.tabs.tabData(int(arg.partition(":")[2]))
        if doc is not None:
            bar.closeRequested.emit(doc)
    else:
        doc = bar.tabs.tabData(int(arg))
        if doc is not None:
            bar.activated.emit(doc)


#: The title row in the chrome's graphite (tokens in ui.rs): Maxence
#: 2026-11-22, "3 different themes: black top/bottom bar, blue bottom bar,
#: grey UI". The bottom Qt rows go; this row stays (it holds the window
#: buttons) and takes the same colours and capsule tabs.
_TITLE_SHEET = """
#document_area { background: #16191D; }
#project_tab_bar::tab { background: transparent; color: #C9CFD6; border: none;
    border-radius: 14px; padding: 5px 14px; margin: 0 2px; min-height: 18px; }
#project_tab_bar::tab:hover { background: rgba(255,255,255,18); color: #EEF1F4; }
#project_tab_bar::tab:selected { background: rgba(106,165,255,61); color: #FFFFFF; }
#project_views, #project_tab_new { border: none; border-radius: 14px; }
#project_views:hover, #project_tab_new:hover { background: rgba(255,255,255,18); }
"""


def _one_theme(win) -> None:
    area = getattr(win, "_doc_area", None)
    if area is not None:
        area.setStyleSheet(_TITLE_SHEET)
    status = win.statusBar()
    if status is not None:
        status.hide()
    scenes = getattr(win, "_scene_tabs", None)
    if scenes is not None:
        scenes.setMaximumHeight(0)
    # The side panels' pull handles: nothing to pull with the docks hidden.
    from PySide6.QtWidgets import QWidget
    for name in ("sidebar_handle", "left_sidebar_handle"):
        for w in win.findChildren(QWidget, name):
            w.hide()
            w.setMaximumSize(0, 0)


def _tabs(view, win) -> None:
    """Model / sheet and the scenes for the bottom capsule bar."""
    scene = win.viewport.scene
    comps = list(getattr(scene, "compositions", None) or [])
    last = min(max(getattr(win, "_last_sheet", 0), 0), len(comps) - 1) if comps else -1
    sheet = comps[last].name if comps else "New sheet"
    views = list(getattr(scene, "saved_views", None) or [])
    row = getattr(getattr(win, "_scene_tabs", None), "tabs", None)
    marked = row.marked_view() if row is not None and hasattr(row, "marked_view") else None
    from views import wgpu_sheets
    on_sheets = wgpu_sheets.shown(view)
    if wgpu_sheets.embeds(win):
        # Sheets is a workspace of this window (views/wgpu_sheets.py).
        sheet = "Sheets"
    work = [("model", "Model", "The model", "", 0 if on_sheets else 1, True),
            ("sheet", sheet, "Sheets (⌃Tab)", "", 1 if on_sheets else 0, True)]
    from ingetrazo_core import scene_tree, visibility_set_rows
    version = getattr(scene, "content_version", 0)
    cached = getattr(view, "_ui_visibility_cache", None)
    if cached is None or cached[0] is not scene or cached[1] != version:
        view._ui_visibility_cache = (scene, version, visibility_set_rows(scene))
    catalog_key = (id(scene), getattr(scene, "version", 0), id(marked))
    if not on_sheets and catalog_key != getattr(view, "_ui_scene_catalog_key", None):
        view._ui_scene_menu = scene_tree.menu_rows(scene, marked)
        view._ui_scene_catalog_key = catalog_key
    scenes, folders = ([], []) if on_sheets else view._ui_scene_menu
    tabs = [work, scenes, view._ui_visibility_cache[2] if not on_sheets else [], folders]
    if tabs != getattr(view, "_ui_tabs", None):
        view._ui_tabs = tabs
        view.renderer.ui_set_tabs(tabs)
    view._ui_scene_views = views


def _sheet_and_jobs(view, win) -> None:
    """The app sheet and the work in progress (views/chrome_dialogs.py)."""
    from views import chrome_dialogs
    sheet = chrome_dialogs.current_sheet(win)
    sig = None if sheet is None else (id(sheet), sheet.title, sheet.text, tuple(chrome_dialogs.sheet_buttons(sheet)))
    if sig != view.__dict__.get("_ui_sheet_sig", False):
        view._ui_sheet_sig = sig
        if sheet is None:
            view.renderer.ui_set_sheet("", "", "", [])
        else:
            view.renderer.ui_set_sheet(sheet.title, sheet.text, sheet.tone, chrome_dialogs.sheet_buttons(sheet))
    if sheet is not None:
        sheet.shown = True
    rows = [(j.id, j.label, j.phase, float(j.fraction), bool(j.cancellable))
            for j in chrome_dialogs.jobs(win)]
    if rows != view.__dict__.get("_ui_jobs_sig", False):
        view._ui_jobs_sig = rows
        view.renderer.ui_set_jobs(rows)
    # Polled work (the background auto-save) says when it is done only by
    # being gone: look again soon while any shows.
    import time
    now = time.monotonic()
    records = chrome_dialogs.history(win)
    recent = []
    for row in records:
        remaining = 5 - (now - row.created)
        opacity = min(1.0, remaining) if row.toast else 0.0
        recent.append((row.id, row.title, row.detail, row.tone, row.action_label,
                       opacity if remaining > 0 else -1.0))
    banner = getattr(win._doc, "_missing_link_banner", "")
    if ((recent, banner) != view.__dict__.get("_ui_activity_sig")
            and hasattr(view.renderer, "ui_set_activity")):
        view.renderer.ui_set_activity(recent, banner)
        view._ui_activity_sig = recent, banner
    view._ui_poll_jobs = bool(rows or recent)


def _chrome_form(view, win) -> None:
    """The form on top, if any (views/chrome_forms.py)."""
    if not hasattr(view.renderer, "ui_set_dialog"):
        return
    from views import chrome_forms
    chrome_forms.before_frame(view, win)


def chrome_only(view) -> bool:
    """Exclusive work holds the window (an open, an import): the model is
    half there, so a frame draws only the chrome over what is resident."""
    if not getattr(view, "_ui_attached", False):
        return False
    win = _window(view)
    if win is None:
        return False
    from views import chrome_dialogs
    return chrome_dialogs.exclusive(win)


def chrome_frame(view):
    """A frame of chrome alone: the projects, the sheet and the jobs, over
    the resident (blank) model. Nothing here reads the scene."""
    renderer = view.renderer
    if renderer is None:
        return None
    win = _window(view)
    if win is not None:
        _projects(view, win)
        from views import wgpu_tabs
        wgpu_tabs.before_frame(view, win)
        _sheet_and_jobs(view, win)
        _chrome_form(view, win)
        from views import wgpu_start
        wgpu_start.before_frame(view, win)
        from views import chrome_sheet
        chrome_sheet.before_frame(view, win)
    cpu, gpu = renderer.render()
    after_frame(view)
    return cpu, gpu, 0.0


def _sync_units(view, history):
    revision = (history, history._native.units_revision)
    if getattr(view, "_ui_units_sent", None) != revision:
        from core import units
        unit = units.model_units_of(history.scene)["length"]
        view.renderer.ui_set_units(units.length_suffix(unit).strip() or unit)
        view._ui_units_sent = revision


def before_frame(view) -> None:
    """Mirror the window's state into the chrome (cheap: a few dict reads)."""
    if not getattr(view, "_ui_attached", False):
        return
    win = _window(view)
    if win is None:
        return
    from core import plugin_panels
    plugin_panels.sync(view, win)
    _sheet_and_jobs(view, win)
    _chrome_form(view, win)
    from views import wgpu_start
    wgpu_start.before_frame(view, win)
    from views import chrome_sheet
    chrome_sheet.before_frame(view, win)
    if not view._ui_tools_sent and win._tool_actions:
        _send_tools(view, win)
    eyedropper = getattr(win, "_act_eyedropper", None)
    active = getattr(view.source_viewport, "nav_mode", None) \
        or ("eyedropper" if eyedropper is not None and eyedropper.isChecked() else "") \
        or next((k for k, a in win._tool_actions.items() if a.isChecked()), "")
    view.renderer.ui_set_active(active)
    bar = win.statusBar()
    base = (getattr(bar, "_base", "") or bar.currentMessage() or "") if bar is not None else ""
    name, _, hint = base.partition(" — ")
    timer = getattr(bar, "_timer", None)
    if timer is not None and timer.isActive():
        hint = getattr(bar, "_msg", None).text() if getattr(bar, "_msg", None) is not None else hint
    view.renderer.ui_set_status(name, hint)
    _sync_units(view, win.viewport.history)
    _cursor_value(view, win)
    _selection(view, win, active)
    _cluster(view, win)
    _form(view, win)
    from views import wgpu_deliveries
    wgpu_deliveries.bind(win)
    _tree(view, win)
    from views import wgpu_library
    wgpu_library.before_frame(view, win)
    _inspector(view, win)
    from views import inspector_bridge
    inspector_bridge.sync(view, win)
    _tabs(view, win)
    _projects(view, win)
    from views import wgpu_tabs
    wgpu_tabs.sync_host_context(view, win)
    wgpu_tabs.before_frame(view, win)
    opts = [(i, label, k, state) for i, label, k, state, _ in _options(view)]
    if opts != getattr(view, "_ui_opts", None):
        view._ui_opts = opts
        view.renderer.ui_set_options(opts)


_CTRL = None


def _ctrl() -> str:
    """The native name of the key the tools call Ctrl (⌘ on the Mac)."""
    global _CTRL
    if _CTRL is None:
        _CTRL = QKeySequence(Qt.CTRL | Qt.Key_A).toString(QKeySequence.NativeText)[:-1].strip("+") or "Ctrl"
    return _CTRL


def _options(view):
    """(id, label, key, state, toggle) for the active tool's strip. Each
    toggle does exactly what tapping the key does; locks are read-only."""
    tool = getattr(view.source_viewport, "active_tool", None)
    host = view.tools  # what the tools are handed as their viewport here
    name = type(tool).__name__ if tool is not None else ""
    out = []
    if name == "MoveTool" and hasattr(tool, "_toggle_copy"):
        out.append(("copy", "Copy", _ctrl(), int(bool(tool._copy)),
                    lambda: tool._toggle_copy(host)))
    elif name == "PushPullTool" and hasattr(tool, "_keep_mode"):
        def keep():
            tool._keep_mode = not tool._keep_mode
            tool._keep_base = tool._keep_mode
            tool.refresh_hint(host)
            host.update()
        out.append(("keep", "Keep base", _ctrl(), int(bool(tool._keep_mode)), keep))
    elif name == "RectangleTool" and hasattr(tool, "_set_from_center"):
        out.append(("centre", "From centre", _ctrl(), int(bool(tool._from_center)),
                    lambda: tool._set_from_center(host, not tool._from_center)))
    elif name == "SelectTool":
        selected = getattr(getattr(view.source_viewport, "scene", None), "selection", ())
        if len(selected) == 1:
            group = next(iter(selected))
            if getattr(group, "text3d", None) and not getattr(group, "locked", False):
                from core import links
                if links.linked_ancestor(group) is None:
                    out.append(("edittext3d", "Edit 3D Text…", "", 0, lambda: _window(view)._on_edit_3d_text(group)))
    elif name == "PlaceGroupTool" and getattr(tool._group, "text3d", None):
        from views.font_picker import text3d
        win = _window(view)
        out.append(("textfont", "Text and font…", "", 0,
                    lambda: text3d(win, tool._group.text3d, win._place_text3d)))
    elif name in ("DimensionTool", "TextTool"):
        from views.font_picker import annotation
        out.append(("font", "Font…", "", 0, lambda: annotation(_window(view))))
    if name == "DimensionTool":
        out.append(("dimstyle", "Dimension style…", "", 0, lambda: _open_form(view, "dimstyle")))
    vp = view.source_viewport
    lock = getattr(vp, "axis_lock", None) if vp is not None else None
    if lock:
        out.append(("axis", f"{lock.upper()} axis locked", "", 2, None))
    if vp is not None and (getattr(vp, "_previous_views", None) or vp.__dict__.get("_isolate_base")):
        out.append(("showall", "Isolated · Show all", "", 1, lambda: _show_all(vp)))
    mode = getattr(vp, "linear_inference_mode", "all") if vp is not None else "all"
    if mode == "off":
        out.append(("inf", "Inferences off", "", 2, None))
    elif mode == "parallel_perp":
        out.append(("inf", "Parallel / perpendicular only", "", 2, None))
    return out


def _cluster(view, win) -> None:
    from views.view_commands import sync
    sync(view, win)


def _cursor_value(view, win) -> None:
    """The value field on the geometry (Maxence 2026-11-22), drawn and kept
    by the chrome (ui/value_field.rs): the tool's caption, the value where the
    tool shows it (``_length_label_at``), the tool's start on screen and the
    document revision, from which Rust reads a commit and its readback.
    The same in both hosts: it reads the tool, not a status bar."""
    from core.i18n import tr
    from views.document_interaction import DocumentInteraction
    tools = getattr(view, "tools", None)
    tool = getattr(tools, "active_tool", None)
    caption = getattr(tool, "vcb_caption", None)
    caption = caption() if callable(caption) else getattr(tool, "vcb_label", None)
    label = DocumentInteraction._length_label_at(tools) if caption is not None else None
    at = view._world_to_pixel(label[1]) if label is not None else None
    history = getattr(getattr(win, "viewport", None), "history", None)
    revision = int(getattr(history, "revision", 0) or 0)
    if tool is None and getattr(view.source_viewport, "nav_mode", None) == "zoom":
        _lens_value(view, tools, revision)
        return
    if at is None:
        view.renderer.ui_set_cursor_value("", "", False, 0.0, 0.0, None, revision)
        return
    start = getattr(tool, "start_point", None)
    start = view._world_to_pixel(start) if start is not None else None
    xy = lambda p: (float(p.x()), float(p.y())) if hasattr(p, "x") else (float(p[0]), float(p[1]))  # noqa: E731
    x, y = xy(at)
    view.renderer.ui_set_cursor_value(tr(caption), label[0], label[2], x, y - 22.0,
                                      xy(start) if start is not None else None, revision)


def _lens_value(view, tools, revision) -> None:
    """Zoom takes the lens typed in the value field (35mm, 60deg; viewport
    ``_handle_zoom_value_key``): the same field, by the cursor, reading the
    focal length the view has now. The lens is what the value acts on, so it
    is part of the revision Rust reads a commit from."""
    from core.i18n import tr
    pos = getattr(view, "_cursor_pos", None)
    cam = view.source_viewport.camera
    if pos is None:
        view.renderer.ui_set_cursor_value("", "", False, 0.0, 0.0, None, revision)
        return
    buffer = str(getattr(tools, "value_buffer", "") or "")
    text = buffer or (f"{cam.focal_mm:.0f} mm" if cam.perspective else "—")
    lens = (revision << 20) ^ int(round(cam.fov_deg * 1000))
    view.renderer.ui_set_cursor_value(tr("Field of view"), text, bool(buffer),
                                      float(pos.x()), float(pos.y()) - 40.0, None, lens)


def _summary(sel) -> str:
    from core.group import Group
    groups = sum(1 for e in sel if isinstance(e, Group))
    rest = len(sel) - groups
    parts = []
    if groups:
        parts.append(f"{groups} group" + ("s" if groups != 1 else ""))
    if rest:
        parts.append(f"{rest} item" + ("s" if rest != 1 else ""))
    return " · ".join(parts)


def _selection(view, win, active) -> None:
    """The selection bar: with Select active, something selected, no drag."""
    scene = getattr(view, "scene", None)
    sel = list(getattr(scene, "selection", ()) or ())
    dragging = getattr(view, "last", None) is not None
    if active != "select" or not sel or dragging:
        if getattr(view, "_ui_sel_sig", None) is not None:
            view.renderer.ui_set_selection("", [])
            view._ui_sel_sig = None
        return
    sig = (id(scene), len(sel), id(sel[0]), id(sel[-1]))
    if sig == getattr(view, "_ui_sel_sig", None):
        return
    view._ui_sel_sig = sig
    view.renderer.ui_set_selection(_summary(sel), [(k, label, key, enabled, more)
                                                   for k, label, key, run, enabled, more in _commands(win, sel)])


def _edit_actions(win) -> dict:
    """The Edit menu's actions by label (submenus included), cached."""
    cached = getattr(win, "_ui_edit_actions", None)
    if cached is not None:
        return cached
    out = {}

    def walk(menu, prefix=""):
        for a in menu.actions():
            if a.isSeparator():
                continue
            if a.menu():
                walk(a.menu(), a.text().replace("&", "") + " ")
            elif a.text():
                out[prefix + a.text().replace("&", "")] = a
    for a in win.menuBar().actions():
        if a.menu() and a.text().replace("&", "") == "Edit":
            walk(a.menu())
    win._ui_edit_actions = out
    return out


#: Geometry fixes, under More ▾: real work, not every-selection work.
MORE = ["Intersect Faces With Model", "Intersect Faces With Selection", "Intersect Faces With Context",
        "Soften / Smooth Edges…", "Reverse Faces", "Orient Faces", "Heal Overlapping Faces",
        "Join Segments", "Rebuild Faces (Planar)", "Split into Pieces", "Simplify Mesh…",
        "Convert Path to Geometry", "Delete Guides"]


#: The More ▾ labels' verb ids (views/verbs.py).
MORE_VERBS = {"Intersect Faces With Model": "intersect", "Intersect Faces With Selection": "intersect",
              "Intersect Faces With Context": "intersect", "Soften / Smooth Edges…": "soften",
              "Reverse Faces": "reverse", "Orient Faces": "orient", "Heal Overlapping Faces": "heal",
              "Join Segments": "join", "Rebuild Faces (Planar)": "rebuild", "Split into Pieces": "split",
              "Simplify Mesh…": "simplify", "Convert Path to Geometry": "topath", "Delete Guides": "guides"}


def _commands(win, sel):
    """(id, label, shortcut, run, enabled, in More) for the selection bar.
    Fixed places (council 2026-11-22): File to…, Isolate, Hide, then what
    the selection can become; geometry fixes under More. Only the verbs
    that apply to the selection's kind (views/verbs.py, tree C): a linked
    file's placement or part gets no Explode, Edit or File to; raw
    geometry no Component."""
    from core.group import Group
    from views import verbs
    groups = [e for e in sel if isinstance(e, Group)]
    edit = _edit_actions(win)
    kind = verbs.kind_of(getattr(getattr(win, "viewport", None), "scene", None), sel)

    def act(label, vid, name=None, more=False):
        a = edit.get(label)
        if a is None or not verbs.offers(kind, vid):
            return None
        return ("e:" + label, name or label.rstrip("…"), a.shortcut().toString(QKeySequence.NativeText),
                a.trigger, a.isEnabled(), more)
    out = [
        ("file", "File to…", "⌘⇧F", (lambda: _file_to(win, len(groups))) if groups else None, bool(groups), False)
        if verbs.offers(kind, "fileto") else None,
        ("isolate", "Isolate", "", lambda: _isolate(win, [e for e in sel if getattr(e, "uid", None)]), True, False)
        if verbs.offers(kind, "isolate") else None,
        act("Hide", "hide"),
    ]
    if len(groups) == 1 and len(sel) == 1:
        one = groups[0]
        if verbs.offers(kind, "open"):
            out.append(("edit", "Edit", "double-click", lambda: win.viewport.begin_group_edit(one), True, False))
        out.append(act("Explode Group", "explode", "Explode"))
    else:
        out.append(act("Make Group", "makegroup", "Group"))
    out.append(act("Make Component…", "makecomp", "Component"))
    for label in MORE:
        name = label.replace("Intersect Faces ", "Intersect Faces ▸ ") if label.startswith("Intersect Faces ") else None
        out.append(act(label, MORE_VERBS.get(label, ""), name, more=True))
    return [c for c in out if c is not None]


def _show_all(vp) -> None:
    """«Isolated · Show all»: back to exactly what was shown before the
    first Isolate (a saved view's filter included), and the selection
    Isolate dropped, when nothing changed in between."""
    from copy import deepcopy
    from core.agent_facets import previous_display, set_display
    base = vp.__dict__.pop("_isolate_base", None)
    if base is not None:
        (expr, color, isolate, key, exact), previous, selection, cv = base
        set_display(vp, deepcopy(expr), color, isolate=isolate, key=key, remember=False, exact=exact)
        vp._previous_views = previous
        scene = vp.scene
        if cv == scene.content_version:
            scene.select(e for e in selection if scene.entity_selectable(e))
        getattr(vp, "notify_scene_changed", vp.update)()
    elif getattr(vp, "_previous_views", []):
        previous_display(vp)
    else:
        set_display(vp, remember=False)


def _isolate(win, uids) -> None:
    """Show only the placements ``uids`` (at any depth: a part of a linked
    file shows through the file's placement) until «Show all»."""
    from copy import deepcopy
    from core.agent_facets import set_display
    vp = win.viewport
    if any(not isinstance(u, str) for u in uids):
        # A placement's key is its UID, or its address when the UID repeats
        # (core.facets): from the native tree, not the facet index.
        from core.link_structure import row_id
        repeated = vp.scene.repeated_uids()
        uids = [u if isinstance(u, str) else row_id(vp.scene.occurrence_path(u) or [u.uid])
                if u.uid in repeated or u.uid.startswith("@[") else u.uid for u in uids]
    uids = [u for u in dict.fromkeys(uids) if u]
    if not uids:
        vp.flash_status("Nothing to isolate: select objects first")
        return
    if vp.__dict__.get("_isolate_base") is None:
        state = (deepcopy(getattr(vp, "_view_expression", None)), getattr(vp, "_view_color", None),
                 getattr(vp, "_view_isolate", True), getattr(vp, "active_view_key", None),
                 getattr(vp, "_view_exact", False))
        vp._isolate_base = (state, list(getattr(vp, "_previous_views", []) or []),
                            list(vp.scene.selection), vp.scene.content_version)
    set_display(vp, {"facet": "occurrence", "in": uids}, isolate=True, exact=False)


# ---- Right column: project tree and inspector -------------------------------

#: Plain-language kinds for the IFC classes a node promises (provisional:
#: the kinds catalogue in docs/project-structure.md replaces this).
KINDS = {
    None: "Drawing only", "IfcSlab": "Floor / slab", "IfcWall": "Wall", "IfcWallStandardCase": "Wall",
    "IfcBeam": "Beam", "IfcColumn": "Column", "IfcMember": "Member", "IfcPlate": "Plate",
    "IfcRailing": "Railing", "IfcStair": "Stair", "IfcStairFlight": "Stair flight", "IfcRamp": "Ramp",
    "IfcRoof": "Roof", "IfcDoor": "Door", "IfcWindow": "Window", "IfcCovering": "Covering",
    "IfcFooting": "Footing", "IfcPile": "Pile", "IfcCurtainWall": "Curtain wall",
    "IfcFurnishingElement": "Furniture", "IfcFurniture": "Furniture", "IfcSpace": "Space",
    "IfcBuildingElementProxy": "Generic object", "IfcDistributionElement": "Services",
    "IfcFlowSegment": "Pipe / duct", "IfcFlowFitting": "Fitting", "IfcFlowTerminal": "Terminal",
    "IfcMechanicalFastener": "Fastener", "IfcDiscreteAccessory": "Accessory",
    "IfcBuildingStorey": "Level", "IfcBuilding": "Building", "IfcSite": "Site",
}


def kind_of(ifc_class) -> str:
    if ifc_class in KINDS:
        return KINDS[ifc_class]
    return str(ifc_class).removeprefix("Ifc") if ifc_class else "Drawing only"


SEP = "\x1f"
#: Item rows shown per node, definition or item before a "Show more" row;
#: each click on it shows this many more (ui/tree-links: the old cap had
#: no way past it but the filter).
ROW_CAP = 200
#: Between a row's kind and its tooltip (ui.rs splits the kind on it).
TIP = "\x1e"
TIPS = {
    "Reference": "Reference — a linked file. Its contents are read from {path} and are never edited "
                 "in this document: open the source to change them. Eye and lock here are this "
                 "document's own overrides of the file's tags; the file itself is not touched.",
    "Drawing only": "Drawing only — this node has no IFC class: its items are drawn, measured and "
                    "printed, but not exported as building elements. Give it a kind to export them.",
    "Linked files": "The files this document references. Each one shows its own structure, read-only.",
}


def _tree_model(view, scene, cv):
    """The tree's nodes and memberships (core/link_structure.py), kept
    until the model changes: a click that only changes the selection
    rebuilds the rows from it without walking every placement again."""
    cached = view.__dict__.get("_ui_tree_model")
    key = (id(scene), cv)
    if cached is not None and cached[0] == key:
        return cached[1]
    model = prepare_tree_model(scene)
    view._ui_tree_model = (key, model)
    return model


def prepare_tree_model(scene):
    from collections import Counter, defaultdict
    from core import bim_registry, link_structure, structure
    from ingetrazo_core import source_direct_count
    registry = bim_registry.get(scene)
    tree = link_structure.Tree(scene, registry)
    recipes = {k: n["recipe"] for k, n in tree.nodes.items()}
    counts, problems, to_fix = Counter(), Counter(), []
    node_layers = defaultdict(Counter)
    unfiled = set(map(id, tree.unfiled))
    for k, gs in tree.direct.items():
        host = tree.nodes.get(k, {}).get("kind") == "host"
        for g in gs:
            counts[k] += 1
            if getattr(g, "layer", None):
                node_layers[k][g.layer] += 1
            if host:
                bad = int(recipes[k].get("class") is not None and not structure._has_status(g.ifc or {})) \
                    + int(id(g) in unfiled)
                problems[k] += bad
                if bad:
                    to_fix.append(g)
    order = tree.order()
    for k, _d in reversed(order):
        parent = tree.nodes[k]["parent"]
        counts[parent] += counts[k]
        problems[parent] += problems[k]
    for k, copies in tree.links.items():
        if not counts[k]:
            counts[k] = source_direct_count(copies)
    return dict(tree=tree, registry=registry, recipes=recipes, counts=counts, problems=problems,
                 to_fix=to_fix, node_layers=node_layers, order=order)


def open_line_type(win, name, tags):
    from views.chrome_dialogs import view_of
    view = view_of(win)
    if not getattr(view, "_ui_attached", False):
        win.viewport.flash_status("Line type editing requires the native Drawing lens.")
        return False
    view._ui_lens, view._ui_mode, view._ui_tree_sig = 1, "", None
    apply_dock_width(view)
    _tree(view, win)
    opened = view.renderer.ui_open_line_type(name, list(tags))
    view.update()
    return opened


def _tree(view, win, *, discover=False):
    """Bind the membership index, revision and disclosure state to the native sidebar providers."""
    scene = getattr(win.viewport, "scene", None)
    if scene is None:
        return
    exp = view.__dict__.setdefault("_ui_exp", set())
    pages = view.__dict__.setdefault("_ui_pages", {})
    flt = view.__dict__.get("_ui_filter", "")
    if discover:
        flt = ""
    lens = 0 if discover else view.__dict__.get("_ui_lens", 0)
    sel_ids = {id(e) for e in scene.selection}
    from ingetrazo_core import ProjectSheets
    catalog = view.__dict__.get("_ui_sheet_catalog")
    if catalog is None:
        catalog = view._ui_sheet_catalog = ProjectSheets()
    sheet_revision = view._ui_sheet_revision = catalog.refresh(scene)
    # Watertight checks in Export must follow mesh edits.
    geometric = lens == 2
    cv = scene.content_version if geometric else _tree_version(view, win, scene)
    sig = (id(scene), id(scene.edit_group), scene.mesh.revision, cv, tuple(sorted(exp)), tuple(sorted(pages.items())), flt, len(sel_ids),
           hash(frozenset(sel_ids)), lens, getattr(view, "_ui_layer_rev", 0), scene.show_section_planes, sheet_revision)
    if not discover and sig == getattr(view, "_ui_tree_sig", None):
        return
    first = not hasattr(view, "_ui_tree")
    if not discover:
        view._ui_tree_sig = sig
    mode = "" if discover else view.__dict__.get("_ui_mode", "")
    from core import structure
    from core import layers as L
    from ingetrazo_core import filing_rows, mark_filing_selection, project_sidebar
    model = _tree_model(view, scene, cv)
    tree, registry, recipes = model["tree"], model["registry"], model["recipes"]
    counts, to_fix = model["counts"], model["to_fix"]
    node_layers, order = model["node_layers"], model["order"]
    if discover:
        from ingetrazo_core import reveal_branches
        exp = reveal_branches(scene, tree)
    nodes, children, direct, unfiled = tree.nodes, tree.children, tree.direct, tree.unfiled
    node_of = tree.member
    active = registry.get("structure", {}).get("active")
    if first:
        # The host's own top nodes open; files stay folded (each is a root).
        exp.update(f"n:{k}" for k, _ in order if not nodes[k]["parent"] and nodes[k]["kind"] == "host")
        k = active
        while k in nodes and nodes[k]["parent"]:
            k = nodes[k]["parent"]
            exp.add(f"n:{k}")
    rows = filing_rows(scene, model, exp, pages, flt, discover, ROW_CAP, lens, sel_ids,
                       active, KINDS, TIPS, FILE_STATES)
    rows, path_bar = project_sidebar(scene, tree, rows, flt, exp, str(getattr(win, "_current_path", "") or ""))
    rows.extend(catalog.rows(flt, exp))
    if discover:
        from ingetrazo_core import reveal_selection
        return reveal_selection(rows)
    if lens == 2:
        from core import deliveries
        from ingetrazo_core import export_readiness
        records = deliveries.records(scene)
        delivery = records.get(getattr(view, "_delivery_key", None)) or next(iter(records.values()), {})
        profile = delivery.get("options", {}).get("profile", "") if delivery else None
        cache_key = (id(scene), scene.content_version, profile)
        cache = getattr(view, "_ui_rules_cache", None)
        if not cache or cache[0] != cache_key:
            cache = cache_key, _problem_rules(scene, registry, structure._nodes(registry), recipes, unfiled, profile)
            view._ui_rules_cache = cache
        rows, readiness = export_readiness(scene, tree, records, getattr(view, "_delivery_key", "") or "",
                                           cache[1], exp, rows)
        view.renderer.ui_panel_set("readiness", readiness)
    rows = mark_filing_selection(rows)
    view._ui_tree = dict(direct=direct, children=children, unfiled=unfiled, recipes=recipes, to_fix=to_fix,
                         active=active, node_of=node_of, nodes=nodes, tree=tree, counts=counts,
                         node_layers={k: list(c) for k, c in node_layers.items()})
    view.renderer.ui_set_tree(rows, len(unfiled))
    view.renderer.ui_set_catalog_info(Path(str(getattr(win, "_current_path", "") or "")).name, tree, counts)
    view.renderer.ui_set_edit_path(path_bar)
    if hasattr(view.renderer, "ui_set_proposal_available"):
        view.renderer.ui_set_proposal_available(not mode and tree._host_roots < 0)
    if not view.__dict__.get("_ui_sort_loaded") and hasattr(view.renderer, "ui_set_tree_sort"):
        from core.settings import settings as QSettings
        view.renderer.ui_set_tree_sort(max(0, QSettings().value("ui/project_tree_sort", 0, type=int)))
        view._ui_sort_loaded = True
    view.renderer.ui_set_style_catalog(id(scene), [*L.LINETYPES, *(L.CUSTOM_LINETYPE + key for key in scene.line_types)], scene.hatch_patterns, scene if lens == 1 else None)
    view.renderer.ui_set_lens(lens)
    view.renderer.ui_set_mode({"library": "Library"}.get(mode, ""))
    view.renderer.ui_set_issues(bool(view.__dict__.get("_ui_issues", False)))
    if not getattr(view, "_ui_kinds_sent", False):
        view.renderer.ui_set_kinds(sorted(set(KINDS.values()) - {"Level", "Building", "Site"}))
        view._ui_kinds_sent = True
    path = " › ".join(structure.path(registry, active)) if active in structure._nodes(registry) else ""
    view.renderer.ui_set_delivery(path, len(unfiled), len(to_fix))


#: A file row's state, in words (its dot's tooltip and the inspector).
FILE_STATES = {"resolved": "Up to date", "modified": "Changed on disk — reload to see the changes",
               "missing": "Not found — relink it", "packed": "Packed in this file"}


#: Steps that change only geometry inside meshes (the feed's own
#: geometry-only set, and paint): the tree has nothing to show for them.
GEOMETRY_STEPS = {"SnapshotMutation", "SnapshotCompound", "MoveVerticesCommand", "AddEdgeCommand",
                  "AddFaceCommand", "EraseSelectionCommand", "DeleteEdgesCommand", "DeleteFaceCommand"}


def _geometry_step(step) -> bool:
    return step.complete and (step.command in GEOMETRY_STEPS or step.command.startswith("SetFace")) \
        and not any(step.placements.values()) and not any(step.tags.values()) \
        and not any(step.materials.values())


def _tree_version(view, win, scene) -> int:
    """The content version the tree follows: it moves on with the model,
    except across History steps the change feed shows to be geometry only
    (a line drawn, a face pushed), which leave every row as it was — on the
    Ride file a rebuild costs 80–190 ms per edit."""
    feed = getattr(getattr(win.viewport, "history", None), "feed", None)
    cv = scene.content_version
    last = view.__dict__.get("_ui_tree_cv")
    if feed is None or last is None or last[0] != id(scene):
        view._ui_tree_cv = (id(scene), cv, getattr(feed, "revision", 0), cv)
        return cv
    _sid, seen_cv, seen_rev, shown = last
    if cv == seen_cv:
        return shown
    rev = feed.revision
    if rev > seen_rev:
        got = feed.since(seen_rev)
        if got["complete"] and got["steps"] and all(_geometry_step(st) for st in got["steps"]) \
                and len(got["steps"]) == rev - seen_rev:
            # Only geometry since the rows were built: keep them.
            view._ui_tree_cv = (id(scene), cv, rev, shown)
            return shown
    view._ui_tree_cv = (id(scene), cv, rev, cv)
    return cv


def _problem_rules(scene, registry, nodes, recipes, unfiled, profile=None):
    from core import structure
    from core.structure_ids import load_requirements, requirement_hits
    rules = []          # (severity, rule id, label, [groups])
    rules.append(("Blocks export", "unfiled", "Not filed", list(unfiled)))
    try:
        profiles = registry.get("export_profiles", {})
        checked = registry if profile is None else {**registry, "export_profiles": {profile: profiles.get(profile, {})}}
        requirements = load_requirements(scene, checked)
    except Exception:
        requirements = []
    ids = {}
    missing, hidden, unclassified = [], [], []
    for g, src, root, key in structure._memberships(scene, registry):
        if key not in nodes or src:
            continue
        rec = g.ifc or {}
        if recipes[key].get("class") is not None:
            if not rec.get("class"):
                unclassified.append(g)
            if not structure._has_status(rec):
                missing.append(g)
            if getattr(g, "hidden", False):
                hidden.append(g)
        if requirements and rec:
            for hit in requirement_hits(g, registry, requirements):
                ids.setdefault(hit.get("specification") or hit.get("file") or "IDS", []).append(g)
    rules.append(("Needs attention", "class", "Missing IFC class", unclassified))
    for spec, gs in sorted(ids.items()):
        rules.append(("Blocks export", "ids:" + spec, f"IDS: {spec}", gs))
    from core import links, link_structure
    stale, states = [], {}
    for g in links.linked_groups(scene):
        rec = links.link_of(g)
        path = rec["path"]
        if path not in states:
            states[path] = link_structure.file_state(scene, path)
        if states[path] in ("missing", "modified"):
            stale.append(g)
    rules.append(("Needs attention", "links", "Linked file changed or missing", stale))
    from ingetrazo_core import solid_problem_targets
    leaky, checked, total = solid_problem_targets(scene)
    rules.append(("Needs attention", "solid", "Solid geometry problems", leaky))
    if checked < total:
        rules.append(("Info", "solidcut", f"Watertight: checked {checked} of {total} meshes", []))
    rules.append(("Needs attention", "status", "Status missing", missing))
    rules.append(("Info", "hidden", "Hidden items in exported nodes", hidden))
    if not requirements:
        rules.append(("Info", "noids", "No IDS checklist in the selected profile", []))
    return rules


def _tex_icon(view, path) -> str:
    """A texture's thumbnail as a chrome icon (loaded once)."""
    key = "tex:" + str(path)
    sent = view.__dict__.setdefault("_ui_tex_icons", set())
    if key not in sent:
        sent.add(key)
        img = QImage(str(path))
        if not img.isNull():
            img = img.scaled(48, 48, Qt.IgnoreAspectRatio, Qt.SmoothTransformation).convertToFormat(QImage.Format_RGBA8888)
            view.renderer.ui_set_icons([(key, 48, 48, bytes(img.constBits())[: 48 * 48 * 4])])
    return key


def _hex(rgb) -> str:
    return "#%02x%02x%02x" % tuple(int(max(0.0, min(1.0, float(c))) * 255) for c in rgb[:3]) if rgb else ""


def _current_material_name():
    from tools.paint import PaintTool
    mat = getattr(PaintTool, "current_material", None)
    return getattr(mat, "name", None)


def _active_tool_id(win) -> str:
    """The checked tool of the window ("paint", "select", …) or ""."""
    return next((k for k, a in getattr(win, "_tool_actions", {}).items() if a.isChecked()), "")


def _palette(view, win, active) -> None:
    """With Paint active: the model's named materials as a strip."""
    items = []
    if active == "paint":
        scene = win.viewport.scene
        cur = _current_material_name()
        for name, mat in list((getattr(scene, "materials", None) or {}).items())[:32]:
            tex = getattr(mat, "texture", None)
            icon = _tex_icon(view, tex["path"]) if tex and tex.get("path") else ""
            items.append((f"m:{name}", name, _hex(mat.color), icon, name == cur))
    if items != getattr(view, "_ui_palette", None):
        view._ui_palette = items
        view.renderer.ui_set_palette(items)


def _components_dir():
    from core.paths import app_root
    return app_root() / "resources" / "components"


def _starter_components():
    """The bundled set: ``sc:<key>`` 3D starters, ``pp:<key>`` 2D people."""
    import json
    res = _components_dir()
    out = {}
    if (res / "components.json").exists():
        for e in json.loads((res / "components.json").read_text(encoding="utf-8")):
            out["sc:" + e["key"]] = e
    if (res / "people.json").exists():
        for e in json.loads((res / "people.json").read_text(encoding="utf-8")):
            out["pp:" + e["key"]] = e
    return out


def _components_in_model(view, win):
    """``[(name, faces, groups, kept template)]`` from the Components tray's
    own listing, kept until the model changes."""
    scene = win.viewport.scene
    sig = (id(scene), scene.content_version)
    cache = getattr(view, "_ui_comp_cache", None)
    if cache is not None and cache[0] == sig:
        return cache[1]
    panel = getattr(getattr(win, "tray", None), "components", None)
    rows = []
    if panel is not None:
        gen = panel._components_in_model()
        try:
            while True:
                next(gen)
        except StopIteration as stop:
            rows = stop.value or []
    view._ui_comp_cache = (sig, rows)
    return rows


def _library_pick(win, view, key) -> None:
    """A Library row: paint with a material, place a component."""
    from core.i18n import tr
    kind, _, rest = key.partition(":")
    if kind == "sc":
        e = _starter_components().get(key)
        if e:
            win._on_insert_component(e["key"], tr(e["name"]))
    elif kind == "pp":
        e = _starter_components().get(key)
        if e:
            win._on_insert_person_2d(f"{e['key']}.png", e["height"], e["name"])
    elif kind == "cm":
        _component_verb(win, view, "cplace", key)
    else:
        _paint_with(win, key)


def _component_verb(win, view, verb, key) -> None:
    from core.group import Group
    from core.kept_components import make_template, place_copy
    comps = (getattr(view, "_ui_comp_cache", None) or (None, []))[1]
    try:
        _name, _f, groups, kept = comps[int(key.partition(":")[2])]
    except (ValueError, IndexError):
        return
    definition = kept if kept is not None else (groups[0] if groups else None)
    if definition is None:
        return
    if verb == "cplace":
        win._start_place(place_copy(kept if kept is not None else make_template(definition)))
    elif verb == "cselect":
        _select(win, groups)
    elif verb == "creplace":
        swap = [g for g in win.viewport.scene.selection
                if isinstance(g, Group) and g.is_component() and g.mesh is not definition.mesh]
        if swap:
            win.replace_selection_with(definition)
        else:
            win.viewport.flash_status("Select copies of another component first")
    elif verb == "ckeep":
        win.keep_component(definition, kept is None)
    elif verb == "crename":
        win.rename_component(definition)


def _paint_with(win, key) -> None:
    """Apply a material from the strip or the Library: Paint becomes active."""
    from views.wgpu_library import _catalogue, _pick_paint
    kind, _, rest = key.partition(":")
    if kind == "m":
        _pick_paint(win, name=rest)
    elif item := _catalogue().get(key):
        _pick_paint(win, recipe=item["recipe"])


UNIT_TEXT = {"m2": "m²", "m3": "m³", "m": "m", "und": "u", "kg": "kg"}


def _measure(view, win, auto=False) -> None:
    """Quantity takeoff (the BIM panel's metrado), per item. The first
    measure walks every tagged object (7.5 s on the 99k-object Ride file),
    so it runs when asked; after it the memo keeps it current for the
    cost of what changed (0.2 s there)."""
    import time
    from core import bim
    scene = win.viewport.scene
    q = view.__dict__.get("_ui_qty")
    if q is None or q["scene"] != id(scene):
        if auto:
            return
        q = view._ui_qty = {"scene": id(scene), "memo": {}, "sig": None, "by_group": {}}
    if q["sig"] == scene.content_version:
        return
    from PySide6.QtCore import Qt
    from PySide6.QtWidgets import QApplication
    t0 = time.perf_counter()
    QApplication.setOverrideCursor(Qt.WaitCursor)
    try:
        objs = bim.collect_objects(scene, q["memo"])
        by_group = {}
        for obj in objs:
            owner = obj.get("group") or obj.get("owner")
            if owner is None:
                continue
            if "_q" not in obj:
                try:
                    obj["_q"] = bim.object_quantities(obj)[2]
                except Exception:
                    obj["_q"] = (None, "")
            value, unit = obj["_q"]
            if unit == "und":
                value = 1.0
            if value is None or not unit:
                continue
            per = by_group.setdefault(id(owner), {})
            per[unit] = per.get(unit, 0.0) + value
        q["by_group"] = by_group
        q["sig"] = scene.content_version
        q["count"] = len(objs)
    finally:
        QApplication.restoreOverrideCursor()
    if not auto:
        win.viewport.flash_status(f"Measured {len(objs)} items in {time.perf_counter() - t0:.1f} s")
    view._ui_tree_sig = None
    view._ui_insp_sig = None


def _group_qty(view, g) -> dict:
    q = view.__dict__.get("_ui_qty")
    return q["by_group"].get(id(g), {}) if q else {}


def _qty_text(per_unit) -> str:
    """The main measure of a row: its largest unit's total, «+» if mixed."""
    if not per_unit:
        return ""
    order = ["m3", "m2", "m", "kg", "und"]
    unit = min(per_unit, key=lambda u: order.index(u) if u in order else 9)
    v = per_unit[unit]
    text = (f"{v:.0f}" if unit == "und" or v >= 100 else f"{v:.2f}") + " " + UNIT_TEXT.get(unit, unit)
    return text + (" +" if len(per_unit) > 1 else "")


def _qty_rows(per_unit) -> list:
    return [(UNIT_TEXT.get(u, u), f"{v:.0f}" if u == "und" else f"{v:.3f}") for u, v in sorted(per_unit.items())]


def _export_csv(win) -> None:
    from PySide6.QtCore import Qt
    from PySide6.QtWidgets import QApplication
    from core.bim import quantities_csv
    from views.filedialogs import file_dialogs
    path, _ = file_dialogs.getSaveFileName(win, "Export quantities CSV", "metrado.csv", "CSV (*.csv);;All files (*)")
    if not path:
        return
    QApplication.setOverrideCursor(Qt.WaitCursor)
    try:
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(quantities_csv(win.viewport.scene))
    finally:
        QApplication.restoreOverrideCursor()
    win.viewport.flash_status(f"Quantities exported to {path}")


def _layer(scene, tag):
    """The tag (layer) a node's recipe names: by key, else by label, else by
    the last part of a folder path."""
    if not tag:
        return None
    ly = scene.layer(tag)
    if ly is None and "/" in str(tag):
        ly = scene.layer(str(tag).rsplit("/", 1)[-1])
    return ly


def _tree_groups(view, key):
    data = getattr(view, "_ui_tree", None)
    if not data:
        return []
    out, stack = [], [key]
    while stack:
        k = stack.pop()
        out.extend(data["direct"].get(k, ()))
        stack.extend(data["children"].get(k, ()))
    return out


def _group_by_uid(win, uid):
    if uid.startswith("@["):
        import json
        return win.viewport.scene.resolve_occurrence(json.loads(uid[1:]))
    from core import structure
    for g, _src, _root in structure._placements(win.viewport.scene):
        if g.uid == uid:
            return g
    return None


def _select(win, groups) -> None:
    scene = win.viewport.scene
    scene.select(g for g in groups if scene.entity_selectable(g))
    getattr(win.viewport, "notify_scene_changed", win.viewport.update)()


def _tree_action(win, view, action) -> None:
    """One verb from the dock: ``verb SEP rowkey [SEP extra]``. Row keys:
    n:<node> (n: alone is Unfiled), i:/c:<uid> items, d:<node>:<mesh>
    copies of a definition, g:<uid> filed elsewhere."""
    from core import bim_registry, structure
    from core import layers as L
    from core.group import Group
    from core.history import CompoundCommand, MakeUniqueCommand, SetRegistryCommand
    from views.tree_navigation import execute
    if SEP not in action:
        # The inspector's chips and the delivery counts: "verb:arg".
        verb, _, key = action.partition(":")
        vp = win.viewport
        sel = list(vp.scene.selection)
        groups = [e for e in sel if isinstance(e, Group)]
        if verb == "insp" and key.startswith("delivery:"):
            _run(win, ["deliver:" + key[9:]])
        elif verb == "insp" and key.startswith("opensheetrow:"):
            _tree_action(win, view, SEP.join(("open", key.partition(":")[2])))
        elif verb == "insp" and key.startswith("sectionview:"):
            from views import view_commands
            view_commands.run(win, key.partition(":")[2])
            view._ui_tree_sig = view._ui_insp_sig = None
        elif verb == "insp" and key.startswith("section:"):
            _, uid, field, *value = key.split(":")
            _tree_action(win, view, SEP.join(("edit", f"section:{uid}:{field}", value[0] if value else "")))
        elif verb == "insp" and key.startswith("status:"):
            _set_status(win, groups, key.partition(":")[2])
        elif verb == "insp" and key == "file" and groups:
            _file_to(win, len(groups))
        elif verb == "insp" and key.startswith(("active:", "isolate:")):
            what, _, nk = key.partition(":")
            _tree_action(win, view, f"{what}{SEP}n:{nk}")
        elif verb == "insp" and key.startswith("nodestatus:"):
            _, nk, value = key.split(":", 2)
            _set_status(win, _tree_groups(view, nk), value)
        elif verb == "insp" and key.startswith("cutlist"):
            nk = key.partition(":")[2]
            if nk:
                _select(win, _tree_groups(view, nk))
            from views.report_sheets import show
            show(win, "cutlist")
        elif verb == "insp" and key.startswith("lib:"):
            from views import wgpu_library
            wgpu_library.action(win, view, key[4:])
        elif verb == "insp" and key.startswith("form:"):
            _open_form(view, key[5:])
        elif verb == "insp" and key == "measure":
            _measure(view, win)
            view._ui_tree_sig = None
        elif verb == "insp" and (key == "solidrepair" or key.startswith("solidfault:")):
            active = key.startswith("solidfault:active/") and view.__dict__.get("_ui_lens") == 2
            if not active and len(groups) != 1:
                return
            out = _core(vp, "repair_solid", placement=None if active else vp.scene.occurrence_path(groups[0]),
                        fault=key.partition(":")[2] or None)
            if out and out.get("error"):
                vp.flash_status(out["error"])
            elif out and out["ok"]:
                d = out["repaired"]
                vp.flash_status(f"Repaired: {d['stray']} stray edges, {d['duplicates']} duplicated faces, "
                                f"{d['holes']} holes closed, {d['partitions']} partitions, {d['reversed']} faces turned")
            view._ui_insp_sig = None
            vp.update()
        elif verb == "insp" and key.startswith("lnk:"):
            what, _, nk = key[4:].partition(":")
            _tree_action(win, view, f"{'eye' if what == 'eye' else what}{SEP}n:{nk}")
        elif verb == "insp" and key == "linkfile" and groups:
            from core import links as _links
            holder = _links.linked_ancestor(groups[0])
            if holder is not None:
                _select(win, [holder])
        elif verb == "insp" and key.startswith("link") and groups:
            from core import links as _links
            holder = _links.linked_ancestor(groups[0])
            _link_action(win, view, holder if holder is not None else groups[0], key)
        elif verb == "unfiled":
            _select(win, (getattr(view, "_ui_tree", None) or {}).get("unfiled", []))
        return
    parts = action.split(SEP)
    verb, rk = parts[0], (parts[1] if len(parts) > 1 else "")
    extra = parts[2] if len(parts) > 2 else ""
    vp = win.viewport
    scene = vp.scene
    exp = view.__dict__.setdefault("_ui_exp", set())
    data = getattr(view, "_ui_tree", None) or {}
    kind, _, ident = rk.partition(":")
    node = ident if kind == "n" else None

    def item():
        if kind == "exportitem":
            import json
            path = json.loads(ident)
            return scene.resolve_occurrence(path) if path else None
        return _group_by_uid(win, ident) if kind in ("i", "c", "g") else None

    def copies():
        if kind != "d":
            return []
        nkey, _, mesh = ident.rpartition(":")
        pool = data.get("direct", {}).get(nkey, []) if nkey else data.get("unfiled", [])
        return [g for g in pool if str(id(g.mesh)) == mesh]

    from core import link_structure
    tree = data.get("tree")
    linked = link_structure.is_linked_key(node)
    tnode = (data.get("nodes") or {}).get(node) or {}

    def address(group):
        return tree.address(group) if tree is not None else group.uid

    def members():
        if node == "":
            return list(data.get("unfiled", []))
        return _tree_groups(view, node) if node else copies() or ([item()] if item() else [])

    def placements():
        # A linked row's own placements: the link placements of the files
        # it stands for (one item each), never their tens of thousands of parts.
        if tree is None or not node:
            return []
        keys = [node] if tnode.get("kind") == "link" else [
            c for c in tree.children.get(node, ()) if tree.nodes[c]["kind"] == "link"]
        return [g for k in keys for g in tree.links.get(k, ())]

    def shown():
        # What Isolate, Zoom and the like act on for this row.
        if linked and tnode.get("kind") in ("link", "linked"):
            return placements()
        return members()

    def tags(key):
        # The layers the node's items (and its sub-nodes') carry, and the
        # tags its recipes name.
        out, stack = [], [key]
        nodes_ = data.get("nodes", {})
        while stack:
            k = stack.pop()
            out.extend(data.get("node_layers", {}).get(k, ()))
            n = nodes_.get(k) or {}
            if n.get("kind") == "host":
                ly = _layer(scene, n.get("recipe", {}).get("tag"))
                if ly is not None:
                    out.append(ly.key)
            elif n.get("tag"):
                out.append(n["tag"])
            stack.extend(data.get("children", {}).get(k, ()))
        return list(dict.fromkeys(out))

    def link_folders(key):
        n = data.get("nodes", {}).get(key) or {}
        if n.get("kind") == "link":
            return list(n.get("folders") or [n["folder"]])
        if n.get("kind") == "linked":
            return [data["nodes"][c]["folder"] for c in data.get("children", {}).get(key, ())
                    if data["nodes"][c].get("kind") == "link"]
        if n.get("kind") == "tagfolder":
            return [n["folder"]]
        return []

    def read_only():
        vp.flash_status("A linked file's structure is read-only here: open its source to change it")

    def rebuild():
        view._ui_tree_sig = None
        view.update()

    try:
        if verb == "hosteye":
            from views import wgpu_tabs
            host, mount = wgpu_tabs.source_context(win)
            if mount is not None:
                _core(host.viewport, "set_tag_state", tags=[rk], visible=extra == "1")
                host.viewport.notify_scene_changed()
                view.update()
        elif verb == "documentroot":
            from views.tree_navigation import open_to
            open_to(vp, [])
        elif verb in ("revealselection", "revealnode"):
            plan = _tree(view, win, discover=True) if scene.selection else None
            if plan:
                target, ancestors, limits = plan
                exp.update(ancestors)
                pages = view.__dict__.setdefault("_ui_pages", {})
                for page, limit in limits:
                    pages[page] = max(pages.get(page, ROW_CAP), limit)
                view._ui_mode = view._ui_filter = ""
                view.renderer.ui_reveal_row(rk if verb == "revealnode" else target)
                rebuild()
            elif scene.selection:
                vp.flash_status("This selection has no occurrence in the Project tree")
        elif verb == "item" and kind in ("sheet-page", "sheet-frame", "saved-scene", "sheet-pages", "saved-scenes"):
            _select(win, [])
            view._ui_insp_node = (rk, (0, 0, 0))
        elif verb == "open" and kind in ("sheet-page", "sheet-frame", "saved-scene"):
            target = view._ui_sheet_catalog.target(scene, rk)
            if target is not None:
                family, index = target
                if family == "sheet":
                    win._show_sheet(index)
                else:
                    win.tray.scenes.activate_view(scene.saved_views[index])
        elif verb == "linkselection":
            from ingetrazo_core import link_selection_sources
            from views.chrome_forms import ask_choice
            sources = link_selection_sources(win)
            if sources:
                def choose(path):
                    if path in sources and win.viewport.scene is scene:
                        win._choose_link_contents(path, select={"uids": sources[path]})
                ask_choice(win, "Link selection", "Open source document", list(sources), done=choose)
            else:
                vp.flash_status("Select groups in another saved, open document first")
        elif verb == "linkregistry":
            from core import project
            from views.filedialogs import file_dialogs
            path, _ = file_dialogs.getSaveFileName(win, "Move registry to project", "", "Project (*.igzproj)")
            if path:
                project.create(scene, path, win._current_path)
                win._relink_done("Project registry updated")
        elif verb in ("eye", "lock") and rk == "geometry:loose":
            _core(vp, "edit_loose_geometry", field=verb)
            scene.select(e for e in list(scene.selection) if scene.entity_selectable(e))
            vp.notify_scene_changed()
            rebuild()
        elif verb in ("eye", "lock") and kind == "preview-tag":
            layer = scene.layer_by_key(ident)
            if layer is not None:
                _core(vp, "set_tag_state", tags=[ident], **({"visible": not layer.visible} if verb == "eye" else {"locked": not layer.locked}))
                vp.notify_scene_changed()
                rebuild()
        elif verb == "eye" and kind == "preview-folder":
            _core(vp, "set_tag_state", folders=[ident], visible=not L.folder_visible(scene, ident, ancestors=False))
            vp.notify_scene_changed()
            rebuild()
        elif verb == "item" and rk == "geometry:loose":
            from itertools import chain
            _select(win, chain(scene.mesh.faces, scene.mesh.edges))
        elif verb == "eye" and rk == "sections:all":
            from views import view_commands
            view_commands.run(win, "section_planes")
            rebuild()
        elif verb == "eye" and kind == "section":
            _core(vp, "edit_section_plane", uid=ident, field="eye")
            vp.notify_scene_changed()
            rebuild()
        elif verb == "item" and kind in ("section", "sections"):
            _select(win, [p for p in scene.section_planes if str(p.uid) == ident])
            sel = list(scene.selection)
            view._ui_insp_node = (rk, (len(sel), id(sel[0]) if sel else 0, id(sel[-1]) if sel else 0))
        elif verb == "toggle":
            if rk.startswith(("r:", "c:inmodel", "c:compmodel")):
                # Rules start open when short: remember a collapse explicitly.
                if rk in exp:
                    exp.discard(rk)
                    exp.add(f"-{rk}")
                elif f"-{rk}" in exp:
                    exp.discard(f"-{rk}")
                    exp.add(rk)
                else:
                    exp.add(f"-{rk}")
            else:
                exp.symmetric_difference_update({rk})
            rebuild()
        elif verb in ("cplace", "cselect", "creplace", "ckeep", "crename"):
            _component_verb(win, view, verb, rk)
            rebuild()
        elif verb == "issues":
            view._ui_issues = not view.__dict__.get("_ui_issues", False)
            rebuild()
        elif verb == "mode":
            from views import wgpu_library
            wgpu_library.set_mode(view, win, rk)
        elif verb == "item" and rk == "solid:active":
            _select(win, [])
            view._ui_insp_node = (rk, (0, 0, 0))
            rebuild()
        elif verb == "filter":
            view._ui_filter = rk
            rebuild()
        elif verb == "sort":
            from core.settings import settings as QSettings
            QSettings().setValue("ui/project_tree_sort", int(rk))
        elif verb == "item" and kind == "exportitem":
            group = item()
            if group is not None:
                _select(win, [group])
                win._on_zoom_selection()
        elif verb == "exportshow":
            import json
            _select(win, filter(None, (scene.resolve_occurrence(path) for path in json.loads(rk))))
            win._on_zoom_selection()
        elif verb == "exportstatus":
            import json
            _core(vp, "patch_record_field", **json.loads(rk))
            vp.notify_scene_changed()
            rebuild()
        elif verb in ("exportpat", "exportlin"):
            from ingetrazo_core import export_style_file
            from views.filedialogs import file_dialogs
            ext = verb[6:]
            path, _ = file_dialogs.getSaveFileName(win, "Export drawing patterns", f"patterns.{ext}", f"Pattern files (*.{ext})")
            if path and win.viewport.scene is scene:
                try:
                    count = export_style_file(scene, path, ext)
                    vp.flash_status(f"{count} patterns written to {Path(path).name}")
                except (OSError, ValueError) as exc:
                    vp.flash_status(str(exc))
        elif verb in ("editline", "tagfolder"):
            from ingetrazo_core import edit_line_type, edit_tag_folder
            edit = edit_line_type if verb == "editline" else edit_tag_folder
            key = edit(vp.history, rk, extra)
            vp.notify_scene_changed()
            rebuild()
            view.renderer.ui_drawing_applied("foldernew" if verb == "tagfolder" and rk == "new" else verb, key)
        elif verb == "hatchpref":
            view.renderer.ui_hatch_preference(rk, extra)
        elif verb == "documentpiece":
            import json
            from ingetrazo_core import document_piece
            document_piece(vp.history, rk, json.loads(extra))
            view.renderer.ui_drawing_applied(verb, "")
            vp.notify_scene_changed()
            rebuild()
        elif verb == "importlin":
            from ingetrazo_core import import_line_file
            from views.filedialogs import file_dialogs
            path, _ = file_dialogs.getOpenFileName(win, "Import line types", "", "Line type files (*.lin)")
            if path and win.viewport.scene is scene:
                try:
                    count, skipped = import_line_file(vp.history, path)
                    omitted = ", ".join(skipped)
                    vp.flash_status(f"{count} line types imported" + (f". Unsupported: {omitted}" if omitted else ""))
                    vp.notify_scene_changed()
                    rebuild()
                except OSError as exc:
                    vp.flash_status(str(exc))
        elif verb == "importpat":
            from core.i18n import tr
            from ingetrazo_core import import_hatch_file
            from views.filedialogs import file_dialogs
            path, _ = file_dialogs.getOpenFileName(win, "Import hatch patterns", "", "Pattern files (*.pat)")
            if path and win.viewport.scene is scene:
                try:
                    count = import_hatch_file(vp.history, path)
                    vp.flash_status(tr("{n} hatch patterns imported from {name}", n=count, name=Path(path).name))
                    rebuild()
                    vp.notify_scene_changed()
                except (OSError, ValueError) as exc:
                    vp.flash_status(str(exc))
        elif verb == "drawingpatch":
            import json
            targets, patch = json.loads(rk)
            result = _core(vp, "edit_tag", tags=targets, field="style", value=json.dumps(patch))
            if result is None:
                return
            if result.get("rejected") or result.get("error"):
                vp.flash_status(result["rejected"] or result["error"])
                return
            view.renderer.ui_drawing_applied(verb, "")
            rebuild()
            vp.notify_scene_changed()
        elif verb == "lens":
            view._ui_lens = int(rk or 0)
            apply_dock_width(view)
            rebuild()
        elif verb in ("print", "color", "linetype", "weight", "hatch") and node and not linked:
            layer = _layer(scene, data["recipes"][node].get("tag"))
            if layer is not None:
                _core(vp, "edit_tag", tags=[layer.key], field=verb, value=extra)
                rebuild()
                vp.notify_scene_changed()
        elif verb == "measure":
            _measure(view, win)
            rebuild()
        elif verb == "exportcsv":
            _export_csv(win)
        elif verb == "expandall":
            exp.update(f"n:{k}" for k in data.get("nodes", {}))
            rebuild()
        elif verb == "collapseall":
            exp.clear()
            rebuild()
        elif verb == "loose" and kind == "m":
            # "Show more": the next page of that list.
            pages = view.__dict__.setdefault("_ui_pages", {})
            pages[ident] = pages.get(ident, ROW_CAP) + ROW_CAP
            rebuild()
        elif verb == "node" and linked:
            # A linked row selects the link placement(s) it stands for (a
            # node inside a file selects nothing): the inspector says what
            # it is. Its parts come with "Select contents".
            view._ui_insp_node = None
            _select(win, placements() if tnode.get("kind") in ("link", "linked") else [])
            sel = list(scene.selection)
            view._ui_insp_node = (node, (len(sel), id(sel[0]) if sel else 0, id(sel[-1]) if sel else 0))
        elif verb == "contents":
            _select(win, members())
        elif verb == "linkfinder" and linked:
            from core.desktop_open import reveal
            if tnode.get("path"):
                reveal(tnode["path"])
        elif verb in ("linkopen", "linkreload", "linkunlink", "linkrelink", "linkresettags", "linksubset", "linknested", "linkpack") and linked:
            gs = placements()
            if not gs:
                return
            if verb != "linkopen" and tnode.get("via"):
                vp.flash_status(f"{tnode.get('name')} comes inside {tnode['via']}: reload or unlink {tnode['via']}")
                return
            if verb in ("linksubset", "linknested") and len(gs) != 1:
                vp.flash_status("Select one occurrence of this linked file first")
                return
            if verb == "linkresettags":
                _core(vp, "reset_link_tags", path=tnode["path"])
                vp.notify_scene_changed()
            elif verb == "linkunlink":
                _core(vp, "unlink_placements", placements=[tree.address(g) for g in gs])
                vp.notify_scene_changed()
            else:
                _link_action(win, view, gs[0], verb)
            rebuild()
        elif verb in ("active", "filehere", "file", "newchild", "delete", "kind", "rename", "hidemembers") \
                and (linked or (verb == "file" and link_structure.is_linked_key(ident if kind == "n" else rk))):
            read_only()
        elif verb in ("node", "item", "loose") or verb == "instances":
            if verb == "node" and node:
                view._ui_insp_node = None   # set after the selection lands
            gs = members() if verb != "instances" else [
                g for g, _s, _r in structure._placements(scene) if item() is not None and g.mesh is item().mesh]
            _select(win, gs)
            if verb == "node" and node:
                sel = list(scene.selection)
                view._ui_insp_node = (node, (len(sel), id(sel[0]) if sel else 0, id(sel[-1]) if sel else 0))
        elif verb == "reveal":
            g = item()
            where = data.get("node_of", {}).get(id(g)) if g is not None else None
            k = where
            while k:
                exp.add(f"n:{k}")
                k = data.get("nodes", {}).get(k, {}).get("parent")
            if g is not None:
                _select(win, [g])
            rebuild()
        elif verb == "isolate":
            _isolate(win, [tree.row_id(g) if tree else g.uid for g in shown()])
        elif verb == "open":
            from core.outliner import path_to
            from views.tree_navigation import open_to
            g = item()
            if g is not None:
                parents = path_to(scene.groups, g)
                if parents is not None:
                    open_to(vp, [*parents, g])
        elif verb == "zoom":
            _select(win, shown())
            win._on_zoom_selection()
        elif verb == "eye":
            folders = link_folders(node) if node else []
            if folders:
                # A linked file (or one of its tag folders): its folder in
                # the host, so its nested files follow.
                on = not all(L.folder_visible(scene, f) for f in folders)
                _core(vp, "set_tag_state", folders=folders, visible=on)
            elif node:
                ts = tags(node)
                if ts:
                    layer = scene.layer(ts[0])
                    shown_now = (L.effective_visible(scene, layer) if linked else bool(layer.visible)) \
                        if layer is not None else False
                    _core(vp, "set_tag_state", tags=ts, visible=not shown_now)
            else:
                gs = members()
                if gs:
                    verb_ = "unhide" if getattr(gs[0], "hidden", False) else "hide"
                    _core(vp, verb_, placements=[address(g) for g in gs])
            vp.notify_scene_changed() if hasattr(vp, "notify_scene_changed") else vp.update()
        elif verb == "lock" or verb == "lockitem":
            if node:
                ts = tree.file_tags(node) if tree is not None and tnode.get("kind") in ("link", "linked") \
                    else tags(node)
                if ts:
                    layer = scene.layer(ts[0])
                    _core(vp, "set_tag_state", tags=ts, locked=not bool(getattr(layer, "locked", False)))
            else:
                gs = members()
                if gs:
                    _core(vp, "lock", placements=[address(g) for g in gs], locked=not getattr(gs[0], "locked", False))
            vp.update()
        elif verb in ("hidemembers", "hideitem"):
            gs = members()
            if gs:
                _core(vp, "hide", placements=[address(g) for g in gs])
                vp.update()
        elif verb == "unique":
            gs = members()
            comps = [g for g in gs if g.is_component()]
            if comps:
                vp.history.execute(CompoundCommand([MakeUniqueCommand(g, rename=True) for g in comps]))
                vp.update()
        elif verb == "fileto":
            _select(win, members())
            _file_to(win, len(members()))
        elif verb == "parent" and node:
            _core(vp, "edit_structure_node", node=node, field="parent", value=extra)
            vp.notify_scene_changed()
            rebuild()
        elif verb == "rename":
            if node:
                _core(vp, "edit_structure_node", node=node, field="name", value=extra)
                vp.notify_scene_changed()
                rebuild()
            elif item() is not None:
                _core(vp, "rename_placement", placement=address(item()), name=extra)
                vp.update()
        elif verb == "fileitem" and (group := item()) is not None:
            _core(vp, "file_placements", placements=[address(group)], node=extra)
            vp.notify_scene_changed()
            rebuild()
        elif verb in ("file", "filehere"):
            key = node if verb == "filehere" else ident if kind == "n" else rk
            uids = [address(g) for g in scene.selection if isinstance(g, Group)]
            if uids and key:
                _core(vp, "file_placements", placements=uids, node=key)
                vp.notify_scene_changed()
                rebuild()
                vp.flash_status(f"Filed {len(uids)} to " + " › ".join(structure.path(bim_registry.get(scene), key)))
        elif verb == "active":
            _core(vp, "edit_structure_node", node=node, field="active", value="")
            vp.notify_scene_changed()
            rebuild()
        elif verb == "kind" and kind == "preview-tag":
            cls = next((c for c, label in KINDS.items() if c and label == extra), None)
            if cls:
                _core(vp, "promote_tag", tag=ident, ifc_class=cls)
                vp.notify_scene_changed()
                rebuild()
        elif verb in ("tagselect","tagisolate") and kind == "preview-tag":
            matches = [g for g in data["unfiled"] if (g.layer or L.DEFAULT_LAYER) == ident]
            if verb == "tagselect":
                _select(win, matches)
            else:
                _isolate(win, matches)
        elif verb == "kind" and node:
            cls = next((c for c, k in KINDS.items() if k == extra and c), None)
            recipe = dict(structure._nodes(bim_registry.get(scene)).get(node, {}).get("recipe", {}))
            if cls:
                recipe["class"] = cls
            else:
                recipe.pop("class", None)
            _core(vp, "set_filing_rule", node=node, recipe=recipe, expected=structure._nodes(bim_registry.get(scene))[node].get("recipe", {}))
            vp.refresh_after_edit()
        elif verb in ("new", "newchild"):
            registry = bim_registry.get(scene)
            nodes = registry.setdefault("structure", {}).setdefault("nodes", {})
            parent = node if verb == "newchild" else None
            from core.group import new_uid
            order = max((n["order"] for n in nodes.values() if n["parent"] == parent), default=-1) + 1
            nodes[new_uid()] = dict(name="New node", parent=parent, order=order)
            if parent:
                exp.add(f"n:{parent}")
            execute(vp, SetRegistryCommand(registry))
        elif verb == "delete" and node:
            n = len(structure.members(scene, node))
            name = data.get("nodes", {}).get(node, {}).get("name", "")
            body = (f"Its {n} items move to the parent node." if n else "It has no items.")
            view._ui_pending = ("delete", node)
            view.renderer.ui_set_prompt(f"Delete “{name}”?", body, [("yes", "Delete"), ("no", "Cancel")])
        elif verb == "propose":
            registry = bim_registry.get(scene)
            proposal = structure.bootstrap(scene, registry)
            view._ui_pending = ("propose", proposal)
            nn = len(proposal["structure"]["nodes"])
            nf = sum(len(v) for v in proposal["filing"].values())
            nc = len(proposal["conflicts"])
            if not nn:
                # Nothing to propose: say why and offer the way that works.
                view._ui_pending = None
                view.renderer.ui_set_prompt("Nothing to propose from tags",
                                            "Every item is on the same tag, so there is no grouping to build "
                                            "nodes from. Start one with New node, then File to… from the selection.",
                                            [("new", "New node"), ("no", "Close")])
                return
            replace = " It replaces the current structure." if structure._nodes(registry) else ""
            view.renderer.ui_set_prompt("Propose structure from tags",
                                        f"{nn} nodes, {nf} items filed, {nc} to review.{replace}",
                                        [("yes", "Apply"), ("no", "Cancel")])
        elif verb == "prompt" and rk == "new":
            _tree_action(win, view, f"new{SEP}")
        elif verb == "prompt":
            pending, view._ui_pending = getattr(view, "_ui_pending", None), None
            if rk == "yes" and pending:
                what, arg = pending
                if what == "propose":
                    execute(vp, structure.apply_bootstrap(scene, arg))
                elif what == "delete":
                    _delete_node(vp, arg)
                elif what == "lib":
                    from views import wgpu_library
                    wgpu_library.confirmed(win, view, arg, extra)
                elif what == "visibility_set":
                    document, operation, key = arg
                    if document is scene:
                        if _core(vp, "edit_visibility_set", verb=operation, key=key, name=extra)["ok"]:
                            vp.notify_scene_changed()
                            view.update()
                elif what == "scene_rename" and extra.strip():
                    panel = getattr(getattr(win, "scenes_dock", None), "panel", None) or getattr(win.tray, "scenes", None)
                    if panel is not None:
                        panel.rename_view(arg, extra.strip())
                elif what == "scene_delete":
                    panel = getattr(getattr(win, "scenes_dock", None), "panel", None) or getattr(win.tray, "scenes", None)
                    if panel is not None:
                        panel.delete_views([arg])
        elif verb == "edit":
            what, _, arg = rk.partition(":")
            if what == "section":
                uid, _, field = arg.partition(":")
                _core(vp, "edit_section_plane", uid=uid, field=field, value=extra)
                vp.notify_scene_changed()
                rebuild()
            elif what == "nodename":
                _core(vp, "edit_structure_node", node=arg, field="name", value=extra)
                vp.notify_scene_changed()
                rebuild()
            elif what in ("predef", "objtype"):
                recipe = dict(structure._nodes(bim_registry.get(scene))[arg].get("recipe", {}))
                field = "predefined_type" if what == "predef" else "object_type"
                if extra.strip():
                    recipe[field] = extra.strip()
                else:
                    recipe.pop(field, None)
                _core(vp, "set_filing_rule", node=arg, recipe=recipe, expected=structure._nodes(bim_registry.get(scene))[arg].get("recipe", {}))
                vp.refresh_after_edit()
            elif what == "segments":
                from core.curve_segments import MAX_SEGMENTS
                sel = list(scene.selection)
                shape = _curve_of(scene, sel)
                n = int(extra.strip())
                if shape is None or not shape.flat:
                    raise ValueError("This curve cannot be rebuilt")
                if not shape.minimum <= n <= MAX_SEGMENTS:
                    raise ValueError(f"Between {shape.minimum} and {MAX_SEGMENTS}")
                if n != shape.segments:
                    out = _core(vp, "resegment_curve", edge=sel[0].key, segments=n)
                    if out and out.get("rejected"):
                        raise ValueError("This curve cannot be rebuilt")
                    vp.update()
            elif what == "libmat":
                from views import wgpu_library
                wgpu_library.edit_field(win, view, arg, extra)
            elif what == "groupname":
                g = _group_by_uid(win, arg)
                if g is not None:
                    _core(vp, "rename_placement", placement=g.uid, name=extra)
                    vp.update()
            view._ui_insp_sig = None
            rebuild()
        elif verb == "showall":
            _show_all(vp)
        elif verb == "unfiled":
            _select(win, data.get("unfiled", []))
    except (KeyError, ValueError) as exc:
        vp.flash_status(str(exc))


def _core(vp, command, /, **args):
    """Run a core protocol command (docs/core-protocol.md §1): the UI edits
    the model through these, by key. A key outside the open context is
    told, not raised."""
    from core.protocol import commands
    try:
        return commands.run(vp.history, command, **args)
    except KeyError as exc:
        vp.flash_status(str(exc).strip("'\""))
        return None


def _delete_node(vp, key) -> None:
    from core import bim_registry, structure
    from core.history import CompoundCommand, SetRegistryCommand
    from views.tree_navigation import execute
    scene = vp.scene
    registry = bim_registry.get(scene)
    section = registry["structure"]
    if structure.tree(registry)[key]:
        raise ValueError("Remove its child nodes first")
    parent = section["nodes"][key]["parent"]
    uids = structure.members(scene, key)
    commands = []
    if uids:
        if not parent:
            raise ValueError("File its items elsewhere before deleting a top node")
        commands.append(structure.file(scene, uids, parent))
    del section["nodes"][key]
    if section.get("active") == key:
        section.pop("active")
    commands.insert(0, SetRegistryCommand(registry, refiled_uids=uids))
    execute(vp, CompoundCommand(commands))


ED = "\x1d"


def _edit(id_, text) -> str:
    """An editable inspector value (ui.rs inspector_rows)."""
    return f"{ED}edit{ED}{id_}{ED}{text}"


def _inspector(view, win) -> None:
    """What is in front of you, in plain words (council 2026-11-22): a node
    picked in the tree, one item, several, or loose geometry. Fields edit in
    place; IFC stays under Export details."""
    scene = win.viewport.scene
    sel = list(scene.selection)
    focus = getattr(view, "_ui_insp_node", None)
    sel_sig = (len(sel), id(sel[0]) if sel else 0, id(sel[-1]) if sel else 0)
    node_focus = focus[0] if focus and focus[1] == sel_sig else None
    filename = str(getattr(win, "_current_path", "") or "")
    sig = (id(scene), scene.content_version, sel_sig, node_focus, getattr(view, "_ui_tree_sig", None) is not None,
           view.__dict__.get("_ui_mat_focus"), view.__dict__.get("_ui_mode"), view.__dict__.get("_ui_lens"),
           filename, frozenset(map(id, sel)),
           _current_material_name(), _active_tool_id(win), scene.show_section_planes, scene.show_section_cuts,
           scene.display_style.section_fill, view.__dict__.get("_ui_sheet_revision"))
    if sig == getattr(view, "_ui_insp_sig", None):
        return
    view._ui_insp_sig = sig
    from collections import Counter
    from core import bim_registry, links, structure
    from core.group import Group
    if view.__dict__.get("_ui_lens") == 2:
        focused_row = "n:" + node_focus if node_focus and node_focus != "solid:active" else None
        paths = [] if node_focus else [scene.occurrence_path(g) for g in sel if isinstance(g, Group)]
        view.renderer.ui_set_export_focus(focused_row, paths)
    registry = bim_registry.get(scene)
    nodes = structure._nodes(registry)
    data = getattr(view, "_ui_tree", None) or {}
    from views import wgpu_library
    if wgpu_library.inspector_key(view, win) is not None:
        got = wgpu_library.inspector(view, win)
        if got is not None:
            view.renderer.ui_set_inspector(*got)
            return
    from core.section import SectionPlane
    from ingetrazo_core import section_inspector, selection_identity
    section_key = node_focus or (f"section:{sel[0].uid}" if len(sel) == 1 and isinstance(sel[0], SectionPlane) else "")
    if node_focus == "solid:active" and view.__dict__.get("_ui_lens") == 2:
        from ingetrazo_core import solid_inspector_sections
        view.renderer.ui_set_inspector(scene.edit_group.name if scene.edit_group else "Loose geometry",
                                       solid_inspector_sections(scene, None, True))
        return
    if node_focus and node_focus.startswith(("sheet-page:", "sheet-frame:", "saved-scene:")):
        sheet = view._ui_sheet_catalog.inspector(node_focus)
        if sheet is not None:
            view.renderer.ui_set_inspector(*sheet)
            return
    if section_key.startswith(("section:", "sections:")):
        section = section_inspector(scene, section_key)
        if section is not None:
            view.renderer.ui_set_inspector(*section)
            if not node_focus and sel:
                view.renderer.ui_set_identity(selection_identity(scene, data, filename, KINDS))
            return
    if node_focus is not None and node_focus in (data.get("nodes") or {}) \
            and data["nodes"][node_focus].get("kind") != "host":
        view.renderer.ui_set_inspector(*_linked_inspector(view, win, data, node_focus))
        return
    if node_focus is not None and node_focus in nodes:
        node = nodes[node_focus]
        recipe = structure.resolve(registry, node_focus)
        own = node.get("recipe", {})
        members = _tree_groups(view, node_focus)
        missing = sum(1 for g in members if (g.ifc or {}).get("class") and not structure._has_status(g.ifc or {}))
        kind = "Reference" if "source" in node else kind_of(recipe.get("class"))
        rows = [("Name", _edit(f"nodename:{node_focus}", node["name"])), ("Kind", kind),
                ("Items", str(len(members)))]
        if missing:
            rows.append(("Missing Status", str(missing)))
        sections = [("Node", rows),
                    ("Actions", [(f"kindmenu:{node_focus}", "Change kind…"), (f"active:{node_focus}", "Draw into"),
                                 (f"isolate:{node_focus}", "Isolate"), (f"cutlist:{node_focus}", "Cut list…")])]
        if missing:
            sections.append(("Actions", [(f"nodestatus:{node_focus}:" + v, "All " + v.lower()) for v in STATUS_CHOICES]))
        export = [("IFC class", str(recipe.get("class") or "none (drawing only)")),
                  ("Predefined type", _edit(f"predef:{node_focus}", str(own.get("predefined_type") or ""))),
                  ("Object type", _edit(f"objtype:{node_focus}", str(own.get("object_type") or ""))),
                  ("Tag", str(recipe.get("tag") or "—"))]
        for c in recipe.get("classifications", [])[:4]:
            export.append((c.get("system", "Classification"), f"{c.get('code', '')} {c.get('name', '')}".strip()))
        if recipe.get("systems"):
            export.append(("Systems", ", ".join(recipe["systems"])))
        if view.__dict__.get("_ui_qty"):
            total = {}
            for g in members:
                for u, v in _group_qty(view, g).items():
                    total[u] = total.get(u, 0.0) + v
            sections.append(("Quantities", _qty_rows(total) or [("Measured", "nothing to measure")]))
        else:
            sections.append(("Actions", [("measure", "Measure quantities")]))
        sections.append(("Export details", export))
        view.renderer.ui_set_inspector(node["name"], sections)
        return
    if not sel:
        view.renderer.ui_set_inspector("", [])
        return
    identity = selection_identity(scene, data, filename, KINDS)
    groups = [g for g in sel if isinstance(g, Group)]
    from views import verbs
    sel_kind = verbs.kind_of(scene, sel)
    if len(sel) == 1 and groups:
        g = groups[0]
        rec = g.ifc or {}
        key = data.get("node_of", {}).get(id(g)) or structure.node_of(g)
        filed_ok = key in nodes
        recipe = data.get("recipes", {}).get(key, {})
        name_cell = _edit(f"groupname:{g.uid}", g.name or "") if verbs.name_editable(sel_kind) else (g.name or "")
        rows = [("Name", name_cell)]
        if verbs.read_only(sel_kind):
            rows.append(("Edits", "Read-only here"))
        if g.is_component():
            same = sum(1 for o, _s, _r in structure._placements(scene) if o.mesh is g.mesh)
            rows.append(("Copies", str(same)))
        if links.link_of(g) or sel_kind == "linked_part":
            needs = [("Record", "Kept in the source file")]
        elif not filed_ok:
            needs = [("Filing", "Needed first")]
        elif not rec:
            needs = [("Record", "Comes with a kind")]
        else:
            needs = [("Status", _status_text(rec) or "Missing")]
        sections = [("Item", rows), ("What it needs", needs)]
        if rec.get("class") and verbs.offers(sel_kind, "status"):
            sections.append(("Actions", [("status:" + v, v.capitalize()) for v in STATUS_CHOICES]))
        if sel_kind == "linked_part":
            sections.append(("Actions", [("linkfile", "Select its linked file"), ("linkopen", "Edit source")]))
        kids = list(getattr(g, "children", None) or [])
        if kids:
            parts = Counter(c.name or "Part" for c in kids)
            prow = [(n, f"×{k}") for n, k in parts.most_common(8)]
            if len(parts) > 8:
                prow.append(("…", f"{len(parts) - 8} more kinds"))
            if view.__dict__.get("_ui_lens") != 2 or not hasattr(view.renderer, "ui_panel_set"):
                sections.append(("Parts", prow))
            sections.append(("Actions", [("cutlist", "Cut list…")]))
        from ingetrazo_core import solid_inspector_sections
        solid = solid_inspector_sections(scene, g, view.__dict__.get("_ui_lens") == 2)
        if solid and not verbs.offers(sel_kind, "repair"):
            solid = [s_ for s_ in solid if s_[0] != "Actions"]
        if solid:
            sections += solid
        link = links.link_of(g)
        if link:
            state = _link_state(scene, link)
            from ingetrazo_core import link_subset_rows
            nested = link.get("include_nested", True)
            sections.append(("Linked file", [("Source", str(link.get("path", "")).rsplit("/", 1)[-1]),
                                             ("Status", LINK_STATES.get(state, state)), *link_subset_rows(g),
                                             ("Nested links", "included" if nested else "source only")]))
            acts = [("linkopen", "Edit source"), ("linkreload", "Reload" if state != "modified" else "Reload changes"),
                    ("linkunlink", "Unlink"), ("linkfinder", "Show in Finder")]
            sections.append(("Actions", acts))
            sections.append(("Actions", [("linkrelink", "Relink…"),
                                         ("linkpack", "Unpack" if state == "packed" else "Pack into this file"),
                                         ("linknested", "Source only" if nested else "Include nested links")]))
            if links.linked_ancestor(g.owner) is None:
                sections.append(("Actions", [("linksubset", "Edit linked subset…")]))
        if _group_qty(view, g):
            sections.append(("Quantities", _qty_rows(_group_qty(view, g))))
        export = [("IFC class", str(rec.get("class") or recipe.get("class") or "generic object")),
                  ("Predefined type", str(rec.get("predefined_type") or recipe.get("predefined_type") or "—")),
                  ("Tag", str(getattr(g, "layer", None) or "—"))]
        sections.append(("Export details", export))
        view.renderer.ui_set_inspector(g.name or "Group", sections)
        view.renderer.ui_set_identity(identity)
        return
    kinds = Counter()
    unfiled = 0
    for g in groups:
        key = structure.node_of(g)
        if key in nodes:
            kinds[nodes[key]["name"]] += 1
        else:
            unfiled += 1
    rows = [(name, str(n)) for name, n in kinds.most_common(8)]
    if unfiled:
        rows.append(("Not filed", str(unfiled)))
    others = len(sel) - len(groups)
    if others:
        from collections import Counter as C2
        from core.mesh import Edge, Face
        loose = C2("Faces" if isinstance(e, Face) else "Edges" if isinstance(e, Edge) else type(e).__name__
                   for e in sel if not isinstance(e, Group))
        rows += [(t, str(n)) for t, n in loose.most_common()]
    sections = [("By node" if groups else "Geometry", rows)]
    if any(type(e).__name__ == "Dimension" for e in sel):
        sections.append(("Actions", [("form:dimstyle", "Dimension style…")]))
    shape = None if groups else _curve_of(scene, sel)
    if shape is not None:
        # «Sides» for a closed shape, as the Circle and Polygon tools count
        # them; «Segments» for an arc. Only a flat curve is rebuilt.
        label = "Sides" if shape.closed else "Segments"
        value = _edit("segments", str(shape.segments)) if shape.flat else f"{shape.segments} (not flat)"
        sections.insert(0, ("Circle" if shape.closed else "Arc", [(label, value)]))
    missing = sum(1 for g in groups if (g.ifc or {}).get("class") and not structure._has_status(g.ifc or {}))
    if missing and verbs.offers(sel_kind, "status"):
        sections.append(("Status", [("Missing on", f"{missing} items")]))
        sections.append(("Actions", [("status:" + v, "All " + v.lower()) for v in STATUS_CHOICES]))
    if groups:
        sections.append(("Export details", []))
    view.renderer.ui_set_inspector(f"{len(sel)} selected", sections)
    view.renderer.ui_set_identity(identity)


def _linked_inspector(view, win, data, key):
    """A linked row in the inspector: what it is (a reference to another
    file, read-only here), where that file is, its state and visibility,
    and what can be done with it."""
    from pathlib import Path
    from core import layers as L
    from core import links
    scene = win.viewport.scene
    tree = data["tree"]
    node = tree.nodes[key]
    count = data.get("counts", {}).get(key, 0)
    kind = node["kind"]
    if kind == "linked":
        files = [tree.nodes[c] for c in tree.children.get(key, ()) if tree.nodes[c]["kind"] == "link"]
        rows = [("What", "Files this document references"), ("Files", str(len(files))), ("Items", str(count))]
        rows += [(f["name"], str(data["counts"].get(c, 0)) + " items")
                 for c, f in ((c, tree.nodes[c]) for c in tree.children.get(key, ()))
                 if f["kind"] == "link"][:12]
        acts = [("lnk:eye:" + key, "Show / hide all"), ("isolate:" + key, "Isolate")]
        return node["name"], [("Linked files", rows), ("Actions", acts)]
    path = node["path"] or ""
    stems = node["stems"]
    if kind == "link":
        copies = tree.links.get(key, [])
        top = len(stems) == 1
        visible = all(L.folder_visible(scene, f) for f in node.get("folders") or [node["folder"]])
        entry = links.registry(scene).get(path) or {}
        overrides = len(entry.get("overrides") or {})
        parent = Path(path).parent
        where = str(parent) if len(parent.parts) <= 3 else "…/" + "/".join(parent.parts[-2:])
        from core.link_structure import file_state
        state = file_state(scene, path, nested=not top)
        rows = [("What", "Linked file (read-only)"), ("File", Path(path).name),
                ("Folder", where), ("Status", LINK_STATES.get(state, state)),
                ("Placed", f"{len(copies)} " + ("copy" if len(copies) == 1 else "copies")),
                ("Items", str(count)),
                ("Visibility", ("Shown" if visible else "Hidden")
                 + (f" · {overrides} tag override{'s' * (overrides != 1)}" if overrides else ""))]
        if not top:
            rows.insert(2, ("Via", " › ".join(stems[:-1])))
        if node.get("merged"):
            rows.append(("Top node", node["merged"]))
        # The file verbs first (views/verbs.py), then what any row offers.
        from views import verbs
        labels = {"linkopen": "Edit source", "linkreload": "Reload" if state != "modified" else "Reload changes",
                  "linkunlink": "Unlink", "linkfinder": "Show in Finder", "linkrelink": "Locate…"}
        acts = [("lnk:" + v + ":" + key, labels[v]) for v in verbs.file_verbs(not top)
                if state != "missing" or v in ("linkunlink", "linkrelink")]
        if len(copies) == 1:
            from ingetrazo_core import link_subset_rows
            rows.extend(link_subset_rows(copies[0]))
            if top and state != "missing":
                acts.append(("lnk:linksubset:" + key, "Edit linked subset…"))
                nested = links.link_of(copies[0]).get("include_nested", True)
                acts.append(("lnk:linknested:" + key, "Source only" if nested else "Include nested links"))
        if top and state != "missing":
            acts.append(("lnk:linkpack:" + key, "Unpack" if state == "packed" else "Pack into this file"))
        if overrides and top:
            acts.append(("lnk:linkresettags:" + key, "Show tags as the source has them"))
        if state != "missing":
            acts.append(("lnk:contents:" + key, "Select contents"))
        if not copies:
            acts += [("isolate:" + key, "Isolate"), ("lnk:eye:" + key, "Hide" if visible else "Show")]
        return Path(path).name or node["name"], [("Linked file", rows), ("Actions", acts)]
    ts = [t for t in (data.get("node_layers", {}).get(k, ()) for k in tree.subtree(key)) for t in t]
    if node["tag"]:
        ts.append(node["tag"])
    layer = scene.layer(ts[0]) if ts else None
    visible = L.effective_visible(scene, layer) if layer is not None else True
    cls = node["recipe"].get("class")
    rows = [("Name", node["name"]), ("Kind", "Drawing only" if kind == "tag" else "Tag folder" if kind == "tagfolder"
                                       else kind_of(cls) if cls or not tree.children.get(key) else "Group of nodes"),
            ("From", Path(path).name + " — read-only here"), ("Items", str(count)),
            ("Visibility", "Shown" if visible else "Hidden")]
    acts = [("lnk:eye:" + key, "Hide" if visible else "Show"), ("isolate:" + key, "Isolate"),
            ("lnk:contents:" + key, "Select contents")]
    export = [("IFC class", str(cls or "none (drawing only)")),
              ("Tag", str(scene.layer(node["tag"]).name if node["tag"] and scene.layer(node["tag"]) else "—"))]
    return node["name"], [("Node", rows), ("Actions", acts), ("Export details", export)]


LINK_STATES = {"resolved": "Up to date", "modified": "Changed on disk", "missing": "Missing", "packed": "Packed in this file"}


def _link_state(scene, link) -> str:
    from core.links import link_state
    return link_state(scene, link["path"])


def _link_action(win, view, g, key) -> None:
    """The Linked files panel's verbs on the selected placement."""
    from pathlib import Path
    from core import links
    vp = win.viewport
    scene = vp.scene
    rec = links.link_of(g)
    if not rec:
        return
    try:
        if key == "linkfinder":
            from core.desktop_open import reveal
            reveal(rec["path"])
            return
        if key == "linkunlink":
            _core(vp, "unlink_placements", placements=[scene.occurrence_path(g)])
            vp.notify_scene_changed()
            view._ui_insp_sig = None
            view._ui_tree_sig = None
            return
        if key == "linksubset":
            import json
            from views.chrome_forms import ask_text
            address = scene.occurrence_path(g)
            def accept(text):
                if text is None or win.viewport.scene is not scene:
                    return
                try:
                    result = _core(vp, "edit_link_subset", placement=address, text=text)
                    if result and result["ok"]:
                        win._relink_done("Linked subset updated")
                    elif result and (result["error"] or result["rejected"]):
                        vp.flash_status(result["error"] or result["rejected"])
                except (KeyError, ValueError, OSError, links.LinkError) as exc:
                    vp.flash_status(str(exc))
            ask_text(win, "Edit linked subset", 'Full document (null), {"uids":[…]} or {"filter":{…}}',
                     json.dumps(rec.get("select"), indent=2), done=accept, multiline=True)
            return
        if key == "linkreload":
            links.reload(scene, rec["path"])
        elif key == "linkopen":
            # A ⛓ tab right of this project (views/wgpu_tabs.py).
            from views import wgpu_tabs
            wgpu_tabs.open_link_source(win, Path(rec["path"]), g)
            return
        elif key == "linkpack":
            packed = not links.is_packed(scene, rec["path"])
            if not packed:
                links.reload(scene, rec["path"])
            links.set_packed(scene, rec["path"], packed)
        elif key == "linknested":
            win._set_link_nested(g, not rec.get("include_nested", True))
        elif key == "linkrelink":
            from views.filedialogs import file_dialogs
            from formats import built
            source = Path(rec["path"])
            path, _ = file_dialogs.getOpenFileName(win, f"Locate {source.name}", str(source.parent),
                                                   built.DOCUMENT_FILTER)
            if not path:
                return
            records = [links.link_of(copy) for copy in links.linked_groups(scene, source)]
            previous = [dict(record) for record in records]
            for record in records:
                record.update(links._link_record(path, win._current_path))
            try:
                links.reload(scene, path)
            except Exception:
                for record, original in zip(records, previous):
                    record.clear()
                    record.update(original)
                raise
        win._relink_done("Linked files updated")
    except (ValueError, OSError, links.LinkError) as exc:
        vp.flash_status(str(exc))
    view._ui_rules_cache = None
    view._ui_insp_sig = None
    view._ui_tree_sig = None


def _curve_of(scene, sel):
    """One curve selected (all its edges, one curve id) that reads back as a
    regular circle, arc or polygon; else None."""
    from core.curve_segments import read_curve
    from core.mesh import Edge
    if not sel or len(sel) > 4096:
        return None
    ids = {getattr(e, "curve", None) if isinstance(e, Edge) else None for e in sel}
    if len(ids) != 1 or None in ids:
        return None
    shape = read_curve(scene.mesh, sel[0])
    return shape if shape is not None and len(sel) == shape.segments else None


#: The IFC Status values offered as one click (the rest stay in the BIM panel).
STATUS_CHOICES = ["EXISTING", "NEW", "DEMOLISH", "TEMPORARY"]


def _status_text(rec) -> str:
    for name, props in (rec.get("psets") or {}).items():
        if name.endswith("Common"):
            value = props.get("Status")
            if isinstance(value, dict):
                value = value.get("value")
            if value:
                return str(value).capitalize()
    return ""


def _set_status(win, groups, value) -> None:
    """Pset_<Class>Common.Status on every selected item that has a record:
    one undo step."""
    from copy import deepcopy
    from core.history import CompoundCommand, SetRecordCommand
    from core.ifc_tables import common_pset
    from views.tree_navigation import execute
    commands = []
    for g in groups:
        rec = g.ifc or {}
        cls = rec.get("class")
        if not cls:
            continue
        psets = deepcopy(rec.get("psets") or {})
        name = next((n for n in psets if n.endswith("Common")), common_pset(cls))
        psets.setdefault(name, {})["Status"] = value
        commands.append(SetRecordCommand([g], {"psets": psets}))
    if commands:
        execute(win.viewport, CompoundCommand(commands))
        win.viewport.flash_status(f"Status {value.capitalize()} on {len(commands)} item" + ("s" if len(commands) != 1 else ""))


def _dock(win, names) -> None:
    from PySide6.QtWidgets import QDockWidget
    for dock in win.findChildren(QDockWidget):
        if any(n in dock.windowTitle() for n in names):
            dock.show()
            dock.raise_()
            return


def _file_to(win, n: int) -> None:
    """The tree takes the next pick (or typed name + Enter)."""
    view = getattr(getattr(win, "_doc", None), "stand_in", None)
    if view is not None:
        view.renderer.ui_start_filing(f"{n} item" + ("s" if n != 1 else ""))
        view.update()


def after_frame(view) -> None:
    if not getattr(view, "_ui_attached", False):
        return
    view._ui_frames = getattr(view, "_ui_frames", 0) + 1
    renderer = view.renderer
    actions = renderer.ui_take_actions()
    if actions:
        win = _window(view)
        # Never switch tools from inside a paint.
        QTimer.singleShot(0, view, lambda: _run(win, actions, view))
    delay = renderer.ui_repaint_after_ms()
    if view.__dict__.get("_ui_poll_jobs"):
        delay = 400 if delay is None else min(delay, 400)
    if delay is not None:
        view._ui_timer.start(max(0, int(delay)))


def free_area(win):
    """Where the model is seen, in device coordinates (ui.rs ``free_ndc``):
    the window minus the rail, dock and bars showing. None without the chrome."""
    view = getattr(getattr(win, "_doc", None), "stand_in", None)
    if view is None or not getattr(view, "_ui_attached", False):
        return None
    return view.renderer.ui_free_ndc()


def _review_camera(win, on: bool) -> None:
    """Review frames the model into the window the panels left (B8); leaving
    it, the camera goes back to where it was if Review's framing is untouched."""
    if on:
        before = win._camera_pose()
        win._on_zoom_extents()
        win.__dict__["_review_back"] = (before, win._camera_pose())
        return
    back = win.__dict__.pop("_review_back", None)
    if back is not None and back[1] == win._camera_pose():
        cam = win.viewport.camera
        x, y, z, cam.distance, cam.yaw, cam.pitch = back[0]
        cam.target = (x, y, z)
        win.viewport.update()


def _run(win, actions, source_view=None) -> None:
    if win is None:
        return
    from views import chrome_dialogs
    for action in actions:
        kind, _, arg = action.partition(":")
        if kind == "scenephoto":
            from ingetrazo_render import render_scene_photo
            view = chrome_dialogs.view_of(win)
            if view is not None:
                pixels = None
                try:
                    pixels = render_scene_photo(win, arg)
                except (RuntimeError, ValueError, OSError):
                    pass  # The visible row retries a missed frame.
                finally:
                    view.renderer.ui_finish_scene_photo(arg, pixels)
            continue
        if kind == "scenephotomode":
            from ingetrazo_render import save_thumbnail_mode
            save_thumbnail_mode(arg == "1")
            continue
        if kind == "font":
            from views.font_picker import answer
            answer(win, arg, source_view)
            continue
        if kind == "review":
            _review_camera(win, arg == "1")
            continue
        if kind == "sheet":
            chrome_dialogs.answer(win, arg)
            continue
        if kind == "job":
            verb, _, jid = arg.partition(":")
            job = chrome_dialogs.find_job(win, jid)
            if verb == "cancel" and job is not None:
                job.cancel()
            continue
        if kind == "dlg":
            if chrome_dialogs.current_sheet(win) is None:
                from views import chrome_forms
                chrome_forms.action(win, arg)
            continue
        if chrome_dialogs.modal(win):
            continue    # a click queued before the sheet came up
        if kind == "plugin":
            from core.plugin_panels import dispatch
            dispatch(win, arg)
            continue
        if kind == "activity":
            row = next((r for r in chrome_dialogs.history(win) if r.id == arg), None)
            if row and row.action:
                row.action()
            continue
        if kind == "linkbanner":
            if arg == "locate":
                from core import links
                scene = win.viewport.scene
                group = next((g for g in links.linked_groups(scene)
                              if links.link_state(scene, links.link_of(g)["path"]) == "missing"), None)
                if group:
                    _link_action(win, chrome_dialogs.view_of(win), group, "linkrelink")
                chrome_dialogs.update_link_banner(win)
            else:
                win._doc._missing_link_banner = ""
            chrome_dialogs._repaint(win)
            continue
        if kind == "lsheet":
            from views import chrome_sheet
            chrome_sheet.action(win, arg)
            continue
        if kind == "start":
            from views import wgpu_start
            wgpu_start.action(win, arg)
            continue
        if kind == "sheets":
            from views import wgpu_sheets
            wgpu_sheets.action(win, arg)
            continue
        if (kind, arg) == ("tab", "scene:editcamera"):
            from views.inspector_bridge import _marked
            win.edit_scene_camera(_marked(win))
            continue
        if kind == "panel" or (kind, arg) in (("act", "camera"), ("tab", "scene:camera")):
            from views import inspector_bridge
            inspector_bridge.action(win, win._doc.stand_in, arg if kind == "panel" else '["camera.open",null]')
            continue
        if kind == "tool":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            run = getattr(view, "_ui_palette_runs", {}).get(arg)
            if run is not None:
                run()
            else:
                win._activate_tool(arg)
        elif kind == "opt":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            run = next((t for i, _, _, _, t in _options(view) if i == arg and t), None) if view else None
            if run is not None:
                run()
                view.update()
        elif kind == "tree":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            _tree_action(win, view, arg)
        elif kind == "pal":
            from views import wgpu_library
            wgpu_library.paint_from_strip(win, arg)
        elif kind == "libsheet":
            from views import wgpu_library
            wgpu_library.open_library(win, arg or None)
        elif kind == "lib":
            from views import wgpu_library
            wgpu_library.action(win, getattr(getattr(win, "_doc", None), "stand_in", None), arg)
        elif kind == "dockw":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            keep_dock_width(view, arg)
        elif kind in ("cmdscope", "cmdq", "cmdrun", "cmdshortcut"):
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            if view is not None:
                command_action(view, win, kind, arg)
                view.update()
        elif kind == "proj":
            _project_action(win, arg)
        elif kind == "ptab":
            from views import wgpu_tabs
            wgpu_tabs.action(win, getattr(getattr(win, "_doc", None), "stand_in", None), arg)
        elif kind == "ctx":
            from views import context_menu
            context_menu.action(win, getattr(getattr(win, "_doc", None), "stand_in", None), arg)
        elif kind == "tab":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            if arg == "model":
                win._show_model()
            elif arg == "sheet":
                win._to_sheets()
            elif arg.startswith("visibility:"):
                operation, _, key = arg[len("visibility:"):].partition(":")
                vp = win.viewport
                if operation in ("save", "rename", "delete"):
                    labels = getattr(view, "_ui_visibility_cache", (None, None, []))[2]
                    label = next((r[1] for r in labels if r[0] == f"visibility:apply:{key}"), "")
                    title = {"save": "Save visibility set", "rename": "Rename visibility set",
                             "delete": f"Delete “{label}”?"}[operation]
                    body = "" if operation == "delete" else f"{ED}edit{ED}name{ED}{label}"
                    view._ui_pending = ("visibility_set", (vp.scene, operation, key))
                    view.renderer.ui_set_prompt(title, body, [("yes", operation.capitalize()), ("no", "Cancel")])
                elif _core(vp, "edit_visibility_set", verb=operation, key=key)["ok"]:
                    vp.notify_scene_changed()
                    view.update()
            elif arg.startswith("scenefolders:"):
                from ingetrazo_core import scene_tree
                vp = win.viewport
                try:
                    scene_tree.edit_menu_folder(vp.history, arg.partition(":")[2])
                    vp.notify_scene_changed()
                    view._ui_tabs = None
                    view.update()
                except ValueError as exc:
                    vp.flash_status(str(exc))
            elif arg in ("scene:add", "scene:update", "scene:up", "scene:down", "scene:delete", "scene:rename", "scene:duplicate"):
                panel = getattr(getattr(win, "scenes_dock", None), "panel", None) or getattr(win.tray, "scenes", None)
                row = getattr(getattr(win, "_scene_tabs", None), "tabs", None)
                marked = row.marked_view() if row is not None else None
                if panel is not None:
                    if arg == "scene:add":
                        panel.add_scene()
                    elif marked is not None and arg == "scene:duplicate":
                        panel.duplicate_view(marked)
                    elif marked is not None and arg == "scene:update":
                        panel.update_view(marked)
                        win.viewport.flash_status(f"Updated “{marked.name}” from the view")
                    elif marked is not None and arg in ("scene:up", "scene:down"):
                        views = list(getattr(win.viewport.scene, "saved_views", []) or [])
                        if marked in views:
                            i = views.index(marked) + (-1 if arg == "scene:up" else 1)
                            if 0 <= i < len(views):
                                panel.move_view(marked, i)
                    elif marked is not None and arg == "scene:rename":
                        view._ui_pending = ("scene_rename", marked)
                        view.renderer.ui_set_prompt("Rename scene", f"{ED}edit{ED}scene{ED}{marked.name}",
                                                    [("yes", "Rename"), ("no", "Cancel")])
                    elif marked is not None and arg == "scene:delete":
                        view._ui_pending = ("scene_delete", marked)
                        view.renderer.ui_set_prompt(f"Delete “{marked.name}”?", "The scene goes; the model does not change.",
                                                    [("yes", "Delete"), ("no", "Cancel")])
                    view._ui_tabs = None
            elif arg in ("scene:prev", "scene:next"):
                views = getattr(view, "_ui_scene_views", [])
                row = getattr(getattr(win, "_scene_tabs", None), "tabs", None)
                marked = row.marked_view() if row is not None else None
                if views:
                    i = views.index(marked) if marked in views else -1
                    i = (i + (1 if arg == "scene:next" else -1)) % len(views)
                    win.tray.scenes.show_view(views[i])
            elif arg.startswith("scene:"):
                views = getattr(view, "_ui_scene_views", [])
                i = int(arg.partition(":")[2])
                if 0 <= i < len(views):
                    win.tray.scenes.show_view(views[i])
        elif kind == "deliver":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            data = getattr(view, "_ui_tree", None) or {}
            if arg in ("unfiled", "problems"):
                from views import wgpu_library
                wgpu_library.set_mode(view, win, "problems")
            elif arg == "active" and data.get("active"):
                _select(win, _tree_groups(view, data["active"]))
            else:
                from views import wgpu_deliveries
                wgpu_deliveries.action(win, view, arg)
        elif kind == "act" and arg.startswith("form-"):
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            if view is not None:
                _open_form(view, arg[5:])
        elif kind == "form":
            view = getattr(getattr(win, "_doc", None), "stand_in", None)
            if view is not None:
                _form_apply(win, view, arg)
        elif kind == "act" and arg.isdecimal():
            # The Sheets workspace retains its composer actions.
            from views.wgpu_sheets import workspace_of
            ws = workspace_of(source_view or getattr(win._doc, "stand_in", None))
            if ws is not None and int(arg) < len(ws._actions):
                ws._actions[int(arg)].trigger()
        elif kind == "act":
            from views.view_commands import run
            run(win, arg)
        elif kind == "cmd":
            sel = list(win.viewport.scene.selection)
            run = next((r for k, _, _, r, ok, _ in _commands(win, sel) if k == arg and ok), None)
            if run is not None:
                run()


# ---- Settings forms ---------------------------------------------------------
# A form mirrors an existing panel's controls, so its logic (live apply,
# daylight range, the style library) stays the panel's: (kind, widget
# attribute, label, value text: QLabel attribute or callable, style attr).
FORMS = {
    "style": ("Style", "styles_panel", [
        ("choice", "_combo", "Style"),
        ("choice", "_mode", "Faces"),
        ("caption", None, "Edges"),
        ("check", "_edges", "Edges"),
        ("color", "_edge_c", "Edge colour", None, "edge_color"),
        ("check", "_profiles", "Profiles"),
        ("check", "_back_edges", "Back edges, dashed"),
        ("caption", None, "Faces and background"),
        ("color", "_front_c", "Front colour", None, "front_color"),
        ("color", "_back_c", "Back colour", None, "back_color"),
        ("check", "_sky", "Sky"),
        ("color", "_sky_c", "Sky colour", None, "sky_color"),
        ("color", "_ground_c", "Ground", None, "ground_color"),
        ("color", "_bg_c", "Background", None, "background"),
        ("check", "_fill", "Section fill"),
        ("color", "_fill_c", "Section fill colour", None, "section_fill_color"),
        ("caption", None, "Ambient occlusion"),
        ("check", "_ao", "Ambient occlusion"),
        ("slider", "_ao_dist", lambda p: p._ao_dist_lbl.text().rstrip(": ") or "Distance", "_ao_dist_val"),
        ("slider", "_ao_int", "Intensity", "_ao_int_val"),
        ("button", "_on_save", "Save style…"),
        ("button", "_on_delete", "Delete style", None, "_del_btn"),
    ]),
    "dimstyle": ("Dimension style", "dimstyle_panel", [
        ("choice", "_units", "Unit"),
        ("slider", "_decimals", "Decimals", lambda p: str(p._decimals.value())),
        ("check", "_show_unit", "Show the unit"),
        ("slider", "_font", "Text size", lambda p: f"{p._font.value()} pt"),
        ("button", None, "Font family…"),
        ("color", "_color_btn", "Colour", None, lambda p: [c / 255 for c in p._style().get("color", [45, 55, 75])]),
        ("choice", "_ends", "Ends"),
        ("slider", "_base_step", "Baseline step", lambda p: f"{p._base_step.value():g} mm", None, 2),
        ("text", None, "Applies to every dimension in the document."),
    ]),
    "shadows": ("Shadows and sun", "shadows_panel", [
        ("check", "_enable", "Show shadows"),
        ("slider", "_doy", "Date", lambda p: p._date.date().toString("d MMMM")),
        ("slider", "_time", "Time", "_time_lbl"),
        ("text", None, lambda p: f"Sunrise {p._sunrise_lbl.text()} · sunset {p._sunset_lbl.text()}"
                                  if p._sunrise_lbl.text() else ""),
        ("slider", "_dark", "Darkness", lambda p: f"{p._dark.value()} %"),
        ("choice", "_tz", "Time zone"),
        ("caption", None, "Site"),
        ("text", None, lambda p: p._site.text()),
        ("button", "_on_north_tool", "Set north on screen"),
        ("button", "_on_add_location", "Add location…"),
    ]),
}


def _open_form(view, name) -> None:
    win = _window(view)
    view._ui_form = name
    view._ui_form_sig = None
    panel = _form_panel(win, name) if win is not None else None
    if panel is not None and hasattr(panel, "refresh"):
        panel.refresh()
    view.update()


def _form_panel(win, name):
    spec = FORMS.get(name)
    return getattr(win, spec[1], None) if spec else None


def _form(view, win) -> None:
    """Send the open form's fields (only when they changed)."""
    name = view.__dict__.get("_ui_form")
    if not name:
        return
    panel = _form_panel(win, name)
    if panel is None:
        view._ui_form = None
        view.renderer.ui_set_form("", [])
        return
    title, _attr, spec = FORMS[name]
    fields = []

    def text_of(t):
        if callable(t):
            return str(t(panel))
        if isinstance(t, str) and t.startswith("_") and hasattr(panel, t):
            return getattr(panel, t).text()
        return str(t or "")

    for i, entry in enumerate(spec):
        kind, attr, label = entry[:3]
        extra = entry[3] if len(entry) > 3 else None
        label = text_of(label) if callable(label) else label
        fid = str(i)
        if kind == "caption":
            fields.append((fid, kind, label, "", 0.0, 0.0, 0.0, []))
            continue
        if kind == "text":
            t = text_of(label)
            if t:
                fields.append((fid, kind, "", t, 0.0, 0.0, 0.0, []))
            continue
        if kind == "button":
            gate = entry[4] if len(entry) > 4 else None
            if gate and not getattr(panel, gate).isEnabled():
                continue
            fields.append((fid, kind, label, "", 0.0, 0.0, 0.0, []))
            continue
        w = getattr(panel, attr, None)
        if w is None or w.isHidden() or not w.isEnabled():
            continue
        if kind == "check":
            fields.append((fid, kind, label, "", float(w.isChecked()), 0.0, 1.0, []))
        elif kind == "slider":
            k = entry[5] if len(entry) > 5 else 1
            fields.append((fid, kind, label, text_of(extra) if extra else str(w.value()),
                           float(w.value()) * k, float(w.minimum()) * k, float(w.maximum()) * k, []))
        elif kind == "choice":
            opts = [w.itemText(j) for j in range(w.count())]
            fields.append((fid, kind, label, w.currentText(), float(w.currentIndex()), 0.0, float(len(opts)), opts))
        elif kind == "color":
            try:
                c = entry[4](panel) if callable(entry[4]) else panel._shown_color(entry[4])
            except Exception:
                c = None
            fields.append((fid, kind, label, _hex(c) if c else "", 0.0, 0.0, 0.0, []))
    sig = (name, tuple(fields))
    if sig != view.__dict__.get("_ui_form_sig"):
        view._ui_form_sig = sig
        view.renderer.ui_set_form(title, fields)


def _form_apply(win, view, arg) -> None:
    if arg == "close":
        view._ui_form = None
        view._ui_form_sig = None
        return
    name = view.__dict__.get("_ui_form")
    panel = _form_panel(win, name)
    if panel is None:
        return
    fid, _, value = arg.partition(":")
    entry = FORMS[name][2][int(fid)]
    kind, attr = entry[0], entry[1]
    if name == "dimstyle" and kind == "button" and attr is None:
        from views.font_picker import annotation
        annotation(win)
    elif kind == "button":
        getattr(panel, attr)()
    else:
        w = getattr(panel, attr)
        if kind == "check":
            w.setChecked(value == "1")
        elif kind == "slider":
            k = entry[5] if len(entry) > 5 else 1
            v = float(value) / k
            w.setValue(v if k != 1 else int(v))
        elif kind == "choice":
            i = int(value)
            w.setCurrentIndex(i)
            w.activated.emit(i)
        elif kind == "color":
            w.click()
    if hasattr(panel, "refresh") and kind != "slider":
        panel.refresh()
    view._ui_form_sig = None
    view._ui_cluster_sig = None
    view.update()


_KEYS = {Qt.Key_PageUp: "PageUp", Qt.Key_PageDown: "PageDown", Qt.Key_Escape: "Escape",
         Qt.Key_Return: "Enter", Qt.Key_Enter: "Enter",
         Qt.Key_Backspace: "Backspace", Qt.Key_Delete: "Delete", Qt.Key_Tab: "Tab",
         Qt.Key_Up: "ArrowUp", Qt.Key_Down: "ArrowDown", Qt.Key_Left: "ArrowLeft",
         Qt.Key_Right: "ArrowRight", Qt.Key_Home: "Home", Qt.Key_End: "End",
         Qt.Key_A: "A", Qt.Key_C: "C", Qt.Key_V: "V", Qt.Key_X: "X", Qt.Key_Z: "Z",
         Qt.Key_F: "F"}


def _opens_commands(view, event) -> bool:
    from PySide6.QtGui import QKeySequence
    window = view.window()
    registry = vars(window).get("_registry_actions")
    action = registry.get("command_search") if registry is not None else getattr(window, "_act_search_commands", None)
    if action is None:
        return False
    return QKeySequence(event.key() | event.modifiers().value) in action.shortcuts()


def _all_commands(win):
    from views.command_metadata import commands
    return [(cmd.name, cmd.path, cmd.keys, cmd.action) for cmd in commands(win)]


def command_action(view, win, verb, arg):
    from views import command_metadata
    if verb == "cmdscope":
        view._ui_command_scope = arg
        view._ui_command_owner = None
    elif verb == "cmdq":
        _command_query(view, win, arg)
    elif verb in ("cmdrun", "cmdshortcut"):
        hits = getattr(view, "_ui_cmd_hits", ())
        i = int(arg)
        if 0 <= i < len(hits):
            action = hits[i]
            if verb == "cmdrun":
                command_metadata.run(action)
            elif command_metadata.visible(action):
                owner = win if hasattr(win, "open_preferences") else win.parentWidget()
                if owner is not None:
                    owner.open_preferences(shortcut_of=action)


def _command_query(view, win, query) -> None:
    from views import command_metadata, wgpu_sheets
    workspace = wgpu_sheets.workspace_of(view)
    owner = workspace.composer if workspace is not None else win
    if not query or getattr(view, "_ui_command_owner", None) is not owner:
        view._ui_command_pool = command_metadata.commands(owner, getattr(view, "_ui_command_scope", ""))
        view._ui_command_owner = owner
    live = [c for c in view._ui_command_pool if command_metadata.visible(c.action)]
    hits = command_metadata.search(owner, query, pool=live)
    view._ui_cmd_hits = [c.action for c in hits]
    view.renderer.ui_set_command_results([(c.name, c.path, c.keys, c.action.isEnabled()) for c in hits])
    detail = getattr(view.renderer, "ui_set_command_details", None)
    if detail:
        detail([(c.tip, bool(getattr(c.action, "isChecked", lambda: False)()),
                 hasattr(c.action, "isChecked")) for c in hits])


def _problem_key(view, event) -> int:
    if view.__dict__.get("_ui_lens") != 2 or not (event.modifiers() & Qt.ControlModifier):
        return 0
    return {Qt.Key_BracketRight: 1, Qt.Key_BracketLeft: -1}.get(event.key(), 0)


def _reveal_shortcut(event):
    return event.key() == Qt.Key_J and _mods(event) == 12


def claims_key(view, event) -> bool:
    """ShortcutOverride: does the chrome own this key (a focused field, or
    Escape while its popup is open)? Then no window shortcut fires."""
    if not getattr(view, "_ui_attached", False):
        return False
    from views import chrome_sheet
    large = chrome_sheet.claims_key(view, event)
    if large is not None:
        # The large sheet owns every key but the window's ⌘ shortcuts.
        return large or _opens_commands(view, event)
    if event.key() == Qt.Key_Escape:
        return True  # Reserve it for the chrome and viewport Escape router.
    if _reveal_shortcut(event) or _problem_key(view, event):
        return True
    if _opens_commands(view, event):
        return True
    name = _KEYS.get(event.key(), "")
    if view.renderer.ui_claims_key(name):
        return True
    from views import wgpu_sheets
    return wgpu_sheets.claims_key(view, event)


def key(view, event, pressed: bool) -> bool:
    """Offer a key to the chrome first. True: it was the chrome's."""
    if not getattr(view, "_ui_attached", False):
        return False
    if _opens_commands(view, event):
        if pressed:
            view.renderer.ui_open_commands()
            view.update()
        return True
    from views import chrome_sheet
    if chrome_sheet.claims_key(view, event) is False:
        return False
    from views import wgpu_sheets
    sheets = wgpu_sheets.shown(view)
    if not sheets and _reveal_shortcut(event):
        if pressed:
            _tree_action(view.window(), view, "revealselection" + SEP)
        return True
    step = 0 if sheets else _problem_key(view, event)
    if step:
        if pressed:
            view.renderer.ui_step_issue(step)
            view.update()
        return True
    name = _KEYS.get(event.key(), "")
    if not view.renderer.ui_claims_key(name):
        # The Sheets workspace: the page's keys go to the composer.
        return wgpu_sheets.key(view, event, pressed) if sheets else False
    view.renderer.ui_key(name, pressed, _mods(event))
    text = event.text()
    if pressed and text and text.isprintable():
        view.renderer.ui_text(text)
    view.update()
    return True


def _mods(event) -> int:
    m = event.modifiers()
    return ((1 if m & Qt.AltModifier else 0) | (2 if m & Qt.MetaModifier else 0)
            | (4 if m & Qt.ShiftModifier else 0) | (8 if m & Qt.ControlModifier else 0))


_BUTTONS = {Qt.LeftButton: 1, Qt.RightButton: 2, Qt.MiddleButton: 3}


def title_double_click(view, event) -> bool:
    """A double-click on the top strip zooms the window, as on a title bar."""
    if not getattr(view, "_ui_attached", False):
        return False
    if not view.renderer.ui_top_empty(float(event.position().x()), float(event.position().y())):
        return False
    chrome = getattr(view.window(), "_mac_chrome", None)
    glass = getattr(chrome, "_glass", None)
    if glass is not None:
        glass.zoom()
        return True
    return False


def wheel(view, event) -> bool:
    if not getattr(view, "_ui_attached", False):
        return False
    from views import chrome_dialogs
    if chrome_dialogs.modal(_window(view)):
        return True
    p = event.position()
    d = event.pixelDelta()
    if d.isNull():
        a = event.angleDelta()
        dx, dy = a.x() / 8.0 * 3, a.y() / 8.0 * 3
    else:
        dx, dy = float(d.x()), float(d.y())
    taken = view.renderer.ui_wheel(float(p.x()), float(p.y()), dx, dy)
    if taken:
        view.update()
        return True
    from views import wgpu_sheets
    return wgpu_sheets.wheel(view, event)


def _held(view, kind, event) -> bool:
    """Exclusive work holds the window: only its chip (Cancel) and the
    window's own drag take the pointer; the model and the rest of the
    chrome see nothing until it ends."""
    from views import chrome_dialogs
    win = _window(view)
    if win is None or not chrome_dialogs.exclusive(win) or chrome_dialogs.current_sheet(win) is not None:
        return False
    if event is None:
        return False
    p = event.position()
    rect = view.renderer.ui_area_rect("jobs")
    if rect is not None:
        x, y, w, h = rect
        if x <= p.x() <= x + w and y <= p.y() <= y + h:
            return False        # the chip's own: Cancel
    if kind == "press" and event.button() == Qt.LeftButton \
            and view.renderer.ui_top_empty(float(p.x()), float(p.y())):
        return False            # the window still moves
    view.renderer.ui_pointer("leave", -1.0, -1.0, 0, 0)
    return True


def pointer(view, kind: str, event=None) -> bool:
    """Offer a pointer event to the chrome first. True: it was the chrome's
    (or, in the Sheets workspace, the page's)."""
    if not getattr(view, "_ui_attached", False):
        return False
    from views import wgpu_sheets
    if _held(view, kind, event):
        return True
    if event is None:
        taken = view.renderer.ui_pointer(kind, -1.0, -1.0, 0, 0)
    elif kind == "press" and event.button() == Qt.LeftButton \
            and view.renderer.ui_top_empty(float(event.position().x()), float(event.position().y())):
        # The empty top strip is the window's title bar: drag moves it.
        handle = view.window().windowHandle()
        if handle is not None:
            handle.startSystemMove()
        return True
    else:
        p = event.position()
        taken = view.renderer.ui_pointer(kind, float(p.x()), float(p.y()),
                                         _BUTTONS.get(event.button(), 1), _mods(event))
    if taken or kind != "move" or getattr(view, "_ui_was_over", False):
        view.update()  # hover highlight in, or out
    if kind in ("move", "leave"):
        was = getattr(view, "_ui_was_over", False)
        shape = view.renderer.ui_cursor_shape() if taken and hasattr(view.renderer, "ui_cursor_shape") else ""
        if shape == "resize_h":
            view.setCursor(Qt.SizeHorCursor)
        elif taken and (not was or getattr(view, "_ui_cursor_shape", "")):
            view.setCursor(Qt.ArrowCursor)
        elif was and not taken and not wgpu_sheets.shown(view):
            view.tools._apply_tool_cursor()
        view._ui_was_over = taken
        view._ui_cursor_shape = shape
    if wgpu_sheets.shown(view):
        # The Sheets workspace: what the chrome leaves is the page's.
        if taken:
            wgpu_sheets.chrome_took(view)
            return True
        return wgpu_sheets.pointer(view, kind, event)
    return taken
