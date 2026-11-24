"""Sheets: a workspace of the Built window, not a window of its own.

* **The page** is native (docs/rust-sheets.md): ``ingetrazo_core`` reads the
  sheet records out of the document (``sheet_page_json``) and the renderer
  paints them under the chrome (``ingetrazo_render`` ui/page.rs), with
  zoom, pan, picking, moving and resizing items and placing a view. Frames
  show the vector pass (the Rust HLR drawing, ``hlr_cache``) as line
  sets; the other styles are rendered by the renderer straight into a page
  texture (views/wgpu_sheet_frames.py: the camera and visibility from
  Rust, the live scene untouched). No QPainter, no read-back.
* **Edits** come back as actions (``sheets:rects``, ``sheets:place``,
  ``sheets:select``), one per gesture, and run through the sheet's history
  (``EditItemCommand``, ``place_tool``), so undo, auto-render and the dock
  stay the composer's.
* **The chrome** is the sheet's while the workspace is on (ui/sheets.rs):
  the sheet tools on the rail, the sheets and the selected item's fields in
  the dock, paper, scale, tool hint and zoom in the bottom bar. The fields
  mirror the composer's own panel widgets, so every edit goes through the
  slot the Qt panel uses (undo, re-render, cache invalidation).

The composer (views/composer.py) is still constructed, never shown, for the
dock, the frame renders, the history and the exports. What the native page
does not do yet (inline text, dimension and drawing tools, images, context
menus, guides) opens in the classic sheet editor: the Qt window in the Qt
app, the classic app from the shell. docs/python-glue.md lists this adapter.
"""
from __future__ import annotations

import re
from pathlib import Path

from PySide6.QtCore import QEvent, Qt
from PySide6.QtGui import QKeySequence

SEP = "\x1f"
ED = "\x1d"

#: Tests force the workspace on without a Metal frame.
FORCE: bool | None = None

#: The rail: select and navigate | sheet items | drawing | dimensions.
RAIL = [["select", "pan", "zoom", "zoom_ventana", "estilo"],
        ["vista", "texto", "etiqueta", "nivel", "llamada", "imagen", "cajetin",
         "escala", "norte", "leyenda", "perfil"],
        ["linea", "flecha", "terreno", "rect", "elipse", "poligono"],
        ["cota", "cota_cadena", "cota_base", "cota_ang", "cota_radio"]]
#: The tools the native page runs (ui/page.rs); the others open the
#: classic sheet editor with that tool.
NATIVE_TOOLS = {"select", "pan", "zoom", "zoom_ventana", "vista", "texto", "llamada", "cajetin", "escala",
                "norte", "leyenda", "perfil", "linea", "flecha", "terreno", "rect", "elipse", "cota",
                "cota_cadena", "cota_base", "cota_ang", "cota_radio", "poligono", "etiqueta", "nivel",
                "imagen", "estilo"}
#: The tools that snap to the frames' model (ui/sheet_tools.rs PICK_TOOLS).
SNAP_TOOLS = {"cota", "cota_cadena", "cota_base", "cota_ang", "cota_radio", "linea", "flecha", "terreno",
              "rect", "elipse", "poligono", "etiqueta", "nivel"}
#: Qt cursors for the page's cursor names.
_CURSORS = {"": Qt.ArrowCursor, "move": Qt.SizeAllCursor, "grab": Qt.OpenHandCursor,
            "grabbing": Qt.ClosedHandCursor, "crosshair": Qt.CrossCursor,
            "zoom": Qt.CrossCursor, "resize_h": Qt.SizeHorCursor, "resize_v": Qt.SizeVerCursor,
            "resize_nwse": Qt.SizeFDiagCursor, "resize_nesw": Qt.SizeBDiagCursor}

#: A frame's fields its content depends on (the rest only moves, titles or
#: decorates it): a change, from the page or an undo, works it out again.
_CONTENT_FIELDS = ("w_mm", "h_mm", "scale_n", "view_key", "style", "paper_bg", "cam_target",
                   "cam_yaw", "cam_pitch", "perspective", "cam_distance", "cam_fov", "shadows",
                   "sun_hour", "rot_deg", "pen_cut_mm", "pen_profile_mm", "pen_edge_mm", "profiles",
                   "hidden_lines", "layer_colours", "cut_fill", "cut_fill_color", "cut_fill_by_layer")

#: The fields of a frame its model annotations follow besides its content.
_ANNOT_FIELDS = ("annotations", "annot_text_mm", "km_marks", "km_step_m", "section_marks")

#: Keys that stay the window's in the workspace (save, open, new, close,
#: quit, preferences): everything else is the page's.
_WINDOW_KEYS = ("Ctrl+S", "Ctrl+Shift+S", "Ctrl+O", "Ctrl+N", "Ctrl+W", "Ctrl+Q",
                "Ctrl+,", "Ctrl+M", "Ctrl+H")


# ---- Where the workspace can be ----------------------------------------------
def _qt_window(win):
    """The QMainWindow behind ``win`` (a DocumentHost answers for one), or None."""
    from PySide6.QtWidgets import QWidget
    real = getattr(win, "_host_window", win) if win is not None else None
    return real if isinstance(real, QWidget) else None


def is_shell(win) -> bool:
    """The native ``built`` shell's window (app_shell/window.py): no widget."""
    return win is not None and _qt_window(win) is None


def embeds(win) -> bool:
    """Whether ``win``'s sheets open as the workspace (else the Qt window)."""
    if FORCE is not None:
        return FORCE
    if is_shell(win):
        return True
    from PySide6.QtGui import QGuiApplication
    if QGuiApplication.platformName() in ("offscreen", "minimal"):
        return False
    doc = getattr(win, "_doc", None)
    return bool(getattr(doc, "_renderer_wgpu", False))


#: The rail's tools: their icon (views/icons.py) and their name.
TOOL_ICONS = {"select": "select", "pan": "pan", "zoom": "zoom", "zoom_ventana": "zoom_window",
              "estilo": "eyedropper", "vista": "comp_vista", "texto": "text", "etiqueta": "comp_etiqueta",
              "nivel": "comp_nivel", "llamada": "comp_llamada", "imagen": "image", "cajetin": "comp_cajetin",
              "escala": "comp_escala", "norte": "comp_norte", "leyenda": "comp_leyenda", "perfil": "comp_perfil",
              "linea": "line", "flecha": "comp_flecha", "terreno": "comp_terreno", "rect": "rectangle",
              "elipse": "circle", "poligono": "polygon", "cota": "dimension", "cota_cadena": "dimension_chain",
              "cota_base": "dimension_baseline", "cota_ang": "dimension_angular",
              "cota_radio": "dimension_radius"}
TOOL_NAMES = {"select": "Select", "pan": "Pan", "zoom": "Zoom", "zoom_ventana": "Zoom window",
              "estilo": "Format painter", "vista": "Model view", "texto": "Text", "etiqueta": "Label",
              "nivel": "Level mark", "llamada": "Detail callout", "imagen": "Image", "cajetin": "Title block",
              "escala": "Scale bar", "norte": "North arrow", "leyenda": "Layer legend",
              "perfil": "Terrain profile", "linea": "Line", "flecha": "Arrow", "terreno": "Ground line",
              "rect": "Rectangle", "elipse": "Ellipse", "poligono": "Polygon", "cota": "Dimension",
              "cota_cadena": "Chain dimension", "cota_base": "Baseline dimension",
              "cota_ang": "Angular dimension", "cota_radio": "Radius dimension"}


class SheetHost:
    """A document's sheets as the window holds them (``_ensure_composer``,
    ``doc.composer``): the open sheet, the armed tool and whether the views
    render on their own; showing it opens the workspace and hiding it
    leaves. Not a window: the page, the dock and the exports are the
    workspace's (and Rust's)."""

    EMBEDDED = True

    def __init__(self, main_window):
        from core.settings import settings
        self._window = main_window
        self.comp = None
        self.tool_mode = "select"
        self._auto_render = str(settings().value("composer/auto_render", "1")) != "0"
        self._ws = Workspace(self)

    # The window's calls on it (MainWindow: open a sheet, switch documents, close).
    def setVisible(self, on):  # noqa: N802
        self._ws.enter() if on else self._ws.leave()

    def show(self):
        self.setVisible(True)

    def hide(self):
        self.setVisible(False)

    def isVisible(self):  # noqa: N802
        return bool(self._ws.shown)

    def isMinimized(self):  # noqa: N802
        return False

    def showMinimized(self):  # noqa: N802
        pass

    def raise_(self):
        pass

    def activateWindow(self):  # noqa: N802
        pass

    def set_document_title(self, name) -> None:
        pass

    def _covers(self, other):
        return True

    def close(self):
        self._ws.leave()
        self._ws.detach()
        return True

    def _show_model(self):
        self.hide()
        self._refresh_sheet_tabs()

    # ---- The document's sheets ----------------------------------------------------
    def _scene(self):
        return self._window.viewport.scene

    def _sheets(self):
        return self._scene().compositions

    def _adopt_document(self) -> None:
        """Coming on screen: the document's sheets are the workspace's (a
        document without any gets its first one)."""
        from core.composition import Composicion
        from core.i18n import tr
        scene = self._scene()
        if not scene.compositions:
            comp = Composicion(name=tr("Sheet {n}", n=1))
            comp.frames.append(comp.default_frame())
            scene.compositions.append(comp)
        if self.comp not in scene.compositions:
            self.comp = scene.compositions[0]

    def _on_comp_switched(self, index: int) -> None:
        comps = self._sheets()
        if 0 <= index < len(comps) and comps[index] is not self.comp:
            self.comp = comps[index]
            self._ws.sheet_changed()
        self._refresh_sheet_tabs()

    def show_sheet(self, index: int) -> None:
        """Sheet ``index`` in the workspace (a tab or the list, from either app)."""
        self._on_comp_switched(index)
        self.show()

    def step_sheet(self, step: int) -> None:
        """The sheet ``step`` places before or after this one — no wrap-around."""
        comps = self._sheets()
        if self.comp in comps and 0 <= comps.index(self.comp) + step < len(comps):
            self.show_sheet(comps.index(self.comp) + step)

    def _reload_comp_combo(self) -> None:
        """Sheets added, renamed, moved or deleted: the lists show it."""
        self._ws.sheet_changed()
        self._refresh_sheet_tabs()

    def _refresh_sheet_tabs(self) -> None:
        comps = self._sheets()
        if self.comp in comps:
            win = getattr(self._window, "_host_window", self._window)
            win._last_sheet = comps.index(self.comp)
        sync = getattr(self._window, "_refresh_sheet_tabs", None)
        if sync is not None:
            sync()

    def _on_comp_add(self) -> None:
        self._ws._add_sheet()

    def _new_sheet_tab(self) -> None:
        self._on_comp_add()
        self.show()

    def _on_model_version(self, version) -> None:
        """The model changed (the window tells, as it paints a new version)."""
        self._ws.model_changed(version)

    def frame_is_perspective(self, frame) -> bool:
        return bool(getattr(frame, "perspective", False))

    def export_all_pdf(self, path: str, sheets=None) -> list:
        """Sheets ``sheets`` (all by default) as one PDF (sheets::pdf)."""
        from views import wgpu_sheet_frames
        view = self._ws.view or self._ws._front_view()
        comps = list(self._sheets()) if sheets is None else list(sheets)
        return wgpu_sheet_frames.export_pdf(view, comps, path, wgpu_sheet_frames.Ground(lambda: None),
                                            getattr(self._window, "_current_path", None),
                                            getattr(self._window, "_ext_sheet_annots", None))


def composer_class():
    """What ``MainWindow._ensure_composer`` makes for a document: its sheets."""
    return SheetHost


def composer_for(win):
    """The composer of ``win``'s front document, made on first use
    (MainWindow._ensure_composer; the shell borrows it)."""
    win = getattr(win, "_host_window", win)
    return win._ensure_composer()


def workspace_of(view):
    ws = view.__dict__.get("_sheets_ws") if view is not None else None
    return ws if ws is not None and ws.shown and ws.view is view else None


def shown(view) -> bool:
    return workspace_of(view) is not None


def to_sheets(win, index=None) -> None:
    """The workspace ▾ chose Sheets (the native shell's _to_sheets; the Qt
    app gets here through MainWindow._to_sheets → the composer's show)."""
    win = getattr(win, "_host_window", win)
    comp = composer_for(win)
    comps = list(getattr(win.viewport.scene, "compositions", None) or [])
    if index is None and comps:
        index = min(max(getattr(win, "_last_sheet", 0) or 0, 0), len(comps) - 1)
    comp.show()
    if index is not None:
        comp.show_sheet(index)


def to_model(win) -> None:
    """The workspace ▾ chose Model (or ⌃Tab)."""
    win = getattr(win, "_host_window", win)
    qt = _qt_window(win)
    if qt is not None and hasattr(qt, "_show_model"):
        qt._show_model()
        return
    comp = getattr(win, "_composer", None)
    if comp is not None and getattr(comp, "EMBEDDED", False):
        comp.hide()


def _cajetin_template():
    """Where the title block templates live and the one a new title block
    starts from (the composer's ``cajetin_templates_dir`` and
    ``default_cajetin_template_name``): ``(folder, name or None)``."""
    from PySide6.QtCore import QSettings, QStandardPaths
    base = (QStandardPaths.writableLocation(QStandardPaths.AppDataLocation)
            or str(Path.home() / ".ingetrazo"))
    name = str(QSettings().value("composer/default_cajetin", "") or "") or None
    return str(Path(base) / "cajetines"), name


def _sheet_template():
    """The template file a new sheet starts from (the composer's
    ``default_template_name`` in its ``templates_dir``), or None."""
    from PySide6.QtCore import QStandardPaths
    from core.settings import settings
    name = str(settings().value("composer/default_template", "") or "")
    if not name:
        return None
    base = (QStandardPaths.writableLocation(QStandardPaths.AppDataLocation)
            or str(Path.home() / ".ingetrazo"))
    return str(Path(base) / "plantillas" / f"{name}.json")


def _int(v) -> int:
    """A Qt enum or flag (or the shell's plain int) as an int."""
    return int(getattr(v, "value", v))


def _alive(obj) -> bool:
    try:
        from shiboken6 import isValid
        return obj is not None and isValid(obj) and not getattr(obj, "_closed", False)
    except Exception:  # noqa: BLE001
        return False


class _Act:
    """What ``wgpu_ui._trigger`` runs for a cluster or View ▾ item."""

    def __init__(self, fn):
        self.trigger = fn


# ---- The workspace ---------------------------------------------------------------
class Workspace:
    def __init__(self, composer):
        import ingetrazo_core as NATIVE
        self.composer = composer
        NATIVE.sheet_forget_looks()       # a new session: the saved dimension look only (sheets::looks)
        NATIVE.sheet_restore_looks()
        self.shown = False
        self.view = None
        self._sent = {}
        self._page = None               # the snapshot the page has
        self._content = {}              # frame key → the fields its content was made from
        self._ground = None             # the samplers terrain profiles read
        self._edit_after = None         # a new text block, opened for typing once on the page
        self._snapped = {}              # frame key → what its snaps were worked out from
        self._annotated = {}            # frame key → what its annotations were worked out from
        self._followed = None           # what bound items last followed
        self._anchor_memo = {}          # (item key, attr) → (point, seen on an edge)
        self._holds = {}                # (item key, attr) → session hold on a vertex
        self._tried = {}                # frame key → the page a render was tried on
        self._more = False              # frames still to render after this turn
        self._message = ("", 0.0)       # a passing message and until when it shows
        self._own_versions = set()      # the scene versions the sheet's own edits made
        self._model_version = None      # the model version the views were last worked out for
        self._stale = False             # the model changed since: auto-render re-renders
        self._selection = None          # the keys the page and the composer agree on
        self._pending = None            # the dock question's subject
        self._prompt = None             # (title, body, field, buttons) to send
        self._actions = []
        self._shell = is_shell(composer._window)
        self._keys = self._shortcuts()

    # ---- In and out ------------------------------------------------------------
    def _front_view(self):
        win = self.composer._window
        doc = getattr(win, "_host_doc", None) or getattr(win, "_doc", None)
        return getattr(doc, "stand_in", None)

    def enter(self) -> None:
        view = self._front_view()
        if self.shown and view is self.view:
            self._touch()
            return
        if self.shown:
            self.leave()
        if view is None or not _alive(view):
            return
        self.shown = True
        self.view = view
        view.__dict__["_sheets_ws"] = self
        tools = getattr(view, "tools", None)
        if tools is not None and hasattr(tools, "cancel"):
            try:
                tools.cancel()          # a half-drawn model line does not wait under the page
            except Exception:  # noqa: BLE001
                pass
        self.composer._adopt_document()
        self._auto_render_stale()
        self._sent = {}
        self._page = None
        self._content = {}
        self._snapped = {}
        self._annotated = {}
        self._tried = {}
        self._selection = None
        renderer = getattr(view, "renderer", None)
        if renderer is not None and hasattr(renderer, "ui_show_sheets"):
            renderer.ui_show_sheets(True)
        self._reset_chrome(view)
        self._touch()

    def leave(self) -> None:
        if not self.shown:
            return
        self.shown = False
        view, self.view = self.view, None
        if view is not None and _alive(view):
            view.__dict__.pop("_sheets_ws", None)
            renderer = getattr(view, "renderer", None)
            if renderer is not None and hasattr(renderer, "ui_show_sheets"):
                renderer.ui_show_sheets(False)
                renderer.page_clear()
            self._reset_chrome(view)
            tools = getattr(view, "tools", None)
            if tools is not None and hasattr(tools, "_apply_tool_cursor"):
                try:
                    tools._apply_tool_cursor()
                except Exception:  # noqa: BLE001
                    pass
            view.update()

    def detach(self) -> None:
        """The composer is closing: nothing of it may be reached again."""
        self.leave()

    @staticmethod
    def _reset_chrome(view) -> None:
        """The chrome's cached state goes: the other workspace sends its own."""
        for name in ("_ui_tabs", "_ui_cluster_sig", "_ui_opts", "_ui_sheet_sig", "_ui_jobs_sig"):
            view.__dict__[name] = None if name != "_ui_sheet_sig" else False

    def _touch(self) -> None:
        view = self.view
        if view is not None and _alive(view):
            view.update()

    def flash(self, text: str, msec: int = 4000) -> None:
        """A passing message in the bottom bar for ``msec``."""
        import time
        self._message = (text, time.monotonic() + msec / 1000.0)
        self._touch()

    def _passing(self) -> str:
        import time
        text, until = self._message
        return text if time.monotonic() < until else ""

    # ---- The page (ingetrazo_core sheets::page → ingetrazo_render ui/page.rs) ------
    def _send_page(self, renderer) -> None:
        import ingetrazo_core as NATIVE
        comp = self.composer
        # Terrain profiles: the ground under their path (sheets::profile).
        if self._ground is None:
            from views.wgpu_sheet_frames import Ground
            self._ground = Ground(self._touch)
        profiles = self._ground.profiles(comp.comp, comp._scene())
        page = NATIVE.sheet_page_json(comp.comp, profiles, self._choices())
        if page != self._page:
            self._page = page
            renderer.page_set(page)
        key, self._edit_after = self._edit_after, None
        if key is not None:
            renderer.page_edit_text(key)

    def _choices(self) -> dict:
        """The lists the inspector offers (ingetrazo_render sheet_inspector.rs)."""
        from core.style import BUILTIN_STYLES, user_styles
        from core.composition import STD_VIEWS as _STD_VIEWS
        from core.i18n import tr
        scene = self.composer._scene()
        views = [["__current__", tr("Current view")]]
        views += [[f"std:{key}", tr(label)] for label, key in _STD_VIEWS]
        views += [[f"scene:{v.name}", tr("Scene: {name}", name=v.name)] for v in scene.saved_views]
        styles = [["sombreado", tr("Model style")]]
        styles += [[f"style:{p.name}", tr(p.name)] for p in BUILTIN_STYLES]
        styles += [[f"style:{p.name}", p.name] for p in user_styles()]
        styles += [["vectorial", tr("Vector (hidden lines removed)")]]
        return {"views": views, "styles": styles}

    def _send_contents(self, renderer) -> None:
        """Each frame's content (views/wgpu_sheet_frames.py): a shaded
        frame rendered straight into its page texture, a vector frame's
        hidden-line drawing as line sets (paper mm in the frame). A frame
        the page holds nothing for is done, one per turn."""
        import ingetrazo_core as NATIVE
        from views import wgpu_sheet_frames
        comp = self.composer
        serial = self._page                # tried again once the sheet changed
        rendered = False
        self._more = False
        for frame in list(comp.comp.frames):
            key = frame._doc_key
            sig = tuple(repr(getattr(frame, f, None)) for f in _CONTENT_FIELDS)
            if self._content.get(key, sig) != sig:  # the window on the model changed (undo too)
                renderer.page_frame_forget(key)
                self._tried.pop(key, None)
                self._annotated.pop(key, None)
            self._content[key] = sig
            if renderer.page_has_content(key) or self._tried.get(key) == serial:
                continue
            if rendered:
                self._more = True              # the next turn draws it
                continue
            self._tried[key] = serial
            rendered = True
            try:
                if frame.style == "vectorial" and not comp.frame_is_perspective(frame):
                    got = wgpu_sheet_frames.drawing(self.view.source_viewport, frame)
                    if got is not None:
                        # Its drawing as the sheet inks it (sheets::items_d).
                        renderer.page_frame_ops(key, NATIVE.sheet_frame_display(frame, got, True))
                else:
                    wgpu_sheet_frames.render(self.view, frame, key)
            except Exception:  # noqa: BLE001 — a view that cannot draw stays empty
                import traceback
                traceback.print_exc()

    def _models(self) -> dict:
        comp = self.composer.comp
        out = {m._doc_key: m for m in comp.all_items()}
        out[comp._doc_key] = comp
        return out

    def _selected_models(self) -> list:
        renderer = getattr(self.view, "renderer", None)
        keys = renderer.page_selection() if renderer is not None else []
        models = self._models()
        return [models[k] for k in keys if k in models]

    def _run(self, inner) -> None:
        """A sheet edit as one step of the document's history."""
        from core.history import SheetCommand
        self._execute(SheetCommand(inner))

    def _forget_render(self, model) -> None:
        """A frame whose window on the model changed: worked out again."""
        from core.composition import MarcoVista
        if isinstance(model, MarcoVista):
            self._content.pop(model._doc_key, None)
            self._tried.pop(model._doc_key, None)
            self._annotated.pop(model._doc_key, None)
            renderer = getattr(self.view, "renderer", None)
            if renderer is not None:
                renderer.page_frame_forget(model._doc_key)

    def _refresh_all(self) -> None:
        """Update all views: every frame worked out again from the model."""
        self._stale = False
        for frame in list(self.composer.comp.frames):
            self._forget_render(frame)
        self._touch()

    def _edit(self, edits) -> None:
        """``[(model, {field: value})]`` from the page or the inspector."""
        from core.composition import CompoundCommand, EditItemCommand
        commands = []
        for model, changes in edits:
            changes = {k: (tuple(v) if k == "cam_target" and v is not None else v)
                       for k, v in changes.items() if k in model._record_fields}
            if changes:
                commands.append(EditItemCommand(model, changes))
                self._forget_render(model)
        if commands:
            import ingetrazo_core as NATIVE
            self._run(commands[0] if len(commands) == 1 else CompoundCommand(commands))
            for c in commands:                # the next one of its kind is born with this look
                NATIVE.sheet_remember_look(c.item, list(c.after))

    def _move(self, arg: str) -> None:
        """``key:x:y:w:h;…`` from a gesture on the page."""
        models = self._models()
        edits = []
        for part in filter(None, arg.split(";")):
            key, x, y, w, h = part.split(":")
            model = models.get(int(key))
            if model is None or getattr(model, "locked", False):
                continue
            new = {"x_mm": float(x), "y_mm": float(y), "w_mm": float(w), "h_mm": float(h)}
            edits.append((model, {k: v for k, v in new.items()
                                  if k in model._record_fields and abs(getattr(model, k) - v) > 1e-9}))
        self._edit(edits)

    def _delete(self) -> None:
        from core.composition import CompoundCommand, RemoveItemCommand
        comp = self.composer.comp
        doomed = [m for m in self._selected_models() if m is not comp and not getattr(m, "locked", False)]
        if doomed:
            self._run(CompoundCommand([RemoveItemCommand(comp, m) for m in doomed]))

    def _duplicate(self) -> None:
        import ingetrazo_core as NATIVE
        from core.composition import AddItemCommand, CompoundCommand
        comp = self.composer.comp
        copies = NATIVE.sheet_duplicates(comp, [m for m in self._selected_models() if m is not comp])
        if copies:
            self._run(CompoundCommand([AddItemCommand(comp, m) for m in copies]))
            self._page = None
            self.view.renderer.page_select([m._doc_key for m in copies])

    def _place(self, x0, y0, x1, y1, tool: str = "vista", hit=None) -> None:
        """A placement tool's click or drag: the item it makes there
        (sheets::create), one step of the history, then back to Select. A
        new text block opens for typing. ``hit``: the first point caught the
        model (label, level): the item is bound to it."""
        import ingetrazo_core as NATIVE
        from core.composition import AddItemCommand, EditItemCommand
        from views import wgpu_sheet_frames
        comp = self.composer.comp
        scene = self.composer._scene()
        frame = self._models().get(hit.get("frame")) if hit and hit.get("world") else None
        if tool == "etiqueta":
            target = self._label_at(x1, y1)
            if target is not None:
                # The text landed on a label: one more leader to it.
                leader = {"ax_mm": x0 - target.x_mm, "ay_mm": y0 - target.y_mm, "anchor_uid": "", "a_world": None}
                if frame is not None:
                    leader.update(anchor_uid=wgpu_sheet_frames.frame_uid(frame), a_world=list(hit["world"]))
                before = [dict(ld) for ld in (target.leaders or [])]
                self._run(EditItemCommand(target, {"leaders": before + [leader]}, {"leaders": before}))
                self._page = None
                self.view.renderer.page_select([target._doc_key])
                self._tool("select")
                return
        template = _cajetin_template() if tool == "cajetin" else (None, None)
        item = NATIVE.sheet_new_item(comp, tool, x0, y0, x1, y1, scene, *template)
        if item is None:
            return
        if tool == "nivel":
            host = self._frame_at(x0, y0)
            if host is not None:
                z = wgpu_sheet_frames.level_at(self.view.source_viewport, host, x0, y0)
                if z is not None:
                    item.z_m = z
        if tool in ("etiqueta", "nivel") and frame is not None:
            wgpu_sheet_frames.anchor(item, frame, scene, [("a_world", hit)])
            if tool == "nivel":
                item.x_mm, item.y_mm = (float(v) for v in hit["p"])
                if frame.view_key == "std:top":
                    item.symbol = "circle"
        self._run(AddItemCommand(comp, item))
        self._page = None
        self.view.renderer.page_select([item._doc_key])
        if tool == "texto":
            self._edit_after = item._doc_key
        self._tool("select")

    def _cota(self, body, total: bool = False) -> None:
        """A linear dimension the page measured (one of a chain's or a
        baseline's too, or a chain's ``total``): the record (sheets::create),
        bound to the model when both ends caught the same frame, one step of
        the history. The tool stays armed for the next."""
        import ingetrazo_core as NATIVE
        from core.composition import AddItemCommand
        from views import wgpu_sheet_frames
        comp = self.composer.comp
        scene = self.composer._scene()
        a, b = body["a"], body["b"]
        if total:
            item = NATIVE.sheet_new_chain_total(comp, tuple(a["p"]), tuple(b["p"]), float(body["sep"]),
                                                body.get("axis", ""), tuple(body["p1"]), tuple(body["p2"]), scene)
        else:
            item = NATIVE.sheet_new_cota(comp, tuple(a["p"]), tuple(b["p"]), float(body.get("sep", 0.0)),
                                         body.get("axis", ""), scene)
        frame = self._models().get(a.get("frame"))
        if frame is not None and a.get("frame") == b.get("frame") and a.get("world") and b.get("world"):
            wgpu_sheet_frames.anchor(item, frame, scene, [("a_world", a), ("b_world", b)])
            item.scale_n = frame.scale_n
        self._run(AddItemCommand(comp, item))
        self._page = None

    def _dimension(self, kind: str, body) -> None:
        """A radius / diameter or an angle the page measured: the record
        (sheets::create), one step of the history; the tool stays armed."""
        import ingetrazo_core as NATIVE
        from core.composition import AddItemCommand
        comp = self.composer.comp
        scene = self.composer._scene()
        if kind == "radial":
            item = NATIVE.sheet_new_radial(comp, tuple(body["c"]), tuple(body["p"]), body.get("kind", "radius"), scene)
        else:
            item = NATIVE.sheet_new_angular(comp, tuple(body["v"]), tuple(body["a"]), tuple(body["b"]),
                                            float(body["r"]))
        if item is None:
            return
        self._run(AddItemCommand(comp, item))
        self._page = None

    def _follow_model(self) -> None:
        """The sheet's items bound to the model follow it when the model or
        a frame's window on it changed (wgpu_sheet_frames.follow_anchors);
        derived state, never a step of the history."""
        from views import wgpu_sheet_frames
        comp = self.composer.comp
        scene = self.composer._scene()
        frames = tuple((f.uid, f.x_mm, f.y_mm, f.w_mm, f.h_mm, f.scale_n, f.view_key, f.cam_yaw, f.cam_pitch,
                        tuple(f.cam_target or ()), f.rot_deg, f.perspective) for f in comp.frames)
        sig = (comp._doc_key, getattr(scene, "version", None), frames)
        if sig == self._followed:
            return
        self._followed = sig
        try:
            wgpu_sheet_frames.follow_anchors(self.view.source_viewport, comp, self._anchor_memo, self._holds)
        except Exception:  # noqa: BLE001 — a broken projection never takes the page down
            import traceback
            traceback.print_exc()

    def _send_annots(self, renderer) -> None:
        """Each frame's model annotations over its content, whatever its
        style (wgpu_sheet_frames.annotations), one frame per turn, again
        when the model or the frame changed."""
        import ingetrazo_core as NATIVE
        from views import wgpu_sheet_frames
        comp = self.composer
        version = getattr(comp._scene(), "version", None)
        providers = getattr(comp._window, "_ext_sheet_annots", None)
        for frame in list(comp.comp.frames):
            key = frame._doc_key
            sig = (version, self._content.get(key), len(providers or ()),
                   tuple(repr(getattr(frame, f, None)) for f in _ANNOT_FIELDS))
            if self._annotated.get(key) == sig:
                continue
            self._annotated[key] = sig
            try:
                got = wgpu_sheet_frames.annotations(self.view.source_viewport, frame, providers)
                renderer.page_frame_annots(key, NATIVE.sheet_frame_annots(frame, got))
            except Exception:  # noqa: BLE001 — a frame that cannot annotate keeps its content
                import traceback
                traceback.print_exc()
            self._more = True
            return

    def _send_snaps(self, renderer) -> None:
        """What each frame offers a snapping tool (wgpu_sheet_frames.snaps),
        one frame per turn, again when the model or the frame changed."""
        from views import wgpu_sheet_frames
        comp = self.composer
        scene = comp._scene()
        for frame in list(comp.comp.frames):
            key = frame._doc_key
            sig = (getattr(scene, "version", None), self._content.get(key), frame.x_mm, frame.y_mm)
            if self._snapped.get(key) == sig:
                continue
            self._snapped[key] = sig
            try:
                got = wgpu_sheet_frames.snaps(self.view.source_viewport, frame)
            except Exception:  # noqa: BLE001 — a frame that cannot draw offers nothing
                import traceback
                traceback.print_exc()
                got = None
            import numpy as np
            if got is None:
                got = (np.empty((0, 2)), np.empty((0, 3)), np.empty((0, 4)), np.empty((0, 2, 3)), np.empty((0, 3)))
            renderer.page_frame_snaps(key, *(np.ascontiguousarray(a, float).reshape(-1).tolist() for a in got))
            self._more = True
            return

    def _label_at(self, x, y):
        """The label whose text block covers page point (x, y), the topmost."""
        import ingetrazo_core as NATIVE
        best = None
        for et in getattr(self.composer.comp, "etiquetas", None) or []:
            if getattr(et, "locked", False):
                continue
            w, h = NATIVE.sheet_item_size(et)
            if et.x_mm - 0.5 <= x <= et.x_mm + w + 0.5 and et.y_mm - 0.5 <= y <= et.y_mm + h + 0.5:
                if best is None or et.z >= best.z:
                    best = et
        return best

    def _frame_at(self, x, y):
        """The topmost frame under page point (x, y)."""
        return next((f for f in reversed(self.composer.comp.frames)
                     if f.x_mm <= x <= f.x_mm + f.w_mm and f.y_mm <= y <= f.y_mm + f.h_mm), None)

    def _add_image(self) -> None:
        """The image tool: a picture file, then the item at the margins
        (sheets::create), one step of the history; back to Select."""
        import ingetrazo_core as NATIVE
        from core.composition import AddItemCommand
        from core.i18n import tr
        from views.filedialogs import file_dialogs
        parent = _qt_window(self.composer._window)
        path, _ = file_dialogs.getOpenFileName(parent, tr("Choose image…"), "", tr("Images (*.png *.jpg *.jpeg)"))
        self._tool("select")
        if not path:
            return
        item = NATIVE.sheet_new_image(self.composer.comp, path)
        if item is None:
            self.flash("That file is not a picture")
            return
        self._run(AddItemCommand(self.composer.comp, item))
        self._page = None
        self.view.renderer.page_select([item._doc_key])

    def _tool(self, name: str) -> None:
        """Arm a tool of the rail (the page follows: ui_set_sheet_active)."""
        self.composer.tool_mode = name if name in NATIVE_TOOLS else "select"
        self._touch()

    def _font(self, model) -> None:
        """The font picker for a sheet text; the choice is one history step."""
        if model is None:
            return
        from views.font_picker import show

        def apply(p):
            self._edit([(model, dict(family=p["font"], bold=p["bold"], italic=p["italic"],
                                     size_pt=p["height"]))])
            self._touch()
        show(self.view.window(), "Sheet text", model.text, model.family, model.bold, model.italic,
             model.size_pt, " pt", apply)

    def _undo(self, redo: bool = False) -> None:
        win = self.composer._window
        win = getattr(win, "_host_window", win)
        (win._on_redo if redo else win._on_undo)()

    # ---- Fit and zoom ----------------------------------------------------------------
    def fit(self, width_only: bool = False) -> None:
        if self.view is not None:
            self.view.renderer.page_fit(bool(width_only))
            self._touch()

    def zoom_to(self, text: str) -> None:
        m = re.search(r"[-+]?\d+(?:[.,]\d+)?", text or "")
        if m and self.view is not None:
            self.view.renderer.page_zoom_to(float(m.group(0).replace(",", ".")))
            self._touch()

    # ---- One frame ---------------------------------------------------------------------
    def before_frame(self, view) -> None:
        renderer = view.renderer
        self._keep_a_sheet()
        self._follow_model()
        self._send_page(renderer)
        self._send_contents(renderer)
        if not self._more:
            self._send_annots(renderer)
        if self.composer.tool_mode in SNAP_TOOLS and not self._more:
            self._send_snaps(renderer)
            style = getattr(self.composer._scene(), "dimension_style", None) or {}
            try:
                step = float(style.get("base_step_mm", 8.0))
            except (TypeError, ValueError):
                step = 8.0
            self._once("run_step", step if step > 0.1 else 8.0, renderer.page_set_run_step)
        self._send(renderer, view)

    def _once(self, key, value, send) -> None:
        if self._sent.get(key, ...) != value:
            self._sent[key] = value
            send(value)

    def _send(self, renderer, view) -> None:
        comp = self.composer
        if "tools" not in self._sent:
            self._sent["tools"] = True
            renderer.ui_set_sheet_tools(self._tool_groups())
        mode = comp.tool_mode if comp.tool_mode in NATIVE_TOOLS else "select"
        renderer.ui_set_sheet_active(mode)
        self._once("list", self.sheet_rows(), renderer.ui_set_sheet_list)
        self._once("exports", self.exports(), renderer.ui_set_sheet_exports)
        self._once("status", self.status(), lambda v: renderer.ui_set_sheet_status(*v))
        prompt, self._prompt = self._prompt, None
        if prompt is not None:
            renderer.ui_set_sheet_prompt(*prompt)
        cluster, menu = self.cluster()
        sig = ("sheets", tuple(map(tuple, cluster)), tuple((t, tuple(r)) for t, r in menu))
        if sig != view.__dict__.get("_ui_cluster_sig"):
            view._ui_cluster_sig = sig
            renderer.ui_set_cluster(cluster, menu)

    def _tool_groups(self):
        from views import wgpu_ui
        groups = []
        for keys in RAIL:
            row = []
            for mode in keys:
                if mode not in TOOL_ICONS:
                    continue
                w, h, rgba = wgpu_ui._icon_rgba(TOOL_ICONS[mode])
                row.append((mode, TOOL_NAMES.get(mode, mode), "", w, h, rgba))
            if row:
                groups.append(row)
        return groups

    # ---- What the dock shows -------------------------------------------------------------
    def sheet_rows(self):
        comps = list(self.composer._scene().compositions)
        cur = self.composer.comp
        rows = []
        for composition in comps:
            paper = f"{composition.paper} {'▭' if composition.landscape else '▯'}"
            n = sum(len(getattr(composition, k, []) or []) for k in (
                "frames", "texts", "images", "scalebars", "nortes", "leyendas", "shapes",
                "cotas", "cotas_ang", "cotas_rad", "etiquetas", "perfiles", "niveles", "llamadas"))
            n += 1 if getattr(composition, "cajetin", None) is not None else 0
            rows.append((composition.name, f"{paper} · {n}", composition is cur))
        return tuple(rows)

    def _frame(self):
        """The one selected frame, or None."""
        from core.composition import MarcoVista
        sel = self._selected_models()
        return sel[0] if len(sel) == 1 and isinstance(sel[0], MarcoVista) else None

    def exports(self):
        frame = self._frame() is not None
        out = [("pdf", "PDF…", True), ("pdf_all", "All sheets…", True), ("png", "Image…", True),
               ("dxf", "View DXF…", frame), ("svg", "View SVG…", frame)]
        return tuple(out)

    def status(self):
        comp = self.composer
        c = comp.comp
        paper = f"{c.paper} {'landscape' if c.landscape else 'portrait'}"
        frame = self._frame() or (c.frames[0] if getattr(c, "frames", None) else None)
        scale = ""
        if frame is not None:
            from core.composition import format_scale
            scale = "No scale" if getattr(frame, "perspective", False) else format_scale(frame.scale_n)
        mode = comp.tool_mode if comp.tool_mode in NATIVE_TOOLS else "select"
        tool = TOOL_NAMES.get(mode, mode)
        hint = self._passing()                     # a passing message; the page says the rest
        renderer = getattr(self.view, "renderer", None)
        at = renderer.page_pointer_mm() if renderer is not None else None
        cursor = f"x: {at[0]:.1f} mm  y: {at[1]:.1f} mm" if at else ""
        zoom = f"{renderer.page_zoom():.0f}%" if renderer is not None else ""
        return (paper, scale, tool, hint, cursor, zoom)

    def cluster(self):
        """The top bar's right group and View ▾ while the workspace is on."""
        comp = self.composer
        acts = []

        def item(label, fn, tip="", icon="", checked=-1, enabled=True):
            acts.append(_Act(fn))
            return (str(len(acts) - 1), label, tip, icon, checked, enabled)
        cluster = [[item("Fit page", self.fit, "Fit the page in the window", "zoom_extents")]]
        menu = [("Zoom", [item("Fit page", self.fit), item("Fit width", lambda: self.fit(True)),
                          *[item(f"{p}%", lambda p=p: self.zoom_to(str(p))) for p in (50, 100, 200, 400)]]),
                ("Sheet", [item("Update all views", self._refresh_all),
                           item("Auto-render views", lambda: self.set_auto_render(not comp._auto_render),
                                checked=int(bool(comp._auto_render))),
                           item("Back to the model", lambda: to_model(comp._window))])]
        self._actions = acts
        return cluster, menu

    # ---- Actions from the chrome -------------------------------------------------------------
    def action(self, arg: str) -> None:
        verb, _, rest = arg.partition(SEP)
        comp = self.composer
        comps = comp._scene().compositions
        if verb == "tool":
            if rest == "imagen":
                self._add_image()
            elif rest in NATIVE_TOOLS:
                self._tool(rest)
        elif verb == "select":
            pass                         # the page keeps its selection; the dock reads it
        elif verb == "rects":
            self._move(rest)
        elif verb == "place":
            import json
            parts = rest.split(SEP)
            tool = parts[4] if len(parts) > 4 else comp.tool_mode
            hit = json.loads(parts[5]) if len(parts) > 5 else None
            self._place(*(float(v) for v in parts[:4]), tool=tool, hit=hit)
        elif verb in ("set", "sheet", "set_many"):
            import json
            models = self._models()
            if verb == "sheet":
                self._edit([(comp.comp, json.loads(rest))])
            else:
                parts = rest.split(SEP)
                self._edit([(models[int(k)], json.loads(v)) for k, v in zip(parts[::2], parts[1::2])
                            if int(k) in models])
        elif verb == "font":
            self._font(self._models().get(int(rest)))
        elif verb in ("radial", "angular"):
            import json
            self._dimension(verb, json.loads(rest))
        elif verb in ("cota", "cota_total"):
            import json
            self._cota(json.loads(rest), total=verb == "cota_total")
        elif verb == "refresh":
            model = self._models().get(int(rest))
            if model is not None:
                self._forget_render(model)
        elif verb == "open":
            comp.show_sheet(int(rest))
        elif verb == "add":
            self._add_sheet()
        elif verb == "dup":
            self._duplicate_sheet(int(rest))
        elif verb == "rename":
            i, _, name = rest.partition(SEP)
            name = name.strip()
            if 0 <= int(i) < len(comps) and name and comps[int(i)].name != name:
                self._edit([(comps[int(i)], {"name": name})])
                comp._reload_comp_combo()
        elif verb in ("up", "down"):
            i = int(rest)
            j = i + (-1 if verb == "up" else 1)
            if 0 <= i < len(comps) and 0 <= j < len(comps):
                from core.history import MoveSheetCommand
                self._execute(MoveSheetCommand(comps[i], j))
                comp._reload_comp_combo()
        elif verb == "del":
            i = int(rest)
            if 0 <= i < len(comps) and len(comps) > 1:
                self._pending = ("del", i)
                self._prompt = (f"Delete “{comps[i].name}”?",
                                "Its views, texts and dimensions go with it. Undo brings it back.",
                                None, [("del", "Delete"), ("cancel", "Cancel")])
        elif verb == "answer":
            self._answer(*rest.split(SEP, 1)) if SEP in rest else self._answer(rest, "")
        elif verb == "export":
            self.export(rest)
        elif verb == "zoom":
            self.zoom_to(rest)
        elif verb == "fit":
            self.fit()
        self._touch()

    def _answer(self, bid: str, text: str) -> None:
        pending, self._pending = self._pending, None
        if pending is None or not bid or bid == "cancel":
            return
        what, arg = pending
        if what == "del" and bid == "del":
            self._delete_sheet(arg)

    # ---- The sheet list, as steps of the document's history ------------------------------------
    def _sheets(self):
        return self.composer._scene().compositions

    def _history(self):
        return self.composer._window.viewport.history

    def _show(self, index: int) -> None:
        index = max(0, min(index, len(self._sheets()) - 1))
        self.composer._on_comp_switched(index)
        self.sheet_changed()

    def _add_sheet(self) -> None:
        """A new sheet at the end, from the default template when there is one."""
        from core.composition import Composicion
        from core.history import InsertSheetCommand
        from core.i18n import tr
        import ingetrazo_core as NATIVE
        sheets = self._sheets()
        template = _sheet_template()
        new = NATIVE.sheet_from_template(template) if template else None
        if new is None:
            new = Composicion()
            new.frames.append(new.default_frame())
        new.name = tr("Sheet {n}", n=len(sheets) + 1)
        self._execute(InsertSheetCommand(new))
        self._show(len(sheets) - 1)

    def _duplicate_sheet(self, index: int) -> None:
        from core.composition import Composicion
        from core.history import InsertSheetCommand
        from core.i18n import tr
        sheets = self._sheets()
        if not 0 <= index < len(sheets):
            return
        src = sheets[index]
        dup = Composicion.from_dict(src.to_dict())
        dup.name = src.name + tr(" (copy)")
        self._execute(InsertSheetCommand(dup, index + 1))
        self._show(index + 1)

    def _delete_sheet(self, index: int) -> None:
        from core.history import RemoveSheetCommand
        sheets = self._sheets()
        if not 0 <= index < len(sheets) or len(sheets) <= 1:
            return
        current = self.composer.comp is sheets[index]
        self._execute(RemoveSheetCommand(sheets[index]))
        if current:
            self._show(index)
        else:
            self.composer._reload_comp_combo()

    def sheet_changed(self) -> None:
        """Another sheet is open, or the list changed: the page and the dock
        are sent again."""
        self._page = None
        self._sent.pop("list", None)
        self._touch()

    def _execute(self, command) -> None:
        """A sheet command as a step of the document's history; the scene
        versions it makes (the window may report them while it runs) are
        the sheet's own, not model changes."""
        self._in_edit = True
        try:
            self._history().execute(command)
        finally:
            self._in_edit = False
            self._own_versions.add(getattr(self.composer._scene(), "version", None))
            if len(self._own_versions) > 256:
                self._own_versions = set(sorted(v for v in self._own_versions if v is not None)[-128:])

    def model_changed(self, version) -> None:
        """The model changed (not one of the sheet's own edits): with
        Auto-render on, the shaded views render again; a drawn vector view
        waits for Update (its exact pass costs seconds on a real model)."""
        if self.__dict__.get("_in_edit"):
            self._own_versions.add(version)
            return
        if version in self._own_versions or version == self._model_version:
            return
        self._model_version = version
        self._stale = True
        if self.shown:
            self._auto_render_stale()

    def _auto_render_stale(self) -> None:
        if not (self._stale and self.composer._auto_render):
            return
        self._stale = False
        for frame in list(self.composer.comp.frames):
            if frame.style != "vectorial" or self.composer.frame_is_perspective(frame):
                self._forget_render(frame)
        self._touch()

    def set_auto_render(self, on: bool) -> None:
        from core.settings import settings
        self.composer._auto_render = bool(on)
        settings().setValue("composer/auto_render", "1" if on else "0")
        self._auto_render_stale()

    def _keep_a_sheet(self) -> None:
        """An undo or redo took the open sheet away: its neighbour shows."""
        sheets = self._sheets()
        if self.composer.comp not in sheets and len(sheets):
            self._show(0)


    # ---- Exports ------------------------------------------------------------------------------
    _EXPORTS = {"pdf": ("Export PDF…", ".pdf", "PDF (*.pdf)"),
                "pdf_all": ("Export all sheets (PDF)…", ".pdf", "PDF (*.pdf)"),
                "png": ("Export image…", ".png", "PNG (*.png);;JPEG (*.jpg *.jpeg)"),
                "dxf": ("Export view as DXF…", ".dxf", "DXF (*.dxf)"),
                "svg": ("Export view as SVG…", ".svg", "SVG (*.svg)")}

    def _default_path(self, kind: str) -> str:
        comp = self.composer
        title, ext, _ = self._EXPORTS[kind]
        stem = comp.comp.name if kind != "pdf_all" else "Sheets"
        doc = getattr(comp._window, "_current_path", None)
        if doc:
            stem = f"{Path(str(doc)).stem} — {stem}"
            folder = Path(str(doc)).parent
        else:
            folder = Path.home() / "Desktop"
        stem = re.sub(r'[\\/:*?"<>|]+', "-", stem).strip() or "sheet"
        return str(folder / f"{stem}{ext}")

    def export(self, kind: str) -> None:
        if kind not in self._EXPORTS:
            return
        if kind in ("dxf", "svg") and self._frame() is None:
            self.flash("Select a model view on the sheet first")
            return
        title, ext, filters = self._EXPORTS[kind]
        from views.filedialogs import file_dialogs
        # The native save panel in both apps (the shell's is app_shell.files').
        parent = _qt_window(self.composer._window)
        path, _ = file_dialogs.getSaveFileName(parent, title, self._default_path(kind), filters)
        if path:
            if not Path(path).suffix:
                path += ext
            self._export_to(kind, path)

    def _export_to(self, kind: str, path: str) -> None:
        comp = self.composer
        from views import chrome_dialogs
        parent = _qt_window(comp._window) or comp._window
        labels = {"pdf": "Exporting PDF", "pdf_all": "Exporting the sheets", "png": "Exporting image",
                  "dxf": "Exporting DXF", "svg": "Exporting SVG"}
        name = Path(path).name
        troubled = []
        try:
            with chrome_dialogs.busy(parent, f"{labels[kind]} · {name}", output_path=path):
                if kind in ("pdf", "pdf_all"):
                    from views import wgpu_sheet_frames
                    comps = [comp.comp] if kind == "pdf" else list(comp._scene().compositions)
                    if self._ground is None:
                        self._ground = wgpu_sheet_frames.Ground(self._touch)
                    troubled = wgpu_sheet_frames.export_pdf(
                        self.view, comps, path, self._ground,
                        getattr(comp._window, "_current_path", None), getattr(comp._window, "_ext_sheet_annots", None))
                elif kind == "png":
                    from views import wgpu_sheet_frames
                    if self._ground is None:
                        self._ground = wgpu_sheet_frames.Ground(self._touch)
                    troubled = wgpu_sheet_frames.export_image(
                        self.view, comp.comp, path, 200.0, self._ground,
                        getattr(comp._window, "_current_path", None), getattr(comp._window, "_ext_sheet_annots", None))
                elif kind == "dxf":
                    from views import wgpu_sheet_frames
                    if wgpu_sheet_frames.export_dxf(self.view.source_viewport, self._frame(), path) is None:
                        troubled = [comp.comp.name]
                elif kind == "svg":
                    from views import wgpu_sheet_frames
                    if wgpu_sheet_frames.export_svg(self.view.source_viewport, self._frame(), path) is None:
                        troubled = [comp.comp.name]
        except Exception as exc:  # noqa: BLE001 — say it, never lose the workspace
            import traceback
            traceback.print_exc()
            self.flash(f"Export failed: {exc}", 8000)
            return
        self.last_export = path
        if troubled:
            self.flash(f"Exported {name} — a view would not render on {', '.join(troubled)}", 10000)
        else:
            self.flash(f"Exported {name}", 5000)

    # ---- Input: the page's (ingetrazo_render ui/page.rs) ---------------------------------
    def pointer(self, kind: str, event) -> None:
        from views import wgpu_ui
        view = self.view
        if view is None:
            return
        renderer = view.renderer
        if kind == "leave" or event is None:
            renderer.page_pointer("leave", -1.0, -1.0, 0, 0)
            return
        if kind == "press" and _int(event.type()) == _int(QEvent.MouseButtonDblClick):
            kind = "double"
        p = event.position()
        shape = renderer.page_pointer(kind, float(p.x()), float(p.y()),
                                      wgpu_ui._BUTTONS.get(event.button(), 1), wgpu_ui._mods(event))
        cursor = _CURSORS.get(shape, Qt.ArrowCursor)
        if view.__dict__.get("_sheets_cursor") != cursor:
            view._sheets_cursor = cursor
            view.setCursor(cursor)
        self._touch()

    def chrome_took(self) -> None:
        """The pointer went over the chrome: the page no longer has it."""
        if self.view is not None:
            self.view.renderer.page_pointer("leave", -1.0, -1.0, 0, 0)
            self.view.__dict__.pop("_sheets_cursor", None)

    def wheel(self, event) -> None:
        """Scrolling pans the page; with ⌘ it zooms about the pointer."""
        p = event.position()
        d = event.pixelDelta()
        if d.isNull():
            a = event.angleDelta()
            dx, dy = a.x() / 8.0 * 3, a.y() / 8.0 * 3
        else:
            dx, dy = float(d.x()), float(d.y())
        zoom = bool(event.modifiers() & Qt.ControlModifier)
        self.view.renderer.page_wheel(float(p.x()), float(p.y()), dx, dy, zoom)
        self._touch()

    def gesture(self, event) -> bool:
        kind = event.gestureType()
        p = event.position()
        if kind == Qt.ZoomNativeGesture:
            self.view.renderer.page_zoom_by(float(p.x()), float(p.y()), 1.0 + float(event.value()))
        elif kind == Qt.SmartZoomNativeGesture:
            self.fit()
        self._touch()
        return True

    def context_menu(self, event) -> None:
        self.flash("The page has no context menu yet: the dock and the shortcuts have its commands", 2500)

    def _shortcuts(self):
        comp = self.composer

        def seqs(*keys):
            out = []
            for k in keys:
                out.extend(QKeySequence.keyBindings(k) if isinstance(k, QKeySequence.StandardKey) else [QKeySequence(k)])
            return out

        def model():
            to_model(comp._window)

        def rename():
            comps = comp._scene().compositions
            view = self.view
            if comp.comp in comps and view is not None and getattr(view, "renderer", None) is not None:
                view.renderer.ui_rename_sheet(comps.index(comp.comp))
        def lock():
            sel = self._selected_models()
            on = not all(getattr(m, "locked", False) for m in sel)
            self._edit([(m, {"locked": on}) for m in sel if m is not comp.comp])
        # The document's history: what the page and the dock change, undone
        # in the order it was done, with the model's edits.
        return [(seqs(QKeySequence.Undo), self._undo),
                (seqs(QKeySequence.Redo, "Ctrl+Shift+Z"), lambda: self._undo(redo=True)),
                (seqs(QKeySequence.Delete, Qt.Key_Backspace, Qt.Key_Delete), self._delete),
                (seqs("Ctrl+D"), self._duplicate), (seqs("Ctrl+L"), lock),
                (seqs("Ctrl+G"), lambda: self.view.renderer.page_command("group")),
                (seqs("Ctrl+Shift+G"), lambda: self.view.renderer.page_command("ungroup")),
                (seqs("Ctrl+Shift+C"), lambda: self.view.renderer.page_command("copy_style")),
                (seqs("Ctrl+Shift+V"), lambda: self.view.renderer.page_command("paste_style")),
                (seqs("Ctrl+PgUp"), lambda: comp.step_sheet(-1)),
                (seqs("Ctrl+PgDown"), lambda: comp.step_sheet(1)),
                (seqs("Ctrl+Tab", "Meta+Tab", "Ctrl+Shift+Tab"), model),
                (seqs("F2"), rename)]

    @staticmethod
    def _combo(event):
        try:
            return QKeySequence(_int(event.modifiers()) | _int(event.key()))
        except Exception:  # noqa: BLE001
            return QKeySequence()

    @staticmethod
    def window_key(event) -> bool:
        combo = Workspace._combo(event)
        return any(QKeySequence(k).matches(combo) == QKeySequence.ExactMatch for k in _WINDOW_KEYS)

    def key(self, event, pressed: bool) -> None:
        """The composer's sheet shortcuts (undo, delete, duplicate, group…,
        on the selection the page and the dock share); arrows and Escape
        are the page's."""
        from views import wgpu_ui
        repeat = bool(event.isAutoRepeat()) if hasattr(event, "isAutoRepeat") else False
        if not pressed:
            return
        if not repeat:
            combo = self._combo(event)
            for keys, run in self._keys:
                if any(s.matches(combo) == QKeySequence.ExactMatch for s in keys):
                    run()
                    self._touch()
                    return
        name = wgpu_ui._KEYS.get(event.key(), "")
        if name and self.view.renderer.page_key(name, wgpu_ui._mods(event)):
            self._touch()

    # ---- The shell's loop -------------------------------------------------------------------
    def timers_pending(self) -> bool:
        """Frames still to render, or a passing message still showing: the
        shell pumps Qt only while something is due."""
        return self._more or bool(self._passing())


# ---- Hooks (views/wgpu_view.py, views/wgpu_ui.py) ---------------------------------------------
def frame(view):
    """A frame of the workspace: the chrome's window state, the page's
    snapshot and frame contents, then the page and the chrome."""
    import time
    from views import chrome_sheet, wgpu_start, wgpu_ui
    ws = workspace_of(view)
    renderer = view.renderer
    start = time.perf_counter()
    win = wgpu_ui._window(view)
    if win is not None and getattr(view, "_ui_attached", False):
        wgpu_ui._sheet_and_jobs(view, win)
        wgpu_start.before_frame(view, win)
        chrome_sheet.before_frame(view, win)
        if not view._ui_tools_sent and win._tool_actions:
            wgpu_ui._send_tools(view, win)
        wgpu_ui._projects(view, win)
        wgpu_ui._tabs(view, win)
    try:
        ws.before_frame(view)
    except Exception:  # noqa: BLE001 — a bad page frame must not take the window
        import traceback
        traceback.print_exc()
    cpu, gpu = renderer.render_page()
    wgpu_ui.after_frame(view)
    if ws.timers_pending():
        timer = getattr(view, "_ui_timer", None)
        if timer is not None and (not timer.isActive() or timer.remainingTime() > 100):
            timer.start(100)
    return cpu, gpu, (time.perf_counter() - start) * 1000


def pointer(view, kind, event) -> bool:
    ws = workspace_of(view)
    if ws is None:
        return False
    ws.pointer(kind, event)
    return True


def chrome_took(view) -> None:
    ws = workspace_of(view)
    if ws is not None:
        ws.chrome_took()


def wheel(view, event) -> bool:
    ws = workspace_of(view)
    if ws is None:
        return False
    ws.wheel(event)
    return True


def gesture(view, event) -> bool:
    ws = workspace_of(view)
    return ws.gesture(event) if ws is not None else False


def context_menu(view, event) -> bool:
    ws = workspace_of(view)
    if ws is None:
        return False
    ws.context_menu(event)
    return True


def claims_key(view, event) -> bool:
    """ShortcutOverride in the workspace: the page takes every key but the
    window's own few (save, open, new, close, quit, preferences)."""
    return shown(view) and not Workspace.window_key(event)


def key(view, event, pressed: bool) -> bool:
    ws = workspace_of(view)
    if ws is None or Workspace.window_key(event):
        return False
    ws.key(event, pressed)
    return True


def action(win, arg: str) -> None:
    """``sheets:<verb>\\x1f<arg>`` from ui/sheets.rs."""
    from views import wgpu_ui
    view = getattr(getattr(win, "_doc", None), "stand_in", None)
    ws = workspace_of(view)
    if ws is None:
        return
    try:
        ws.action(arg)
    except Exception:  # noqa: BLE001
        import traceback
        traceback.print_exc()
        ws.flash("That did not work — see the log")
    _ = wgpu_ui


def timers_pending(view) -> bool:
    ws = workspace_of(view)
    return ws is not None and ws.timers_pending()
