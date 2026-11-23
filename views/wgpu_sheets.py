"""Sheets: a workspace of the Built window, not a window of its own.

* **The page** is native (docs/rust-sheets.md): ``ingetrazo_core`` reads the
  sheet records out of the document (``sheet_page_json``) and the renderer
  paints them under the chrome (``ingetrazo_render`` ui/page.rs), with
  zoom, pan, picking, moving and resizing items and placing a view. Frames
  show the composer's frame content: the vector pass (the Rust HLR drawing,
  ``hlr_cache``) as line sets, the other styles as pictures
  (``render_cache``). No QPainter, no uploaded page image.
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
from PySide6.QtGui import QImage, QKeySequence

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
NATIVE_TOOLS = {"select", "pan", "zoom", "zoom_ventana", "vista"}
#: Qt cursors for the page's cursor names.
_CURSORS = {"": Qt.ArrowCursor, "move": Qt.SizeAllCursor, "grab": Qt.OpenHandCursor,
            "grabbing": Qt.ClosedHandCursor, "crosshair": Qt.CrossCursor,
            "zoom": Qt.CrossCursor, "resize_h": Qt.SizeHorCursor, "resize_v": Qt.SizeVerCursor,
            "resize_nwse": Qt.SizeFDiagCursor, "resize_nesw": Qt.SizeBDiagCursor}

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


_CLASS = None


def composer_class():
    """``ComposerWindow`` as the workspace's: never on screen; showing it
    opens the workspace and hiding it leaves (the main window's sheet
    calls — open a sheet, switch documents, close — stay as they were)."""
    global _CLASS
    if _CLASS is not None:
        return _CLASS
    from views.composer import ComposerWindow

    class SheetComposer(ComposerWindow):
        EMBEDDED = True

        def __init__(self, main_window):
            self._ws = None
            super().__init__(main_window)
            self._ws = Workspace(self)

        # A widget that is never shown: setVisible is the one way Qt
        # itself would show it, so it never reaches QWidget.
        def setVisible(self, on):  # noqa: N802
            ws = self.__dict__.get("_ws")
            if ws is not None:
                ws.enter() if on else ws.leave()

        def show(self):
            self.setVisible(True)

        def hide(self):
            self.setVisible(False)

        def isVisible(self):  # noqa: N802
            ws = self.__dict__.get("_ws")
            return bool(ws is not None and ws.shown)

        def isMinimized(self):  # noqa: N802
            return False

        def showMinimized(self):  # noqa: N802
            pass

        def raise_(self):
            pass

        def activateWindow(self):  # noqa: N802
            pass

        def _show_model(self):
            self.hide()
            sync = getattr(self._window, "_refresh_sheet_tabs", None) if not is_shell(self._window) else None
            if sync is not None:
                sync()

        def _covers(self, other):
            return True

        def _new_sheet_tab(self):
            self._on_comp_add()
            self.show()
            self._refresh_sheet_tabs()

        def close(self):
            ws = self.__dict__.get("_ws")
            if ws is not None:
                ws.leave()
                ws.detach()
            classic = self.__dict__.get("_classic")
            if classic is not None and _alive(classic):
                classic.close()
            return super().close()

    _CLASS = SheetComposer
    return _CLASS


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
        self.composer = composer
        self.shown = False
        self.view = None
        self._sent = {}
        self._page = None               # the snapshot the page has
        self._content = {}              # frame key → the cache entry it shows
        self._tried = {}                # frame key → the page a render was tried on
        self._more = False              # frames still to render after this turn
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
        self._sent = {}
        self._page = None
        self._content = {}
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
        self.composer.statusBar().showMessage(text, msec)
        self._touch()

    # ---- The page (ingetrazo_core sheets::page → ingetrazo_render ui/page.rs) ------
    def _send_page(self, renderer) -> None:
        import ingetrazo_core as NATIVE
        page = NATIVE.sheet_page_json(self.composer.comp)
        if page != self._page:
            self._page = page
            renderer.page_set(page)

    def _send_contents(self, renderer) -> None:
        """Each frame's content as the composer has it: the vector pass's
        lines (paper mm in the frame) or the render, sent when it changed. A
        frame without any is rendered, one per turn."""
        comp = self.composer
        serial = self._page                # tried again once the sheet changed
        rendered = False
        self._more = False
        for frame in list(comp.comp.frames):
            key = frame._doc_key
            fid = id(frame)
            lines = comp.hlr_cache.get(fid)
            image = comp.render_cache.get(fid)
            have = lines if lines is not None else image
            if have is None and self._tried.get(key) != serial and rendered:
                self._more = True              # the next turn renders it
            if have is None and not rendered and self._tried.get(key) != serial:
                self._tried[key] = serial
                rendered = True
                try:
                    comp.render_frame(frame)
                except Exception:  # noqa: BLE001 — a view that cannot render stays empty
                    import traceback
                    traceback.print_exc()
                lines, image = comp.hlr_cache.get(fid), comp.render_cache.get(fid)
                have = lines if lines is not None else image
            if have is None or self._content.get(key) is have:
                continue
            self._content[key] = have
            if lines is not None:
                import numpy as np
                kinds = comp.hlr_kinds.get(fid)
                segs = np.ascontiguousarray(np.asarray(lines, dtype=np.float32).reshape(-1, 4))
                kinds = (np.zeros(len(segs), dtype=np.uint8) if kinds is None
                         else np.ascontiguousarray(np.asarray(kinds, dtype=np.uint8)))
                renderer.page_frame_lines(key, segs, kinds)
            elif isinstance(image, QImage) and not image.isNull():
                import numpy as np
                rgba = image.convertToFormat(QImage.Format_RGBA8888)
                w, h = rgba.width(), rgba.height()
                pixels = np.frombuffer(rgba.constBits(), dtype=np.uint8,
                                       count=rgba.bytesPerLine() * h).reshape(h, -1)[:, :w * 4]
                renderer.page_frame_image(key, w, h, pixels.copy().reshape(-1))

    def _sync_selection(self, renderer) -> None:
        """The composer's selection (what the dock shows) and the page's."""
        keys = [it.model._doc_key for it in self._selected()]
        if keys != self._selection:
            self._selection = keys
            renderer.page_select(keys)

    def _select(self, keys) -> None:
        from views.composer import _SheetItem
        comp = self.composer
        want = set(keys)
        for it in comp.canvas.items():
            if isinstance(it, _SheetItem):
                it.setSelected(it.model._doc_key in want)
        self._selection = [it.model._doc_key for it in self._selected()]
        comp.on_selection_changed()

    def _models(self) -> dict:
        return {m._doc_key: m for m in self.composer.comp.all_items()}

    def _move(self, arg: str) -> None:
        """``key:x:y:w:h;…`` from a gesture on the page: one step of the
        sheet's history."""
        from core.composition import CompoundCommand, EditItemCommand, MarcoVista
        comp = self.composer
        models = self._models()
        commands = []
        for part in filter(None, arg.split(";")):
            key, x, y, w, h = part.split(":")
            model = models.get(int(key))
            if model is None or getattr(model, "locked", False):
                continue
            new = {"x_mm": float(x), "y_mm": float(y), "w_mm": float(w), "h_mm": float(h)}
            changes = {k: v for k, v in new.items()
                       if k in model._record_fields and abs(getattr(model, k) - v) > 1e-9}
            if not changes:
                continue
            commands.append(EditItemCommand(model, changes))
            if isinstance(model, MarcoVista) and ("w_mm" in changes or "h_mm" in changes):
                # Another window on the model: the view is worked out again.
                comp._stale.add(id(model))
                comp.hlr_cache.pop(id(model), None)
                comp.render_cache.pop(id(model), None)
        if commands:
            comp.history.execute(commands[0] if len(commands) == 1 else CompoundCommand(commands))

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
        self._send_page(renderer)
        self._sync_selection(renderer)
        self._send_contents(renderer)
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
        title, sections = self.inspector()
        self._once("insp", (title, sections), lambda v: renderer.ui_set_sheet_inspector(*v))
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
        comp = self.composer
        icons = {m: icon for m, icon, _tip, _drag in comp.TOOLS}
        groups = []
        for keys in RAIL:
            row = []
            for mode in keys:
                if mode not in icons:
                    continue
                w, h, rgba = wgpu_ui._icon_rgba(icons[mode])
                name = comp.TOOL_NAMES.get(mode, mode)
                if mode not in NATIVE_TOOLS:
                    name += " (classic sheet editor)"
                row.append((mode, name, "", w, h, rgba))
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

    def _selected(self):
        from views.composer import _SheetItem
        items = [it for it in self.composer.canvas.selectedItems() if isinstance(it, _SheetItem)]
        return items

    def _frame(self):
        from views.composer import FrameItem
        item = self.composer._selected_item()
        return item.model if isinstance(item, FrameItem) else None

    #: Mirrored panel fields per item kind: (kind, widget attribute, label).
    #: The composer keeps those widgets in step with the selection, and
    #: their signals carry the edit to the panel's own slot.
    FRAME = (("pick", "view_combo", "View"), ("pick", "scale_combo", "Scale"),
             ("pick", "style_combo", "Style"), ("pick", "shadow_combo", "Shadows"),
             ("spin", "fw_spin", "Width"), ("spin", "fh_spin", "Height"),
             ("spin", "rot_spin", "Rotation"), ("check", "persp_check", "Perspective"),
             ("check", "title_check", "View title"), ("check", "paper_bg_check", "Paper background"),
             ("button", "_on_refresh_selected_frame", "Update view"))
    TEXT = (("plain", "text_edit", "Text"), ("spin", "text_size", "Size"),
            ("check", "text_bold", "Bold"), ("check", "text_italic", "Italic"),
            ("check", "text_underline", "Underline"), ("pick", "text_align", "Alignment"),
            ("font", "text_family", "Font…"))
    CAJETIN = (("table", "caj_table", "Fields"), ("pick", "caj_design", "Design"),
               ("spin", "caj_w", "Width"), ("spin", "caj_h", "Height"),
               ("pick", "caj_layout", "Layout"))
    PAGE = (("name", None, "Name"), ("pick", "paper_combo", "Paper"),
            ("check", "landscape_check", "Landscape"), ("check", "border_check", "Sheet border"),
            ("check", "auto_check", "Auto-render views"),
            ("button", "refresh_all_frames", "Update all views"))

    def _spec(self):
        from views.composer import CajetinItem, FrameItem, TextItem
        items = self._selected()
        if not items:
            return "Sheet", self.PAGE
        if len(items) > 1:
            return f"{len(items)} items", ()
        item = self.composer._selected_item() or items[0]
        if isinstance(item, FrameItem):
            return "Model view", self.FRAME
        if isinstance(item, TextItem):
            return "Text", self.TEXT
        if isinstance(item, CajetinItem):
            return "Title block", self.CAJETIN
        return self.composer._item_label(item.model), ()

    @staticmethod
    def _clean(text) -> str:
        return str(text).replace(ED, " ").replace(SEP, " ").replace("|", "¦")

    def inspector(self):
        title, spec = self._spec()
        comp = self.composer
        rows = []
        for i, (kind, attr, label) in enumerate(spec):
            fid = f"{i}"
            if kind == "name":
                rows.append((label, f"{ED}edit{ED}{fid}{ED}{self._clean(comp.comp.name)}"))
                continue
            if kind in ("button", "font"):
                rows.append((label, f"{ED}button{ED}{fid}"))
                continue
            w = getattr(comp, attr, None)
            if w is None or w.isHidden() or not w.isEnabled():
                continue
            if kind == "pick":
                options = [self._clean(w.itemText(j)) for j in range(w.count())]
                at = w.currentIndex()
                current = self._clean(w.currentText())
                if w.isEditable() and current:
                    # An editable list (the scale) says what is typed, not
                    # what is highlighted.
                    if current in options:
                        at = options.index(current)
                    else:
                        options.append(current)
                        at = len(options) - 1
                if not options:
                    continue
                rows.append((label, f"{ED}pick{ED}{fid}{ED}{'|'.join(options)}{ED}{max(at, 0)}"))
            elif kind == "spin":
                text = f"{w.value():g}{w.suffix()}"
                rows.append((label, f"{ED}edit{ED}{fid}{ED}{self._clean(text)}"))
            elif kind == "check":
                rows.append((label, f"{ED}check{ED}{fid}{ED}{int(w.isChecked())}"))
            elif kind == "plain":
                rows.append((label, f"{ED}text{ED}{fid}{ED}{w.toPlainText().replace(ED, ' ').replace(SEP, ' ')}"))
            elif kind == "table":
                for r in range(w.rowCount()):
                    name = w.item(r, 0)
                    value = w.item(r, 1)
                    rows.append((self._clean(name.text() if name else f"Row {r + 1}"),
                                 f"{ED}edit{ED}{fid}.{r}{ED}{self._clean(value.text() if value else '')}"))
        if not spec and title.endswith("items"):
            rows.append(("", "Several items: their fields one at a time."))
        elif not spec:
            rows.append(("", "Its fields are in the classic sheet editor for now."))
        if spec is not self.FRAME and spec is not self.TEXT:
            rows.append(("Open in classic sheet editor", f"{ED}button{ED}classic"))
        return title, ((("", tuple(rows)),) if rows else ())

    def apply(self, fid: str, value: str) -> None:
        """An edit from the dock, through the panel widget it mirrors."""
        from PySide6.QtWidgets import QTableWidgetItem
        _title, spec = self._spec()
        comp = self.composer
        head, _, row = fid.partition(".")
        try:
            kind, attr, _label = spec[int(head)]
        except (ValueError, IndexError):
            return
        if kind == "name":
            comps = comp._scene().compositions
            if comp.comp in comps:
                comp.rename_sheet(comps.index(comp.comp), value)
            return
        if kind == "font":
            from views.font_picker import show
            item = comp._selected_item()
            model = item.model
            def apply(p):
                comp._panel_edit(item, dict(family=p["font"], bold=p["bold"], italic=p["italic"],
                                            size_pt=p["height"]))
                comp.on_selection_changed()
                self._touch()
            show(self.view.window(), "Sheet text", model.text, model.family, model.bold, model.italic,
                 model.size_pt, " pt", apply)
            return
        if kind == "button":
            getattr(comp, attr)()
            return
        w = getattr(comp, attr, None)
        if w is None:
            return
        if kind == "pick":
            i = int(value)
            if i < w.count():
                w.setCurrentIndex(i)
                w.activated.emit(i)
            if attr == "scale_combo":
                comp._on_scale_committed()
        elif kind == "spin":
            m = re.search(r"[-+]?\d+(?:[.,]\d+)?", value)
            if m:
                w.setValue(float(m.group(0).replace(",", ".")))
        elif kind == "check":
            w.setChecked(value == "1")
        elif kind == "plain":
            w.setPlainText(value)
        elif kind == "table":
            r = int(row or 0)
            cell = w.item(r, 1)
            if cell is None:
                w.setItem(r, 1, QTableWidgetItem(value))
            else:
                cell.setText(value)
        self._touch()

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
        tool = comp.TOOL_NAMES.get(mode, mode)
        hint = comp.statusBar().currentMessage()
        if not hint:
            hint = next((tip for m, _i, tip, _d in comp.TOOLS if m == mode), "")
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
                ("Sheet", [item("Update all views", comp.refresh_all_frames),
                           item("Auto-render views", lambda: comp.auto_check.toggle(),
                                checked=int(bool(comp._auto_render))),
                           item("Open in classic sheet editor…", self.open_classic),
                           item("Back to the model", lambda: to_model(comp._window))])]
        self._actions = acts
        return cluster, menu

    # ---- Actions from the chrome -------------------------------------------------------------
    def action(self, arg: str) -> None:
        verb, _, rest = arg.partition(SEP)
        comp = self.composer
        comps = comp._scene().compositions
        if verb == "tool":
            act = comp._tool_actions.get(rest)
            if rest not in NATIVE_TOOLS:
                self.open_classic(tool=rest)
            elif act is not None:
                act.trigger()
        elif verb == "select":
            self._select(int(k) for k in rest.split(",") if k)
        elif verb == "rects":
            self._move(rest)
        elif verb == "place":
            x0, y0, x1, y1 = (float(v) for v in rest.split(SEP))
            if comp.tool_mode == "vista":
                comp.place_tool(x0, y0, x1, y1)
        elif verb == "open":
            comp.show_sheet(int(rest))
        elif verb == "add":
            comp._on_comp_add()
        elif verb == "dup":
            comp.duplicate_sheet(int(rest), show=True)
        elif verb == "rename":
            i, _, name = rest.partition(SEP)
            comp.rename_sheet(int(i), name)
        elif verb in ("up", "down"):
            i = int(rest)
            j = i + (-1 if verb == "up" else 1)
            if 0 <= i < len(comps) and 0 <= j < len(comps):
                from core.history import MoveSheetCommand
                comp._window.viewport.history.execute(MoveSheetCommand(comps[i], j))
                comp._reload_comp_combo()
        elif verb == "del":
            i = int(rest)
            if 0 <= i < len(comps) and len(comps) > 1:
                self._pending = ("del", i)
                self._prompt = (f"Delete “{comps[i].name}”?",
                                "Its views, texts and dimensions go with it. Undo cannot bring it back.",
                                None, [("del", "Delete"), ("cancel", "Cancel")])
        elif verb == "answer":
            self._answer(*rest.split(SEP, 1)) if SEP in rest else self._answer(rest, "")
        elif verb == "edit":
            fid, _, value = rest.partition(SEP)
            self.apply(fid, value)
        elif verb == "button" and rest == "classic" or verb == "classic":
            self.open_classic()
        elif verb == "button":
            self.apply(rest, "")
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
            self.composer.delete_sheet(arg, confirm=False)


    # ---- The escape hatch ----------------------------------------------------------------
    def open_classic(self, tool: str = "") -> None:
        """What the dock does not cover yet (pens, colours, cajetín designs,
        guides, rulers…): the classic sheet editor. In the Qt app that is
        the composer's own window on this document, with the workspace
        left for the model; in the native shell, the classic app."""
        comp = self.composer
        win = comp._window
        real = getattr(win, "_host_window", win)
        if self._shell:
            from app_shell import files
            files.open_in_classic(real, why="The classic sheet editor")
            return
        from views.composer import ComposerWindow, bring_to_front
        from views.documents import DocumentHost
        classic = comp.__dict__.get("_classic")
        if classic is None or not _alive(classic):
            classic = ComposerWindow(real)
            host = DocumentHost(real, getattr(win, "_host_doc", None) or real._doc)
            classic._window = host
            panel = getattr(classic, "views_panel", None)
            if panel is not None and hasattr(panel, "_window"):
                panel._window = host
            comp._classic = classic
        comps = comp._scene().compositions
        index = comps.index(comp.comp) if comp.comp in comps else 0
        to_model(win)
        bring_to_front(classic)
        classic.show_sheet(index)
        act = classic._tool_actions.get(tool) if tool else None
        if act is not None:
            act.trigger()

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
                if kind == "pdf":
                    comp.refresh_all_frames()
                    comp.export_pdf(path)
                elif kind == "pdf_all":
                    troubled = comp.export_all_pdf(path)
                elif kind == "png":
                    comp.refresh_all_frames()
                    comp.export_image(path, comp.IMAGE_EXPORT_DPI)
                elif kind == "dxf":
                    comp.export_frame_dxf(self._frame(), path)
                elif kind == "svg":
                    comp.export_frame_svg(self._frame(), path)
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
        self.flash("The sheet's context menu is in the classic sheet editor for now", 2500)

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
        return [(seqs(QKeySequence.Undo), comp._on_undo),
                (seqs(QKeySequence.Redo, "Ctrl+Shift+Z"), comp._on_redo),
                (seqs(QKeySequence.Delete, Qt.Key_Backspace, Qt.Key_Delete), comp._on_delete_item),
                (seqs("Ctrl+D"), comp.duplicate_selected), (seqs("Ctrl+G"), comp.group_selected),
                (seqs("Ctrl+Shift+G"), comp.ungroup_selected), (seqs("Ctrl+L"), comp.lock_selected),
                (seqs("Ctrl+Shift+C"), comp.copy_style), (seqs("Ctrl+Shift+V"), comp.paste_style),
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
        """The composer's own timers (auto-render, coalesced edits), or
        frames still to render: the shell pumps Qt only while something is due."""
        if self._more:
            return True
        t = getattr(self.composer, "_auto_timer", None)
        return t is not None and t.isActive()


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
