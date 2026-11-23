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
* Offscreen Qt hosts expose the same form model. A missing host or a
  failed frame cancels the form and reports the failure.

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
        """An edit from the chrome: typed into its value."""
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


class OperationForm(Form):
    """Adapt a Rust operation model to the shared chrome form transport."""

    def __init__(self, title, model, done, *, note=""):
        super().__init__(title, [], note=note, width=480,
                         validate=self._range_error,
                         on_done=lambda answer, values: done(values) if answer == "ok" else None)
        self.model = model

    def _range_error(self, *_):
        bounds = self.model.invalid_range()
        if bounds is not None:
            return tr("Between {lo} and {hi}.", lo=bounds[0], hi=bounds[1])

    def receive(self, fid, raw):
        self.model.receive(fid, raw)
        self.changed()

    def values(self):
        return self.model.values()

    def wire(self):
        seq, title, note, width, _, buttons = super().wire()
        rows = [(fid, kind, tr(label), text, value, lo, hi, options, flags, tr(tip))
                for fid, kind, label, text, value, lo, hi, options, flags, tip
                in self.model.rows()]
        return seq, title, note, width, rows, buttons


# ---- On the window -------------------------------------------------------------
def forms(win) -> list:
    return win.__dict__.setdefault("_chrome_forms", [])


def current(win):
    stack = win.__dict__.get("_chrome_forms") if win is not None else None
    return stack[-1] if stack else None


def show(parent, form: Form) -> str:
    """Post a form model, or cancel the operation if its host is unavailable."""
    from views import chrome_dialogs
    win = chrome_dialogs.async_host(parent)
    if win is None:
        print(f"Built could not show the form: {form.title}", file=sys.stderr)
        form.finish(form.cancel)
        return "none"
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
    view = chrome_dialogs.live(parent)
    if view is not None:
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
    """Cancel a form that was never drawn."""
    from views import chrome_dialogs
    if chrome_dialogs.in_shell():
        return
    from PySide6.QtCore import QTimer
    frames = getattr(view, "_ui_frames", 0)

    def check():
        if form.closed or form.shown or getattr(view, "_ui_frames", 0) != frames:
            return
        win.viewport.flash_status(f"Built could not draw the {form.title} form.")
        form.close()
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
def ask_text(parent, title, label, text="", done=None, *, multiline=False, ok=None, note=""):
    """A name prompt: ``done(text)`` or ``done(None)`` on Cancel."""
    form = Form(title, [Field("text", "edit", label, text, focus=True, multiline=multiline,
                              full=multiline)],
                note=note, buttons=[("cancel", tr("Cancel")), ("ok", ok or tr("OK"))],
                width=440 if not multiline else 520,
                on_done=lambda a, v: done(v["text"] if a == "ok" else None) if done else None)
    show(parent, form)


def ask_number(parent, title, label, value, lo, hi, decimals=0, done=None, *, suffix="", step=1, note=""):
    """``QInputDialog.getInt`` (decimals 0) / ``getDouble``: ``done(value)``
    or ``done(None)``."""
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
    on the turn the chrome delivers it."""
    def step(value=None, first=False):
        try:
            req = next(gen) if first else gen.send(value)
        except StopIteration:
            return
        req.start(step)
    step(first=True)
