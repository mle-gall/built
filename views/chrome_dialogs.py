"""Questions, messages and progress drawn by the Built chrome (egui, in the
Metal frame) instead of Qt windows.

Two surfaces, both drawn by ``rust/ingetrazo_render/src/ui.rs``:

* **The sheet**: one question at a time, hung under the top
  bar over the model, with a scrim that takes every click. Unsaved
  changes, errors, confirmations. :func:`ask` answers synchronously, as
  ``QMessageBox.exec()`` does (see "Blocking answers" below).
* **Jobs**: opening, importing, saving, auto-saving, reloading links. A
  capsule in the middle of the top bar with the file, the phase, the
  progress and Cancel when the work can stop. Notices fade below it.
  The capsule retains recent results and notice actions.

Custom questions use the sheet and cancel when there is no frame.
Standard message calls map to chrome notices and questions.

Blocking answers
----------------
:func:`ask` pushes the sheet, then runs a local :class:`QEventLoop`, the
same way ``QDialog.exec()`` does. The view keeps painting, the chrome
reports the click (``sheet:<id>``, from ``wgpu_ui._run``) or the key, and
:func:`answer` stores it and quits the loop. While a sheet is up (or an
exclusive job runs), :class:`_Guard` makes the window modal as Qt would.
It eats key events and shortcuts (Enter, Esc and ⌘D answer the sheet),
mouse presses on any widget except the Metal view, and trackpad gestures.
It greys out the menu bar and refuses window closes. The view's own
pointer events go to the chrome, whose scrim covers the model. Sheets
stack. A question asked while another waits goes on top, and its loop
finishes first, so nested loops always unwind in order.

In the ``built`` shell
----------------------
The shell (``rust/built_app``) owns the event loop. ``app_shell.modal``
gives :func:`ask` a ``live`` that knows the shell's view and a
``QEventLoop`` whose ``exec()`` turns the shell's modal pump, so the
blocking call works there too. Forms (``views/chrome_forms.py``) and
:func:`ask_later` never block at all: they take a continuation
(``done(answer)``) that the chrome's click calls on a later turn, so the
window's ordinary input (typing into a field) keeps flowing through the
host while they are up.
"""
from __future__ import annotations

from dataclasses import dataclass
import itertools
import time
import sys

from core.input_types import NATIVE, QEvent
from core.host_callbacks import Signal
from core.timers import timer
if NATIVE:
    from core.input_codes import StandardButton as QMessageBox
    from app_shell.modal import ShellLoop as QEventLoop
else:
    from PySide6.QtCore import QEventLoop, QObject
    from PySide6.QtGui import QGuiApplication
    from PySide6.QtWidgets import QApplication, QMessageBox, QWidget

#: Cancel if the sheet has still not been drawn after this interval.
WATCHDOG_MS = 1500
_UNDRAWN = object()
_ids = itertools.count(1)


def in_shell() -> bool:
    """True inside the ``built`` binary (its built-in ``built_shell``
    module is there), where no Qt window can show."""
    return "built_shell" in sys.builtin_module_names


# ---- Is the chrome there to answer? ------------------------------------------
def window_of(parent):
    """The main window ``parent`` belongs to, if it is one."""
    if parent is None:
        return None
    if hasattr(parent, "_tool_actions") and hasattr(parent, "_documents") \
            and not hasattr(parent, "isWindow"):
        return parent                   # the shell's window (no QWidget)
    try:
        win = parent.window()
    except (AttributeError, RuntimeError):
        return None
    return win if hasattr(win, "_tool_actions") and hasattr(win, "_documents") else None


def view_of(win):
    doc = getattr(win, "_doc", None)
    return getattr(doc, "stand_in", None) if doc is not None else None


def live(parent):
    """The Metal view whose chrome can show a sheet for ``parent`` now,
    or ``None`` if a blocking question cannot be drawn."""
    if NATIVE or QGuiApplication.platformName() in ("offscreen", "minimal"):
        return None
    win = window_of(parent)
    if win is None or getattr(win, "_closed", False):
        return None
    if parent is not win and parent is not None and not _inside_view_area(parent, win):
        return None
    try:
        if not win.isVisible() or win.isMinimized():
            return None
    except RuntimeError:
        return None
    if QApplication.activeModalWidget() is not None:
        return None     # a Qt dialog is up: the chrome is behind it
    view = view_of(win)
    if (view is None or not getattr(view, "_ui_attached", False)
            or getattr(view, "renderer", None) is None or not view.isVisible()):
        return None
    if getattr(view, "_drawing_frame", False):
        return None     # asked from inside a frame: none could draw the sheet
    return view


def async_host(parent):
    """Resolve an unanswered model's host, including an offscreen Qt window."""
    win = window_of(parent)
    if live(parent) is not None or (not in_shell() and
            QGuiApplication.platformName() in ("offscreen", "minimal")):
        return win
    return None


def _inside_view_area(widget, win) -> bool:
    """A panel of the window (docked, even hidden), not a dialog of its own."""
    w = widget
    while w is not None and w is not win:
        if w.isWindow():
            return False
        w = w.parentWidget()
    return w is win


# ---- The sheet ---------------------------------------------------------------
class _Sheet:
    __slots__ = ("title", "text", "tone", "buttons", "default", "cancel",
                 "discard", "answer", "event_loop", "shown", "blocking", "done", "checklist")

    def __init__(self, title, text, tone, buttons, default, cancel, discard, blocking):
        self.title, self.text, self.tone = title, text, tone
        self.buttons = buttons          # [(id, label, key hint, role)]
        self.default, self.cancel, self.discard = default, cancel, discard
        self.answer = None
        self.event_loop = None
        self.shown = False
        self.blocking = blocking
        self.checklist = None
        self.done = None                # non-blocking: called with the answer


def _sheets(win) -> list:
    return win.__dict__.setdefault("_chrome_sheets", [])


def current_sheet(win):
    sheets = win.__dict__.get("_chrome_sheets") if win is not None else None
    return sheets[-1] if sheets else None


def _roles(buttons, default, cancel, discard):
    out = []
    for bid, label in buttons:
        if bid == default:
            role, hint = 1, "↩"
        elif bid == discard:
            role, hint = 2, "⌘D"
        elif bid == cancel:
            role, hint = 3, "esc"
        else:
            role, hint = 0, ""
        out.append((bid, label, hint, role))
    return out


def ask(parent, title: str, text: str, buttons, *, default=None, cancel=None,
        discard=None, tone: str = "question", checklist=None):
    """Show a sheet and wait for its answer: the id of the button chosen,
    or ``cancel``'s id on Esc or a missing frame.

    ``buttons``: [(id, label)] in reading order. ``default`` answers Enter,
    ``cancel`` answers Esc, ``discard`` answers ⌘D and sits apart on the
    left (macOS's «Don't Save»). ``tone``: question, info, warning, error."""
    view = live(parent)
    if view is None:
        print(f"Built could not show the question: {title}", file=sys.stderr)
        return cancel
    win = window_of(parent)
    sheet = _Sheet(title, text, tone, _roles(buttons, default, cancel, discard),
                   default, cancel, discard, True)
    sheet.checklist = checklist
    loop = QEventLoop()
    sheet.event_loop = loop
    _push(win, sheet)
    frames = getattr(view, "_ui_frames", 0)

    def watchdog():
        # Nothing drew the sheet (no frame came: the window was covered,
        # the surface lost): release the caller.
        if not sheet.shown and getattr(view, "_ui_frames", 0) == frames \
                and sheet.answer is None:
            sheet.answer = _UNDRAWN
            loop.quit()

    dog = timer(win, watchdog)
    dog.start(WATCHDOG_MS)
    try:
        if sheet.answer is None:
            loop.exec()
    finally:
        dog.stop()
        _pop(win, sheet)
    if sheet.answer is _UNDRAWN:
        win.viewport.flash_status(f"Built could not draw the question: {title}")
        return cancel
    if sheet.answer is None:
        return sheet.cancel     # the loop was ended from outside (the app quit)
    return sheet.answer


def ask_later(parent, title: str, text: str, buttons, done, *, default=None,
              cancel=None, discard=None, tone: str = "question") -> None:
    """Post a question and deliver its answer to the continuation."""
    win = async_host(parent)
    if win is None:
        print(f"Built could not show the question: {title}", file=sys.stderr)
        done(cancel)
        return
    sheet = _Sheet(title, text, tone, _roles(buttons, default, cancel, discard),
                   default, cancel, discard, False)
    sheet.done = done
    _push(win, sheet)


@dataclass
class Activity:
    id: str
    title: str
    detail: str
    tone: str
    created: float
    action_label: str
    action: object
    toast: bool


def history(win):
    rows = win.__dict__.setdefault("_chrome_history", [])
    cutoff = time.monotonic() - 45
    rows[:] = [r for r in rows if r.created > cutoff][:30]
    return rows


def _record(win, title, detail, tone, action_label="", action=None, toast=True):
    history(win).insert(0, Activity(str(next(_ids)), title, detail, tone, time.monotonic(),
                                    action_label, action, toast))
    _repaint(win)


def notify(parent, title, text, *, tone="info", button="OK", action_label="", action=None):
    """Post a nonmodal toast and retain its action in the activity history."""
    win = async_host(parent)
    if win is None:
        print(f"{title}: {text}", file=sys.stderr)
        return False
    if "<br" in text:
        from html import unescape
        from ingetrazo_render import notice_text
        text = unescape(notice_text(text))
    _record(win, title, text, tone, action_label, action)
    return True


def offer_recovery(win, path, slot):
    """After opening the requested file, keep the other version available."""
    from core import autosave
    from core.i18n import tr
    from uuid import uuid4
    # Reserve the snapshot before auto-save can reuse this document's slot.
    archived = autosave.discarded_dir() / f"{path.stem}-{uuid4().hex[:12]}{slot.suffix}"
    try:
        slot.rename(archived)
    except OSError:
        archived = slot

    from functools import partial
    from ingetrazo_render import open_recovery_copy
    _record(win, tr("Auto-saved copy available"), path.name, "info",
            tr("Open copy"), partial(open_recovery_copy, win, archived))


def save_failed(win, exc, retry):
    from core.i18n import tr
    if live(win) is None:
        critical(win, tr("Save failed"), str(exc))
        return
    answer = ask(win, tr("Save to another location?"), str(exc),
                 [("cancel", tr("Cancel")), ("retry", tr("Save As…"))],
                 default="retry", cancel="cancel", tone="error")
    if answer == "retry":
        retry()
    elif answer is None:
        critical(win, tr("Save failed"), str(exc))


def opened(bar, win, path):
    """Retain a way back to the project that finished opening."""
    if isinstance(bar, ChromeBar):
        doc = win._doc

        def show():
            if doc in win.documents():
                win.activate_document(doc)
            else:
                win.open_path(path)

        bar.job.end(action_label="Show", action=show)


def close_failed(bar, error):
    if isinstance(bar, ChromeBar):
        bar.job.end(failed=True, detail=str(error))
    bar.close()


def autosave_result(win, name, error):
    if live(win) is not None:
        _record(win, f"Auto-save failed · {name}" if error else f"Auto-saved · {name}",
                error, "error" if error else "info", toast=bool(error))


def update_link_banner(win):
    from core import links
    from core.i18n import tr
    scene = win.viewport.scene
    count = sum(links.link_state(scene, p) == "missing" for p in links.sources(scene))
    win._doc._missing_link_banner = (tr("1 linked file missing") if count == 1 else
                                     tr("{n} linked files missing", n=count) if count else "")
    return count


def sheet_buttons(sheet):
    rows = list(sheet.buttons)
    for i, (label, checked) in enumerate(sheet.checklist or ()):
        rows.append((f"check:{i}", label, "", 5 if checked else 4))
    return rows


def _push(win, sheet) -> None:
    stack = _sheets(win)
    stack.append(sheet)
    _Guard.engage(win)
    _repaint(win)


def _pop(win, sheet) -> None:
    stack = _sheets(win)
    if sheet in stack:
        stack.remove(sheet)
    _Guard.release(win)
    _repaint(win)


def answer(win, bid: str) -> bool:
    """The chrome (or a key) answered the sheet on top."""
    sheet = current_sheet(win)
    if sheet and sheet.checklist and bid.startswith("check:"):
        index = bid[6:]
        if index.isdecimal() and int(index) < len(sheet.checklist):
            i = int(index)
            label, checked = sheet.checklist[i]
            sheet.checklist[i] = (label, not checked)
            _repaint(win)
            return True
    if sheet is None or not any(b[0] == bid for b in sheet.buttons):
        return False
    sheet.answer = bid
    if sheet.blocking:
        if sheet.event_loop is not None:
            sheet.event_loop.quit()
    else:
        _pop(win, sheet)
        if sheet.done is not None:
            sheet.done(bid)
    return True


def sheet_key(win, event) -> bool:
    """Enter = default, Esc = cancel, ⌘D = discard. True: it was the sheet's."""
    from core.input_types import Qt
    sheet = current_sheet(win)
    if sheet is None:
        return False
    key, mods = event.key(), event.modifiers()
    if key in (Qt.Key_Return, Qt.Key_Enter) and sheet.default:
        answer(win, sheet.default)
    elif key == Qt.Key_Escape and sheet.cancel:
        answer(win, sheet.cancel)
    elif key == Qt.Key_D and mods & Qt.ControlModifier and sheet.discard:
        answer(win, sheet.discard)
    return True


def _repaint(win) -> None:
    view = view_of(win)
    if view is not None:
        view._ui_sheet_sig = False      # sent again at the next frame
        view._ui_jobs_sig = False
        view.update()


# ---- Message boxes: QMessageBox's static calls, drawn on the sheet -----------
_STD = [  # (button, label, accept?) in the order they read, left to right
    (QMessageBox.Discard, "Don't Save", False), (QMessageBox.No, "No", False),
    (QMessageBox.NoToAll, "No to All", False), (QMessageBox.Abort, "Abort", False),
    (QMessageBox.Ignore, "Ignore", False), (QMessageBox.Retry, "Retry", True),
    (QMessageBox.Cancel, "Cancel", False), (QMessageBox.Close, "Close", False),
    (QMessageBox.Reset, "Reset", False), (QMessageBox.RestoreDefaults, "Restore Defaults", False),
    (QMessageBox.Apply, "Apply", True), (QMessageBox.Open, "Open", True),
    (QMessageBox.YesToAll, "Yes to All", True), (QMessageBox.SaveAll, "Save All", True),
    (QMessageBox.Save, "Save", True), (QMessageBox.Yes, "Yes", True),
    (QMessageBox.Ok, "OK", True),
]


def _standard(parent, tone, title, text, buttons, default):
    from core.i18n import tr
    if buttons is None:
        buttons = QMessageBox.Yes | QMessageBox.No if tone == "question" else QMessageBox.Ok
    chosen = [(b, label, accept) for b, label, accept in _STD if buttons & b]
    if not chosen:
        chosen = [(QMessageBox.Ok, "OK", True)]
    if len(chosen) == 1 and chosen[0][0] == QMessageBox.Ok:
        notify(parent, title, text, tone=tone)
        return QMessageBox.Ok
    ids = {str(int(b.value)): b for b, _, _ in chosen}
    nobutton = default is None or default == QMessageBox.NoButton
    if nobutton:
        accept = [b for b, _, a in chosen if a]
        default = accept[-1] if accept else chosen[-1][0]
    present = {b for b, _, _ in chosen}
    escape = next((b for b in (QMessageBox.Cancel, QMessageBox.No, QMessageBox.Close)
                   if b in present), None)
    if escape is None and len(chosen) == 1:
        escape = chosen[0][0]
    discard = QMessageBox.Discard if QMessageBox.Discard in present else None
    key = lambda b: str(int(b.value)) if b is not None else None
    got = ask(parent, title, text, [(key(b), tr(label)) for b, label, _ in chosen],
              default=key(default), cancel=key(escape), discard=key(discard), tone=tone)
    return ids.get(got, QMessageBox.NoButton)


def critical(parent, title, text, buttons=QMessageBox.Ok, defaultButton=None):
    return _standard(parent, "error", title, text, buttons, defaultButton)


def warning(parent, title, text, buttons=QMessageBox.Ok, defaultButton=None):
    return _standard(parent, "warning", title, text, buttons, defaultButton)


def information(parent, title, text, buttons=QMessageBox.Ok, defaultButton=None):
    return _standard(parent, "info", title, text, buttons, defaultButton)


def question(parent, title, text, buttons=QMessageBox.Yes | QMessageBox.No, defaultButton=None):
    return _standard(parent, "question", title, text, buttons, defaultButton)


# ---- Modality while a sheet is up --------------------------------------------
class _Guard(object if NATIVE else QObject):
    """What a modal QMessageBox does for its window, for the chrome's sheet
    and exclusive jobs: no shortcut, key or click reaches anything else."""

    def __init__(self, win):
        if not NATIVE:
            super().__init__(win)
        self.win = win
        self.menus = []
        self.count = 0

    @classmethod
    def engage(cls, win) -> None:
        guard = win.__dict__.get("_chrome_guard")
        if guard is None:
            guard = win._chrome_guard = cls(win)
        guard.count += 1
        if guard.count == 1:
            if not NATIVE:
                QApplication.instance().installEventFilter(guard)
            bar = win.menuBar() if hasattr(win, "menuBar") else None
            guard.menus = []
            if bar is not None:
                for action in bar.actions():
                    if action.isEnabled():
                        action.setEnabled(False)
                        guard.menus.append(action)

    @classmethod
    def release(cls, win) -> None:
        guard = win.__dict__.get("_chrome_guard")
        if guard is None or guard.count == 0:
            return
        guard.count -= 1
        if guard.count == 0:
            if not NATIVE:
                QApplication.instance().removeEventFilter(guard)
            for action in guard.menus:
                try:
                    action.setEnabled(True)
                except RuntimeError:
                    pass
            guard.menus = []

    _KEYS = (QEvent.KeyPress, QEvent.KeyRelease, QEvent.ShortcutOverride, QEvent.Shortcut)
    _CLICKS = (QEvent.MouseButtonPress, QEvent.MouseButtonRelease,
               QEvent.MouseButtonDblClick, QEvent.Wheel, QEvent.ContextMenu)

    def eventFilter(self, obj, event) -> bool:  # noqa: N802 (Qt API)
        kind = event.type()
        if kind in self._KEYS:
            if kind == QEvent.ShortcutOverride:
                event.accept()          # the key is "taken": no shortcut fires
            elif current_sheet(self.win) is None and _form_up(self.win) \
                    and kind in (QEvent.KeyPress, QEvent.KeyRelease):
                # A form on the chrome: its fields take the keys.
                from views import wgpu_ui
                view = view_of(self.win)
                if view is not None:
                    wgpu_ui.key(view, event, kind == QEvent.KeyPress)
            elif kind == QEvent.KeyPress and not event.isAutoRepeat():
                if current_sheet(self.win) is not None:
                    sheet_key(self.win, event)
                else:
                    job_key(self.win, event)
            return True
        if kind == QEvent.NativeGesture:
            return True
        if kind in self._CLICKS and isinstance(obj, QWidget):
            return obj is not view_of(self.win)
        return False


def _form_up(win) -> bool:
    return bool(win.__dict__.get("_chrome_forms"))


def engage(win) -> None:
    """Hold the window as a sheet does (views/chrome_forms.py)."""
    _Guard.engage(win)


def release(win) -> None:
    _Guard.release(win)


def modal(win) -> bool:
    """A sheet or an exclusive job holds the window."""
    guard = win.__dict__.get("_chrome_guard") if win is not None else None
    return guard is not None and guard.count > 0


# ---- Jobs: open, import, save, auto-save, link reloads -----------------------
class Job:
    """One piece of work shown in the top bar. ``exclusive`` work (an open,
    an import) holds the window the way Qt's window-modal bar did: the
    model is half there, so nothing else may touch it."""

    def __init__(self, win, label: str, *, exclusive: bool = False, on_cancel=None):
        self.win = win
        self.id = str(next(_ids))
        self.label = label
        self.phase = ""
        self.fraction = -1.0
        self.on_cancel = on_cancel
        self.exclusive = exclusive
        self.cancelled = False
        self._ended = False
        self._last_paint = 0.0
        win.__dict__.setdefault("_chrome_jobs", []).append(self)
        if exclusive:
            _Guard.engage(win)
        self.paint(force=True)

    @property
    def cancellable(self) -> bool:
        return self.on_cancel is not None and not self.cancelled

    def update(self, fraction=None, phase=None) -> None:
        if fraction is not None:
            self.fraction = float(fraction)
        if phase is not None:
            self.phase = phase
        self.paint()

    def cancel(self) -> None:
        if not self.cancellable:
            return
        self.cancelled = True
        self.phase = "Cancelling…"
        callback, self.on_cancel = self.on_cancel, None
        callback()
        self.paint(force=True)

    def paint(self, force: bool = False) -> None:
        """Ask for a frame (at most 30 a second while it reports)."""
        import time
        now = time.perf_counter()
        if not force and now - self._last_paint < 1 / 30:
            return
        self._last_paint = now
        _repaint(self.win)

    def draw_now(self) -> None:
        """A frame now, before blocking work that pumps no events (a save)."""
        view = view_of(self.win)
        if view is not None:
            view._ui_jobs_sig = False
            try:
                view.draw_frame()
            except Exception:  # noqa: BLE001 — a missed frame only loses the chip
                pass

    def end(self, *, failed=False, detail="", action_label="", action=None) -> None:
        """Finish once and optionally retain a result action in the capsule."""
        if self._ended:
            return
        self._ended = True
        jobs = self.win.__dict__.get("_chrome_jobs", [])
        if self in jobs:
            jobs.remove(self)
        if self.exclusive:
            _Guard.release(self.win)
        result = "Cancelled" if self.cancelled else "Failed" if failed else "Done"
        _record(self.win, f"{result} · {self.label.rstrip('…')}", detail,
                "error" if failed else "info", action_label, action, toast=False)


def jobs(win) -> list:
    """The jobs to show: those running, then work polled from elsewhere
    (the background auto-save)."""
    out = list(win.__dict__.get("_chrome_jobs", ()))
    auto = win.__dict__.get("_background_autosave")
    if auto is not None and getattr(auto, "busy", False):
        out.append(_AutosaveJob(auto))
    return out


def exclusive(win) -> bool:
    return any(j.exclusive for j in win.__dict__.get("_chrome_jobs", ()))


def find_job(win, jid):
    return next((j for j in win.__dict__.get("_chrome_jobs", ()) if j.id == jid), None)


def job_key(win, event) -> bool:
    """Esc cancels the exclusive job, when it can stop."""
    from core.input_types import Qt
    if event.key() == Qt.Key_Escape:
        job = next((j for j in reversed(win.__dict__.get("_chrome_jobs", ()))
                    if j.exclusive and j.cancellable), None)
        if job is not None:
            job.cancel()
    return True


class _AutosaveJob:
    """The background auto-save as a quiet job (polled, never cancelled here)."""
    exclusive = False
    cancellable = False
    fraction = -1.0
    phase = ""

    def __init__(self, auto):
        job = getattr(auto, "job", None) or {}
        doc = job.get("doc")
        name = doc.display_name() if doc is not None and hasattr(doc, "display_name") else ""
        from core.i18n import tr
        self.id = "autosave"
        self.label = tr("Auto-saving {name}", name=name).strip() if name else tr("Auto-saving")


class busy:
    """``with busy(win, "Saving x…"):`` around blocking work that reports no
    progress: the chip is drawn before it starts and goes when it ends. A
    no-op when the chrome is not on screen."""

    def __init__(self, parent, label: str, *, output_path=None, exclusive=False):
        self.exclusive = exclusive
        self.output_path = output_path
        self.parent, self.label, self.job = parent, label, None

    def __enter__(self):
        if live(self.parent) is not None:
            self.job = Job(window_of(self.parent), self.label, exclusive=self.exclusive)
            self.job.draw_now()
        return self.job

    def __exit__(self, *exc):
        if self.job is not None:
            label, action = "", None
            if self.output_path and exc[0] is None:
                from pathlib import Path
                from core.desktop_open import open_url
                folder = Path(self.output_path).resolve().parent
                label = "Open folder"
                action = lambda: open_url(folder)
            self.job.end(failed=exc[0] is not None, detail=str(exc[1] or ""),
                         action_label=label, action=action)
        return False


class ChromeBar(object if NATIVE else QObject):
    """The part of ``QProgressDialog`` the loaders use, drawn as a job in
    the top bar instead of a window-modal dialog (``_import_progress``)."""

    canceled = Signal()

    def __init__(self, win, title: str, *, exclusive=True):
        if not NATIVE:
            super().__init__(win)
        self._value = 0
        self._visible = True
        self.job = Job(win, title, exclusive=exclusive)
        self.job.update(0.0, "")

    # QProgressDialog's vocabulary --------------------------------------
    def setValue(self, value: int) -> None:  # noqa: N802
        self._value = int(value)
        if self.job.fraction >= 0:
            self.job.update(fraction=max(0.0, min(1.0, value / 100.0)))

    def value(self) -> int:
        return self._value

    def setLabelText(self, text: str) -> None:  # noqa: N802
        self.job.update(phase=text)

    def setRange(self, low: int, high: int) -> None:  # noqa: N802
        self.job.fraction = -1.0 if high <= low else 0.0
        self.job.paint(force=True)

    def setCancelButtonText(self, _text: str) -> None:  # noqa: N802
        self.set_cancellable(True)

    def set_cancellable(self, on: bool) -> None:
        self.job.on_cancel = self.canceled.emit if on else None
        self.job.paint(force=True)

    def wasCanceled(self) -> bool:  # noqa: N802
        return self.job.cancelled

    def isVisible(self) -> bool:  # noqa: N802
        return self._visible

    def reset(self) -> None:
        pass

    def hide(self) -> None:
        self.close()

    def close(self) -> bool:
        self._visible = False
        self.job.end()
        return True
