"""Forms drawn by the Built chrome (egui) instead of Qt dialogs: name and
number prompts, Preferences, export options, Location, Scale, Soften,
About. ``rust/ingetrazo_render/src/ui/dialog.rs`` draws them.

A :class:`Form` is plain data: fields with values, buttons, callbacks. It
works the same in the Qt app and in the ``built`` shell, which has no Qt
widgets and no Qt event loop of its own (see ``views/chrome_dialogs.py``,
"In the built shell"):

* :func:`show` puts it on the window and returns at once. The chrome
  reports every edit (Python always holds what is on screen) and the
  button pressed; ``on_done(answer, values)`` then runs, on a later turn
  of whichever loop drives the window. No nested ``exec()`` anywhere.
* When the chrome is not on screen (``BUILT_UI=0``, the classic app,
  headless tests, a parent that is a Qt dialog) the same form runs as a
  Qt dialog built from its fields (:func:`_qt_exec`), inline: ``on_done``
  has run when :func:`show` returns. Callers that keep a richer Qt
  dialog (Preferences) ask :func:`available` first and open that.

So a caller writes its continuation once::

    def make(name):
        if name is not None:
            ...
    chrome_forms.ask_text(win, tr("Make Component"), tr("Component name:"),
                          text=suggested, done=make)

Multi-step questions read top to bottom with :func:`flow`::

    def steps():
        system = yield chrome_forms.request(chrome_forms.ask_choice, win, ...)
        if system is None:
            return
        code = yield chrome_forms.request(chrome_forms.ask_choice, win, ...)
    chrome_forms.flow(steps())
"""
from __future__ import annotations

import itertools
import sys

from core.i18n import tr

_seq = itertools.count(1)

#: Dialog field flags (ui/dialog.rs).
DISABLED, FOCUS, MULTILINE, PASSWORD, FULL = 1, 2, 4, 8, 16
_ROLES = {"default": 1, "discard": 2, "cancel": 3}


# ---- What a form is made of ---------------------------------------------------
class Field:
    """One row. ``kind``: page, caption, text, edit, number, check, choice,
    color, slider, path, button.

    ``value``: edit/path → str, number/slider → float (int when
    ``decimals`` is 0), check → bool, choice → the key of one of
    ``options`` ([(key, label)] or plain labels), color → "#rrggbb".
    ``enabled`` / ``visible`` may be callables of the form, evaluated each
    time the form is drawn (a check that enables the row under it)."""

    def __init__(self, fid, kind, label="", value=None, *, options=(), lo=0.0, hi=0.0,
                 decimals=0, suffix="", tip="", enabled=True, visible=True, focus=False,
                 multiline=False, password=False, full=False, row_label="", note="",
                 pick=None, filters="", fmt=None):
        self.id = str(fid)
        self.kind = kind
        self.label = label
        self.value = value
        self.options = [(o, o) if isinstance(o, str) else (o[0], o[1]) for o in options]
        self.lo, self.hi, self.decimals = float(lo), float(hi), int(decimals)
        self.suffix, self.tip = suffix, tip
        self.enabled, self.visible = enabled, visible
        self.focus, self.multiline, self.password, self.full = focus, multiline, password, full
        self.row_label = row_label      # check: the left column's label
        self.note = note                # button: a note right of it
        self.pick = pick                # path: "open" | "save" | "folder"
        self.filters = filters          # path: a Qt filter string
        self.fmt = fmt                  # slider: value → shown text

    def keys(self):
        return [k for k, _ in self.options]

    def flags(self, form) -> int:
        f = 0
        if not _truth(self.enabled, form):
            f |= DISABLED
        if self.focus:
            f |= FOCUS
        if self.multiline:
            f |= MULTILINE
        if self.password:
            f |= PASSWORD
        if self.full:
            f |= FULL
        return f


def _truth(v, form):
    return bool(v(form)) if callable(v) else bool(v)


def number_text(value, decimals: int) -> str:
    if value is None:
        return ""
    if decimals <= 0:
        return str(int(round(float(value))))
    return f"{float(value):.{decimals}f}".rstrip("0").rstrip(".") or "0"


class Form:
    """A dialog's data and its answers. ``buttons``: [(id, label)] in
    reading order; ``default`` answers Enter, ``cancel`` Esc (and closes
    without validating), ``discard`` sits apart on the left.

    ``on_done(answer, values)``: once, with the button id and every value.
    ``on_change(form, field_id, value)``: each edit, live.
    ``on_button(form, field_id)``: an inline button (or a path's Choose…
    when the field has no ``pick``).
    ``validate(form, answer)``: an error text keeps the form open and
    shows the text under the title; ``None`` lets it close."""

    def __init__(self, title, fields, *, note="", buttons=None, default="ok", cancel="cancel",
                 discard=None, width=420, on_done=None, on_change=None, on_button=None,
                 validate=None, key=None):
        self.seq = str(next(_seq))
        self.title = title
        self.note = note
        self.error = ""
        self.fields = list(fields)
        self.buttons = list(buttons) if buttons is not None else [
            ("cancel", tr("Cancel")), ("ok", tr("OK"))]
        self.default, self.cancel, self.discard = default, cancel, discard
        self.width = width
        self.on_done, self.on_change, self.on_button = on_done, on_change, on_button
        self.validate = validate
        self.key = key                  # one per window: a second show raises it
        self.page = 0
        self.answer = None
        self.closed = False
        self.shown = False              # drawn at least once
        self.win = None
        self._texts = {}                # number fields: the text being typed
        self.version = 0

    # ---- values -----------------------------------------------------------
    def field(self, fid):
        return next((f for f in self.fields if f.id == str(fid)), None)

    def value(self, fid):
        f = self.field(fid)
        return None if f is None else f.value

    def values(self) -> dict:
        return {f.id: f.value for f in self.fields
                if f.kind not in ("page", "caption", "text", "button")}

    def set(self, fid, value) -> None:
        """Change a value from Python (restore defaults, a picked path)."""
        f = self.field(fid)
        if f is None:
            return
        f.value = value
        self._texts.pop(f.id, None)
        self.changed()

    def changed(self) -> None:
        self.version += 1
        if self.win is not None:
            _repaint(self.win)

    def receive(self, fid, raw: str) -> None:
        """An edit from the chrome (or the Qt fallback): typed into its value."""
        f = self.field(fid)
        if f is None:
            return
        if f.kind == "edit" or f.kind == "path":
            f.value = raw
        elif f.kind == "number":
            self._texts[f.id] = raw
            try:
                v = float(raw.replace(",", ".").strip())
            except ValueError:
                return                  # half typed: the value waits
            if f.hi > f.lo:
                v = min(max(v, f.lo), f.hi)
            f.value = int(round(v)) if f.decimals <= 0 else v
        elif f.kind == "check":
            f.value = raw in ("1", "true", "True")
        elif f.kind == "choice":
            try:
                f.value = f.keys()[int(float(raw))]
            except (ValueError, IndexError):
                return
        elif f.kind == "slider":
            try:
                v = float(raw)
            except ValueError:
                return
            f.value = int(round(v)) if f.decimals <= 0 else v
        elif f.kind == "color":
            f.value = raw
        else:
            return
        if self.on_change is not None:
            self.on_change(self, f.id, f.value)
        self.changed()

    # ---- answers ----------------------------------------------------------
    def press(self, bid: str) -> None:
        if self.closed:
            return
        if any(b[0] == bid for b in self.buttons):
            if bid != self.cancel and self.validate is not None:
                err = self.validate(self, bid)
                if err:
                    self.error = err
                    self.changed()
                    return
            self.finish(bid)
            return
        f = self.field(bid)
        if f is None or not _truth(f.enabled, self):
            return
        if f.kind == "path" and f.pick:
            _pick_path(self, f)
        elif self.on_button is not None:
            self.on_button(self, f.id)
        self.changed()

    def finish(self, answer) -> None:
        if self.closed:
            return
        self.closed = True
        self.answer = answer
        if self.win is not None:
            _remove(self.win, self)
        if self.on_done is not None:
            self.on_done(answer, self.values())

    def close(self) -> None:
        """Take it down from Python, as Cancel would."""
        self.finish(self.cancel)

    # ---- what the chrome draws ----------------------------------------------
    def wire(self):
        rows = []
        page = -1
        for f in self.fields:
            if f.kind == "page":
                page += 1
            if not _truth(f.visible, self):
                continue
            flags = f.flags(self)
            k = f.kind
            if k in ("page", "caption"):
                rows.append((f.id, k, f.label, "", 0.0, 0.0, 0.0, [], flags, f.tip))
            elif k == "text":
                rows.append((f.id, k, f.label, str(f.value or ""), 0.0, 0.0, 0.0, [], flags, f.tip))
            elif k == "edit":
                rows.append((f.id, k, f.label, str(f.value or ""), 0.0, 0.0, 0.0, [], flags, f.tip))
            elif k == "number":
                text = self._texts.get(f.id, number_text(f.value, f.decimals))
                rows.append((f.id, k, f.label, text, float(f.value or 0), f.lo, f.hi,
                             [f.suffix] if f.suffix else [], flags, f.tip))
            elif k == "check":
                rows.append((f.id, k, f.label, f.row_label, 1.0 if f.value else 0.0, 0.0, 1.0, [],
                             flags, f.tip))
            elif k == "choice":
                keys = f.keys()
                at = keys.index(f.value) if f.value in keys else 0
                rows.append((f.id, k, f.label, "", float(at), 0.0, float(len(keys)),
                             [label for _, label in f.options], flags, f.tip))
            elif k == "slider":
                shown = f.fmt(f.value) if f.fmt else number_text(f.value, f.decimals) + (
                    f" {f.suffix}" if f.suffix else "")
                rows.append((f.id, k, f.label, shown, float(f.value or 0), f.lo, f.hi, [], flags, f.tip))
            elif k == "color":
                rows.append((f.id, k, f.label, str(f.value or ""), 0.0, 0.0, 0.0, [], flags, f.tip))
            elif k == "path":
                rows.append((f.id, k, f.label, str(f.value or ""), 0.0, 0.0, 0.0, [tr("Choose…")],
                             flags, f.tip))
            elif k == "button":
                rows.append((f.id, k, f.label, f.note, 0.0, 0.0, 0.0, [], flags, f.tip))
        buttons = []
        for bid, label in self.buttons:
            role = 1 if bid == self.default else 2 if bid == self.discard else 3 if bid == self.cancel else 0
            hint = {1: "↩", 3: "esc"}.get(role, "")
            buttons.append((bid, label, hint, role))
        note = self.note
        if self.error:
            note = f"{self.error}\n{note}" if note else self.error
        return self.seq, self.title, note, float(self.width), rows, buttons


# ---- On the window -------------------------------------------------------------
def available(parent) -> bool:
    """The chrome can show a form for ``parent`` now."""
    from views import chrome_dialogs
    return chrome_dialogs.live(parent) is not None


def forms(win) -> list:
    return win.__dict__.setdefault("_chrome_forms", [])


def current(win):
    stack = win.__dict__.get("_chrome_forms") if win is not None else None
    return stack[-1] if stack else None


def show(parent, form: Form) -> str:
    """Put ``form`` on the chrome ("chrome"), or run it as a Qt dialog now
    ("qt"); in the shell with no chrome up, cancel it ("none")."""
    from views import chrome_dialogs
    view = chrome_dialogs.live(parent)
    if view is None:
        if chrome_dialogs.in_shell():
            print(f"[built] form «{form.title}» with no chrome to show it", file=sys.stderr, flush=True)
            form.finish(form.cancel)
            return "none"
        _qt_exec(parent, form)
        return "qt"
    win = chrome_dialogs.window_of(parent)
    if form.key is not None:
        old = next((f for f in forms(win) if f.key == form.key), None)
        if old is not None:
            forms(win).remove(old)
            forms(win).append(old)      # raised, not opened twice
            _repaint(win)
            return "chrome"
    form.win = win
    forms(win).append(form)
    chrome_dialogs.engage(win)
    _watch(view, win, form)
    _repaint(win)
    return "chrome"


def _remove(win, form) -> None:
    from views import chrome_dialogs
    stack = forms(win)
    if form in stack:
        stack.remove(form)
        chrome_dialogs.release(win)
    _repaint(win)


def _repaint(win) -> None:
    from views import chrome_dialogs
    view = chrome_dialogs.view_of(win)
    if view is not None:
        view._ui_dialog_sig = None
        view.update()


def _watch(view, win, form) -> None:
    """The sheet's watchdog: no frame drew the form in time (the window
    covered, the surface lost) → it runs as a Qt dialog instead. Never in
    the shell, which has no Qt dialog to fall back to."""
    from views import chrome_dialogs
    if chrome_dialogs.in_shell():
        return
    from PySide6.QtCore import QTimer
    frames = getattr(view, "_ui_frames", 0)

    def check():
        if form.closed or form.shown or getattr(view, "_ui_frames", 0) != frames:
            return
        _remove(win, form)
        form.win = None
        _qt_exec(win, form)
    QTimer.singleShot(chrome_dialogs.WATCHDOG_MS, check)


def action(win, arg: str) -> None:
    """``dlg:<seq>\\x1f<v|b|p>\\x1f<id>[\\x1f<value>]`` from the chrome."""
    seq, _, rest = arg.partition("\x1f")
    form = current(win)
    if form is None or form.seq != seq:
        return                          # a click for a form already gone
    kind, _, rest = rest.partition("\x1f")
    fid, _, value = rest.partition("\x1f")
    if kind == "v":
        form.receive(fid, value)
    elif kind == "b":
        form.press(fid)
    elif kind == "p":
        try:
            form.page = int(fid)
        except ValueError:
            pass


def before_frame(view, win) -> None:
    """Send the form on top (only when it changed)."""
    form = current(win)
    renderer = view.renderer
    if form is None:
        if view.__dict__.get("_ui_dialog_sig", 0) != ():
            view._ui_dialog_sig = ()
            renderer.ui_set_dialog("")
        return
    wire = form.wire()
    sig = (wire, form.version)
    if sig != view.__dict__.get("_ui_dialog_sig"):
        view._ui_dialog_sig = sig
        renderer.ui_set_dialog(*wire)
    form.shown = True


def _pick_path(form, f) -> None:
    """A path field's Choose…: ``views.filedialogs.file_dialogs``, which is
    Qt's picker in the Qt app and the Mac's own panel in the shell
    (``app_shell.files.ShellFileDialog``)."""
    from views.filedialogs import file_dialogs
    parent = form.win if hasattr(form.win, "isWindow") else None
    current_value = str(f.value or "")
    if f.pick == "folder":
        got = file_dialogs.getExistingDirectory(parent, f.label or tr("Choose a folder"), current_value)
    elif f.pick == "save":
        got, _ = file_dialogs.getSaveFileName(parent, f.label or tr("Save as"), current_value, f.filters)
    else:
        got, _ = file_dialogs.getOpenFileName(parent, f.label or tr("Open"), current_value, f.filters)
    if got:
        f.value = got
        if form.on_change is not None:
            form.on_change(form, f.id, got)


# ---- Prompts -------------------------------------------------------------------
def _no_window(done) -> bool:
    """In the shell with no chrome up, no Qt prompt can show: cancel."""
    from views import chrome_dialogs
    if not chrome_dialogs.in_shell():
        return False
    if done is not None:
        done(None)
    return True


def ask_text(parent, title, label, text="", done=None, *, multiline=False, ok=None, note=""):
    """A name prompt: ``done(text)`` or ``done(None)`` on Cancel."""
    if not available(parent):
        if _no_window(done):
            return
        from views import prompts
        if multiline:
            value, accepted = prompts.get_multiline_text(parent, title, label, text)
        else:
            value, accepted = prompts.get_text(parent, title, label, text=text)
        if done is not None:
            done(value if accepted else None)
        return
    form = Form(title, [Field("text", "edit", label, text, focus=True, multiline=multiline,
                              full=multiline)],
                note=note, buttons=[("cancel", tr("Cancel")), ("ok", ok or tr("OK"))],
                width=440 if not multiline else 520,
                on_done=lambda a, v: done(v["text"] if a == "ok" else None) if done else None)
    show(parent, form)


def ask_number(parent, title, label, value, lo, hi, decimals=0, done=None, *, suffix="", step=1, note=""):
    """``QInputDialog.getInt`` (decimals 0) / ``getDouble``: ``done(value)``
    or ``done(None)``."""
    if not available(parent):
        if _no_window(done):
            return
        from PySide6.QtWidgets import QInputDialog
        if decimals <= 0:
            got, accepted = QInputDialog.getInt(parent, title, label, int(value), int(lo), int(hi), int(step))
        else:
            got, accepted = QInputDialog.getDouble(parent, title, label, float(value), float(lo),
                                                   float(hi), int(decimals))
        if done is not None:
            done(got if accepted else None)
        return

    def check(form, _answer):
        raw = form._texts.get("value")
        if raw is not None:
            try:
                v = float(raw.replace(",", ".").strip())
            except ValueError:
                return tr("Type a number.")
            if not lo <= v <= hi:
                return tr("Between {lo} and {hi}.", lo=number_text(lo, decimals),
                          hi=number_text(hi, decimals))
        return None
    form = Form(title, [Field("value", "number", label, value, lo=lo, hi=hi, decimals=decimals,
                              suffix=suffix, focus=True)],
                note=note, validate=check, width=400,
                on_done=lambda a, v: done(v["value"] if a == "ok" else None) if done else None)
    show(parent, form)


def ask_choice(parent, title, label, items, current=0, done=None, *, note=""):
    """``QInputDialog.getItem`` (not editable): ``done(item)`` or ``done(None)``."""
    items = [str(i) for i in items]
    if not available(parent):
        if _no_window(done):
            return
        from PySide6.QtWidgets import QInputDialog
        got, accepted = QInputDialog.getItem(parent, title, label, items, int(current), False)
        if done is not None:
            done(got if accepted else None)
        return
    start = items[current] if 0 <= current < len(items) else (items[0] if items else None)
    form = Form(title, [Field("item", "choice", label, start, options=items)], note=note, width=420,
                on_done=lambda a, v: done(v["item"] if a == "ok" else None) if done else None)
    show(parent, form)


# ---- Steps that read top to bottom ---------------------------------------------
class request:  # noqa: N801 — reads like a call at the yield
    """``yield request(ask_text, parent, title, label, text=…)``: the call,
    with ``done`` left for :func:`flow` to fill in."""

    def __init__(self, fn, *args, **kwargs):
        self.fn, self.args, self.kwargs = fn, args, kwargs

    def start(self, resume) -> None:
        self.fn(*self.args, done=resume, **self.kwargs)


def flow(gen) -> None:
    """Run a generator of :class:`request`: each answer is sent back in,
    on whichever turn it comes (at once with Qt, later on the chrome)."""
    def step(value=None, first=False):
        try:
            req = next(gen) if first else gen.send(value)
        except StopIteration:
            return
        req.start(step)
    step(first=True)


# ---- The Qt fallback ------------------------------------------------------------
def _qt_exec(parent, form: Form) -> None:
    """``form`` as a modal QDialog, now: same fields, same buttons, same
    callbacks (``on_change`` on every edit, ``on_done`` on close)."""
    from PySide6.QtCore import Qt
    from PySide6.QtGui import QColor
    from views.color_dialog import get_color
    from PySide6.QtWidgets import (QCheckBox, QComboBox, QDialog, QDialogButtonBox,
                                   QDoubleSpinBox, QFormLayout, QHBoxLayout, QLabel, QLineEdit,
                                   QPlainTextEdit, QPushButton, QSlider, QSpinBox, QTabWidget,
                                   QVBoxLayout, QWidget)
    if parent is not None and not hasattr(parent, "isWindow"):
        parent = None                   # not a QWidget (a shell object)
    dlg = QDialog(parent)
    dlg.setWindowTitle(form.title)
    dlg.setMinimumWidth(int(min(form.width, 560)))
    outer = QVBoxLayout(dlg)
    note = QLabel(form.note)
    note.setWordWrap(True)
    note.setVisible(bool(form.note))
    outer.addWidget(note)
    tabs = None
    if any(f.kind == "page" for f in form.fields):
        tabs = QTabWidget()
        outer.addWidget(tabs)
    page_form = None
    if tabs is None:
        page_form = QFormLayout()
        outer.addLayout(page_form)
    widgets = {}
    syncs = []

    def changed(fid, raw):
        form.receive(fid, raw)
        for s in syncs:
            s()

    for f in form.fields:
        if f.kind == "page":
            page = QWidget()
            page_form = QFormLayout(page)
            tabs.addTab(page, f.label)
            continue
        if page_form is None:
            continue
        w = None
        label = f.label
        if f.kind == "caption":
            cap = QLabel(f"<b>{f.label}</b>")
            page_form.addRow(cap)
            continue
        if f.kind == "text":
            t = QLabel(str(f.value or f.label))
            t.setWordWrap(True)
            page_form.addRow(t)
            continue
        if f.kind == "edit":
            if f.multiline:
                w = QPlainTextEdit(str(f.value or ""))
                w.textChanged.connect(lambda fid=f.id, w=w: changed(fid, w.toPlainText()))
            else:
                w = QLineEdit(str(f.value or ""))
                if f.password:
                    w.setEchoMode(QLineEdit.Password)
                w.textEdited.connect(lambda t, fid=f.id: changed(fid, t))
        elif f.kind == "number":
            if f.decimals <= 0:
                w = QSpinBox()
                w.setRange(int(f.lo), int(f.hi) if f.hi > f.lo else 2**31 - 1)
                w.setValue(int(f.value or 0))
                w.valueChanged.connect(lambda v, fid=f.id: changed(fid, str(v)))
            else:
                w = QDoubleSpinBox()
                w.setDecimals(f.decimals)
                w.setRange(f.lo, f.hi if f.hi > f.lo else 1e12)
                w.setValue(float(f.value or 0))
                w.valueChanged.connect(lambda v, fid=f.id: changed(fid, repr(v)))
            if f.suffix:
                w.setSuffix(" " + f.suffix)
        elif f.kind == "check":
            w = QCheckBox(f.label)
            w.setChecked(bool(f.value))
            w.toggled.connect(lambda on, fid=f.id: changed(fid, "1" if on else "0"))
            label = f.row_label
        elif f.kind == "choice":
            w = QComboBox()
            for k, text in f.options:
                w.addItem(text, k)
            keys = f.keys()
            w.setCurrentIndex(keys.index(f.value) if f.value in keys else 0)
            w.currentIndexChanged.connect(lambda i, fid=f.id: changed(fid, str(i)))
        elif f.kind == "slider":
            w = QSlider(Qt.Horizontal)
            w.setRange(int(f.lo), int(f.hi))
            w.setValue(int(f.value or 0))
            w.valueChanged.connect(lambda v, fid=f.id: changed(fid, str(v)))
        elif f.kind == "color":
            w = QPushButton(str(f.value or ""))

            def pick_color(_=False, fid=f.id, button=w):
                c = get_color(QColor(form.value(fid) or "#808080"), dlg)
                if c.isValid():
                    button.setText(c.name())
                    changed(fid, c.name())
            w.clicked.connect(pick_color)
        elif f.kind == "path":
            w = QWidget()
            row = QHBoxLayout(w)
            row.setContentsMargins(0, 0, 0, 0)
            line = QLineEdit(str(f.value or ""))
            line.textEdited.connect(lambda t, fid=f.id: changed(fid, t))
            more = QPushButton(tr("Choose…"))

            def choose(_=False, fid=f.id, line=line):
                form.press(fid)
                line.setText(str(form.value(fid) or ""))
            more.clicked.connect(choose)
            row.addWidget(line, 1)
            row.addWidget(more)
        elif f.kind == "button":
            w = QPushButton(f.label)
            w.clicked.connect(lambda _=False, fid=f.id: (form.press(fid), [s() for s in syncs]))
            label = ""
        if w is None:
            continue
        if f.tip:
            w.setToolTip(f.tip)
        widgets[f.id] = w
        page_form.addRow(label, w)

        def sync(f=f, w=w):
            w.setEnabled(_truth(f.enabled, form))
            w.setVisible(_truth(f.visible, form))
        syncs.append(sync)
    for s in syncs:
        s()
    box = QDialogButtonBox()
    made = {}
    for bid, text in form.buttons:
        role = (QDialogButtonBox.RejectRole if bid == form.cancel else
                QDialogButtonBox.DestructiveRole if bid == form.discard else
                QDialogButtonBox.AcceptRole)
        b = box.addButton(text, role)
        made[b] = bid
        if bid == form.default:
            b.setDefault(True)
    outer.addWidget(box)
    answer = {"id": form.cancel}

    def clicked(b):
        bid = made.get(b, form.cancel)
        if bid != form.cancel and form.validate is not None:
            err = form.validate(form, bid)
            if err:
                note.setText(err)
                note.setVisible(True)
                return
        answer["id"] = bid
        dlg.done(1 if bid != form.cancel else 0)
    box.clicked.connect(clicked)
    dlg.exec()
    form.finish(answer["id"])
