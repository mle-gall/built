"""Sheets: a workspace of the Built window, not a window of its own.

The sheet composer (views/composer.py, QPainter on a QGraphicsScene) keeps
its model, its tools and its undo; only where it is seen changes:

* **The page** is the composer's own canvas view (a ``QGraphicsView``),
  never shown. It is sized to the Metal view, rendered offscreen into an
  image at the device pixel ratio, and only the rectangles the scene
  reports changed (``QGraphicsScene.changed``) are rendered again and
  uploaded (``Renderer.page_upload``). ``Renderer.render_page`` draws that
  layer instead of the model, with the chrome over it.
* **Input** that the chrome does not take (views/wgpu_ui.py hears it first)
  is handed to that canvas view as Qt events, so placement, snapping, the
  dimension tools, view editing and inline text keep their code. The
  composer window's shortcuts are mapped here (it has no window to catch
  them).
* **The chrome** is the sheet's while the workspace is on (ui/sheets.rs):
  the sheet tools on the rail, the sheets and the selected item's fields in
  the dock, paper, scale, tool hint and zoom in the bottom bar. The fields
  mirror the composer's own panel widgets, so every edit goes through the
  slot the Qt panel uses (undo, re-render, cache invalidation).

It works the same in the Qt app (``BUILT_UI`` on, Metal) and in the native
``built`` shell (app_shell/), where Qt is an offscreen library: no Qt
widget is ever shown. In the shell a guard refuses any Qt window the
composer would open, and the questions it asks (delete a sheet, where to
export) are asked in the dock. With ``BUILT_UI=0`` (or the GL view) the
composer stays the Qt window it always was.

The long-term plan ports sheets to Rust (docs/rust-port-plan.md, R3 / S3):
this adapter is deliberately thin, and the composer's model and logic stay
where they are.
"""
from __future__ import annotations

import re
from pathlib import Path

from PySide6.QtCore import QEvent, QObject, QPoint, QPointF, QRect, QRectF, QSize, Qt, QTimer
from PySide6.QtGui import (QColor, QContextMenuEvent, QFocusEvent, QImage, QKeyEvent,
                           QKeySequence, QMouseEvent, QPainter, QRegion, QResizeEvent,
                           QWheelEvent)

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
#: Tools that open a Qt dialog before anything else (no Qt window in the shell).
_DIALOG_TOOLS = {"imagen"}

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
    from views import wgpu_ui
    if not wgpu_ui.ENABLED:
        return False
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
            return super().close()

    _CLASS = SheetComposer
    return _CLASS


def composer_for(win):
    """The composer of ``win``'s front document, made on first use."""
    qt = _qt_window(win)
    if qt is not None and hasattr(qt, "_ensure_composer"):
        return qt._ensure_composer()
    comp = win.__dict__.get("_sheet_composer")
    if comp is None:
        comp = win.__dict__["_sheet_composer"] = composer_class()(win)
    return comp


def workspace_of(view):
    ws = view.__dict__.get("_sheets_ws") if view is not None else None
    return ws if ws is not None and ws.shown and ws.view is view else None


def shown(view) -> bool:
    return workspace_of(view) is not None


def to_sheets(win, index=None) -> None:
    """The workspace ▾ chose Sheets (or a sheet)."""
    qt = _qt_window(win)
    if qt is not None and hasattr(qt, "_to_sheets"):
        if index is None:
            qt._to_sheets()
        else:
            qt._show_sheet(index)
        return
    comp = composer_for(win)
    if index is not None:
        comp.show_sheet(index)
    comp.show()


def to_model(win) -> None:
    """The workspace ▾ chose Model."""
    qt = _qt_window(win)
    if qt is not None and hasattr(qt, "_show_model"):
        qt._show_model()
        return
    comp = win.__dict__.get("_sheet_composer")
    if comp is not None:
        comp.hide()


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


class _ShellGuard(QObject):
    """The shell has no Qt windows: a dialog, a menu or a picker the
    composer opens is closed before it can wait for an answer nobody can
    give, and the status line says so."""

    def __init__(self, ws):
        super().__init__()
        self._ws = ws

    def eventFilter(self, obj, event):  # noqa: N802
        if event.type() == QEvent.Show:
            from PySide6.QtWidgets import QWidget
            if isinstance(obj, QWidget) and obj.isWindow():
                QTimer.singleShot(0, lambda w=obj: self._refuse(w))
        return False

    def _refuse(self, w):
        if not _alive(w):
            return
        name = w.windowTitle() or type(w).__name__
        from PySide6.QtWidgets import QDialog, QMenu
        if isinstance(w, QDialog):
            w.reject()
        elif isinstance(w, QMenu):
            w.close()
        else:
            w.hide()
        self._ws.flash(f"{name}: not in the Built shell yet")


# ---- The workspace ---------------------------------------------------------------
class Workspace:
    def __init__(self, composer):
        from PySide6.QtWidgets import QFrame, QWidget
        self.composer = composer
        self.shown = False
        self.view = None
        cv = self.cv = composer._view
        # Out of the window's grid (rulers, splitter) onto a bare host the
        # workspace sizes: the window's layouts never run, it is not shown.
        area = getattr(composer, "_canvas_area", None)
        if area is not None and area.layout() is not None:
            area.layout().removeWidget(cv)
        self._host = QWidget(composer)
        self._host.setObjectName("sheet_canvas_host")
        self._host.hide()
        cv.setParent(self._host)
        cv.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        cv.setVerticalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        cv.setFrameShape(QFrame.NoFrame)
        composer.canvas.changed.connect(self._scene_changed)
        self.image = None
        self.dirty = QRegion()
        self.full = True
        self._view_sig = None
        self._fit_pending = True
        self._sent = {}
        self._pending = None            # the dock question's subject
        self._prompt = None             # (title, body, field, buttons) to send
        self._flash = None
        self._on_page = False           # the last press landed on the page
        self._actions = []
        self._shell = is_shell(composer._window)
        self._guard = None
        if self._shell:
            from PySide6.QtWidgets import QApplication
            self._guard = _ShellGuard(self)
            QApplication.instance().installEventFilter(self._guard)
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
        self._activate(True)
        self.full = True
        self.image = None
        self._sent = {}
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
        try:
            self.cv.cancel_placement()
            self.composer.end_inline_edit()
        except Exception:  # noqa: BLE001 — never block leaving
            pass
        self._activate(False)
        self.image = None
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
        try:
            self.composer.canvas.changed.disconnect(self._scene_changed)
        except (RuntimeError, TypeError):
            pass
        if self._guard is not None:
            from PySide6.QtWidgets import QApplication
            app = QApplication.instance()
            if app is not None:
                app.removeEventFilter(self._guard)
            self._guard = None

    @staticmethod
    def _reset_chrome(view) -> None:
        """The chrome's cached state goes: the other workspace sends its own."""
        for name in ("_ui_tabs", "_ui_cluster_sig", "_ui_opts", "_ui_sheet_sig", "_ui_jobs_sig"):
            view.__dict__[name] = None if name != "_ui_sheet_sig" else False

    def _activate(self, on: bool) -> None:
        """The scene as if its window were active and focused: items take
        focus (inline text), the selection paints as current."""
        from PySide6.QtWidgets import QApplication
        scene = self.composer.canvas
        if on:
            QApplication.sendEvent(scene, QEvent(QEvent.WindowActivate))
            QApplication.sendEvent(scene, QFocusEvent(QEvent.FocusIn, Qt.OtherFocusReason))
        else:
            QApplication.sendEvent(scene, QFocusEvent(QEvent.FocusOut, Qt.OtherFocusReason))
            QApplication.sendEvent(scene, QEvent(QEvent.WindowDeactivate))

    def _touch(self) -> None:
        view = self.view
        if view is not None and _alive(view):
            view.update()

    def flash(self, text: str, msec: int = 4000) -> None:
        self.composer.statusBar().showMessage(text, msec)
        self._touch()

    # ---- The page ----------------------------------------------------------------
    def _scene_changed(self, rects) -> None:
        if not self.shown:
            return
        for r in rects:
            box = self.cv.mapFromScene(r).boundingRect().adjusted(-2, -2, 2, 2)
            self.dirty = self.dirty.united(box)
        self._touch()

    def _settle(self, view) -> None:
        """The canvas the size of the Metal view, where it is on screen."""
        from PySide6.QtWidgets import QApplication
        size = QSize(max(1, view.width()), max(1, view.height()))
        cv = self.cv
        if cv.size() != size:
            old = cv.size()
            self._host.resize(size)
            cv.setGeometry(0, 0, size.width(), size.height())
            QApplication.sendEvent(cv, QResizeEvent(size, old))
            self.full = True
        # Dialogs and menus the composer places by its own widgets land
        # over the Metal view.
        try:
            want = view.mapToGlobal(QPoint(0, 0))
            have = cv.mapToGlobal(QPoint(0, 0))
            if want != have:
                self.composer.move(self.composer.pos() + (want - have))
        except Exception:  # noqa: BLE001
            pass
        sig = (cv.transform(), cv.horizontalScrollBar().value(), cv.verticalScrollBar().value(),
               cv.backgroundBrush().color().rgba())
        if sig != self._view_sig:
            self._view_sig = sig
            self.full = True

    def render(self, view):
        """Re-render what changed of the page into the image; returns the
        logical rect that changed (None: nothing)."""
        dpr = float(view.devicePixelRatioF())
        w, h = max(1, view.width()), max(1, view.height())
        W, H = max(1, int(w * dpr)), max(1, int(h * dpr))
        if self.image is None or (self.image.width(), self.image.height()) != (W, H):
            self.image = QImage(W, H, QImage.Format_RGBA8888)
            self.image.setDevicePixelRatio(dpr)
            self.image.fill(QColor(70, 76, 84))
            self.full = True
        bounds = QRect(0, 0, w, h)
        if self.full:
            region = QRegion(bounds)
        else:
            region = self.dirty.intersected(QRegion(bounds))
        self.full = False
        self.dirty = QRegion()
        if region.isEmpty():
            return None
        rect = region.boundingRect()
        painter = QPainter(self.image)
        try:
            painter.setClipRect(rect)
            self.cv.viewport().render(painter, rect.topLeft(), QRegion(rect))
        finally:
            painter.end()
        return rect

    def upload(self, view, rect) -> None:
        import numpy as np
        renderer = view.renderer
        image = self.image
        dpr = image.devicePixelRatio()
        x0, y0 = max(0, int(rect.x() * dpr)), max(0, int(rect.y() * dpr))
        x1 = min(image.width(), int((rect.x() + rect.width()) * dpr + 0.999))
        y1 = min(image.height(), int((rect.y() + rect.height()) * dpr + 0.999))
        if x1 <= x0 or y1 <= y0:
            return
        part = image.copy(QRect(x0, y0, x1 - x0, y1 - y0))
        part.setDevicePixelRatio(1.0)
        pixels = np.frombuffer(part.constBits(), dtype=np.uint8)[: (x1 - x0) * (y1 - y0) * 4]
        if not renderer.page_upload(image.width(), image.height(), x0, y0, x1 - x0, y1 - y0, pixels):
            self.full = True
        self.last_upload = (x0, y0, x1 - x0, y1 - y0)

    # ---- Fit and zoom ----------------------------------------------------------------
    def free_rect(self, view) -> QRectF:
        """The part of the window the chrome leaves the page."""
        w, h = view.width(), view.height()
        renderer = getattr(view, "renderer", None)

        def area(*names):
            for n in names:
                r = renderer.ui_area_rect(n) if renderer is not None and hasattr(renderer, "ui_area_rect") else None
                if r is not None:
                    return r
            return None
        top = area("top")
        bottom = area("bottom")
        rail = area("sheets_rail", "palette")
        dock = area("sheets_dock", "column")
        left = (rail[0] + rail[2]) if rail else 80.0
        right = dock[0] if dock else w - 316.0
        t = (top[1] + top[3]) if top else 40.0
        b = bottom[1] if bottom else h - 36.0
        r = QRectF(left + 8, t + 8, max(80.0, right - left - 16), max(80.0, b - t - 16))
        return r

    def fit(self, width_only: bool = False) -> None:
        view = self.view
        if view is None:
            return
        self._settle(view)
        free = self.free_rect(view)
        pw, ph = self.composer.comp.page_size_mm()
        s = (free.width() - 16) / max(pw, 1.0)
        if not width_only:
            s = min(s, (free.height() - 16) / max(ph, 1.0))
        c = free.center()
        self.cv._zoom_anchored(max(s, 1e-3), QPointF(pw / 2.0, ph / 2.0 if not width_only else
                                                      self.cv.mapToScene(c.toPoint()).y()), c.toPoint())
        self._fit_pending = False
        self.full = True
        self._touch()

    def zoom_to(self, text: str) -> None:
        m = re.search(r"[-+]?\d+(?:[.,]\d+)?", text or "")
        if m:
            self.composer.set_zoom(float(m.group(0).replace(",", ".")))
            self.full = True
            self._touch()

    # ---- One frame ---------------------------------------------------------------------
    def before_frame(self, view) -> None:
        renderer = view.renderer
        self._settle(view)
        if self._fit_pending:
            self.fit()
        self._send(renderer, view)
        rect = self.render(view)
        if rect is not None:
            self.upload(view, rect)
        self._sync_cursor(view)

    def _once(self, key, value, send) -> None:
        if self._sent.get(key, ...) != value:
            self._sent[key] = value
            send(value)

    def _send(self, renderer, view) -> None:
        comp = self.composer
        if "tools" not in self._sent:
            self._sent["tools"] = True
            renderer.ui_set_sheet_tools(self._tool_groups())
        renderer.ui_set_sheet_active(comp.tool_mode)
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
                if mode not in icons or (self._shell and mode in _DIALOG_TOOLS):
                    continue
                w, h, rgba = wgpu_ui._icon_rgba(icons[mode])
                row.append((mode, comp.TOOL_NAMES.get(mode, mode), "", w, h, rgba))
            if row:
                groups.append(row)
        return groups

    def _sync_cursor(self, view) -> None:
        if not self._on_page:
            return
        c = self.cv.viewport().cursor()
        key = (int(c.shape().value) if hasattr(c.shape(), "value") else int(c.shape()),
               c.pixmap().cacheKey() if c.shape() == Qt.BitmapCursor else 0)
        if key != view.__dict__.get("_sheets_cursor"):
            view._sheets_cursor = key
            view.setCursor(c)

    # ---- What the dock shows -------------------------------------------------------------
    def sheet_rows(self):
        comps = list(self.composer._scene().compositions)
        cur = self.composer.comp
        rows = []
        for c in comps:
            paper = f"{c.paper} {'▭' if c.landscape else '▯'}"
            n = sum(len(getattr(c, k, []) or []) for k in (
                "frames", "texts", "images", "scalebars", "nortes", "leyendas", "shapes",
                "cotas", "cotas_ang", "cotas_rad", "etiquetas", "perfiles", "niveles", "llamadas"))
            n += 1 if getattr(c, "cajetin", None) is not None else 0
            rows.append((c.name, f"{paper} · {n}", c is cur))
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
            ("check", "text_underline", "Underline"), ("pick", "text_align", "Alignment"))
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
            if kind == "button":
                rows.append((label, f"{ED}button{ED}{fid}"))
                continue
            w = getattr(comp, attr, None)
            if w is None or w.isHidden() or not w.isEnabled():
                continue
            if kind == "pick":
                options = [self._clean(w.itemText(j)) for j in range(w.count())]
                at = w.currentIndex()
                current = self._clean(w.currentText())
                if w.isEditable() and current and current not in options:
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
            rows.append(("", "More of its fields in the Qt sheet window for now."))
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
        self.full = True

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
        mode = comp.tool_mode
        tool = comp.TOOL_NAMES.get(mode, mode)
        hint = comp.statusBar().currentMessage()
        if not hint:
            hint = next((tip for m, _i, tip, _d in comp.TOOLS if m == mode), "")
        cursor = comp._pos_label.text() if hasattr(comp, "_pos_label") else ""
        zoom = f"{comp.zoom_percent():.0f}%"
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
                           item("Back to the model", lambda: to_model(comp._window))])]
        self._actions = acts
        if self.view is not None:
            self.view._ui_actions = acts
        return cluster, menu

    # ---- Actions from the chrome -------------------------------------------------------------
    def action(self, arg: str) -> None:
        verb, _, rest = arg.partition(SEP)
        comp = self.composer
        comps = comp._scene().compositions
        if verb == "tool":
            act = comp._tool_actions.get(rest)
            if act is not None and not (self._shell and rest in _DIALOG_TOOLS):
                act.trigger()
        elif verb == "open":
            comp.show_sheet(int(rest))
            self.full = True
            self._fit_pending = True
        elif verb == "add":
            comp._on_comp_add()
            self._fit_pending = True
        elif verb == "dup":
            comp.duplicate_sheet(int(rest), show=True)
            self._fit_pending = True
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
            if self.composer.delete_sheet(arg, confirm=False):
                self._fit_pending = True
        elif what == "export" and bid == "export" and text.strip():
            self._export_to(arg, str(Path(text.strip()).expanduser()))

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
        if self._shell:
            # No native picker in the shell yet: where it goes is asked in the dock.
            self._pending = ("export", kind)
            self._prompt = (title.rstrip("…"), "The file to write:", self._default_path(kind),
                            [("export", "Export"), ("cancel", "Cancel")])
            self._touch()
            return
        from views.filedialogs import file_dialogs
        parent = _qt_window(self.composer._window)
        path, _ = file_dialogs.getSaveFileName(parent, title, self._default_path(kind), filters)
        if path:
            if not Path(path).suffix:
                path += ext
            self._export_to(kind, path)

    def _export_to(self, kind: str, path: str) -> None:
        comp = self.composer
        from views import chrome_dialogs
        parent = _qt_window(comp._window)
        labels = {"pdf": "Exporting PDF", "pdf_all": "Exporting the sheets", "png": "Exporting image",
                  "dxf": "Exporting DXF", "svg": "Exporting SVG"}
        name = Path(path).name
        troubled = []
        try:
            with chrome_dialogs.busy(parent, f"{labels[kind]} · {name}"):
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
        self.full = True

    # ---- Input -----------------------------------------------------------------------------
    def _mouse(self, qtype, event):
        pos = QPointF(event.position())
        try:
            gpos = QPointF(event.globalPosition())
        except Exception:  # noqa: BLE001
            gpos = QPointF(pos)
        return QMouseEvent(qtype, pos, gpos, Qt.MouseButton(event.button()),
                           Qt.MouseButtons(event.buttons()), Qt.KeyboardModifiers(event.modifiers()))

    def pointer(self, kind: str, event) -> None:
        from PySide6.QtWidgets import QApplication
        vp = self.cv.viewport()
        if kind == "leave" or event is None:
            self._on_page = False
            QApplication.sendEvent(vp, QEvent(QEvent.Leave))
            return
        qtype = event.type()
        if kind == "press" and qtype not in (QEvent.MouseButtonPress, QEvent.MouseButtonDblClick):
            qtype = QEvent.MouseButtonPress
        elif kind == "release":
            qtype = QEvent.MouseButtonRelease
        elif kind == "move":
            qtype = QEvent.MouseMove
        if kind == "press":
            self._on_page = True
        elif kind == "move" and not self._on_page:
            self._on_page = True
            QApplication.sendEvent(vp, QEvent(QEvent.Enter))
        QApplication.sendEvent(vp, self._mouse(qtype, event))
        if self.view is not None:
            self._sync_cursor(self.view)

    def chrome_took(self) -> None:
        """The pointer went over the chrome: the page no longer has it."""
        if self._on_page:
            from PySide6.QtWidgets import QApplication
            self._on_page = False
            QApplication.sendEvent(self.cv.viewport(), QEvent(QEvent.Leave))

    def wheel(self, event) -> None:
        from PySide6.QtWidgets import QApplication
        pos = QPointF(event.position())
        try:
            gpos = QPointF(event.globalPosition())
        except Exception:  # noqa: BLE001
            gpos = QPointF(pos)
        phase = event.phase() if hasattr(event, "phase") else Qt.NoScrollPhase
        inverted = bool(event.inverted()) if hasattr(event, "inverted") else False
        buttons = event.buttons() if hasattr(event, "buttons") else Qt.NoButton
        ev = QWheelEvent(pos, gpos, event.pixelDelta(), event.angleDelta(), Qt.MouseButtons(buttons),
                         Qt.KeyboardModifiers(event.modifiers()), phase, inverted)
        QApplication.sendEvent(self.cv.viewport(), ev)
        self._touch()

    def gesture(self, event) -> bool:
        kind = event.gestureType()
        pos = QPointF(event.position()).toPoint()
        if kind == Qt.ZoomNativeGesture:
            self.cv._zoom_about(1.0 + float(event.value()), pos)
        elif kind == Qt.SmartZoomNativeGesture:
            self.fit()
        else:
            return True
        self.full = True
        self._touch()
        return True

    def context_menu(self, event) -> None:
        if self._shell:
            self.flash("Context menus are not in the Built shell yet", 2500)
            return
        if not self._on_page:
            return
        from PySide6.QtWidgets import QApplication
        pos = event.pos()
        try:
            gpos = event.globalPos()
        except Exception:  # noqa: BLE001
            gpos = self.view.mapToGlobal(pos)
        ev = QContextMenuEvent(QContextMenuEvent.Mouse, pos, gpos, Qt.KeyboardModifiers(event.modifiers()))
        QApplication.sendEvent(self.cv.viewport(), ev)
        self.full = True
        self._touch()

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
            return QKeySequence(int(event.modifiers().value) | int(getattr(event.key(), "value", event.key())))
        except Exception:  # noqa: BLE001
            return QKeySequence()

    def _editing(self) -> bool:
        from views.composer import InlineTextEditor
        return isinstance(self.composer.canvas.focusItem(), InlineTextEditor)

    @staticmethod
    def window_key(event) -> bool:
        combo = Workspace._combo(event)
        return any(QKeySequence(k).matches(combo) == QKeySequence.ExactMatch for k in _WINDOW_KEYS)

    def key(self, event, pressed: bool) -> None:
        from PySide6.QtWidgets import QApplication
        k = event.key()
        k = Qt.Key(int(getattr(k, "value", k)))
        mods = Qt.KeyboardModifiers(event.modifiers())
        text = event.text() or ""
        repeat = bool(event.isAutoRepeat()) if hasattr(event, "isAutoRepeat") else False
        if pressed and not self._editing() and not repeat:
            combo = self._combo(event)
            for keys, run in self._keys:
                if any(s.matches(combo) == QKeySequence.ExactMatch for s in keys):
                    run()
                    self.full = True
                    self._touch()
                    return
        ev = QKeyEvent(QEvent.KeyPress if pressed else QEvent.KeyRelease, k, mods, text, repeat)
        QApplication.sendEvent(self.cv, ev)
        self.full = True
        self._touch()

    # ---- The shell's loop -------------------------------------------------------------------
    def timers_pending(self) -> bool:
        """The composer's own timers (auto-render, coalesced edits): the
        shell pumps Qt only while something is due."""
        comp = self.composer
        for name in ("_auto_timer",):
            t = getattr(comp, name, None)
            if t is not None and t.isActive():
                return True
        return False


# ---- Hooks (views/wgpu_view.py, views/wgpu_ui.py) ---------------------------------------------
def frame(view):
    """A frame of the workspace: the chrome's window state, the page's
    changed rectangles, then the page and the chrome."""
    import time
    from views import wgpu_start, wgpu_ui
    ws = workspace_of(view)
    renderer = view.renderer
    start = time.perf_counter()
    win = wgpu_ui._window(view)
    if win is not None and getattr(view, "_ui_attached", False):
        wgpu_ui._sheet_and_jobs(view, win)
        wgpu_start.before_frame(view, win)
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
