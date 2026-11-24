"""Questions that wait for their answer, and work that reports progress,
without Qt's event loop.

``views/chrome_dialogs.ask`` shows the egui sheet and then runs a local
``QEventLoop`` until the chrome reports the click, as ``QDialog.exec()``
does. In the shell Qt's loop sees no input and draws nothing (winit owns
the window), so :func:`install` gives chrome_dialogs two shell pieces:

* ``live(parent)``: the shell's view when its chrome can draw the sheet
  (the offscreen platform name made the original say "never").
* ``QEventLoop`` → :class:`ShellLoop`, whose ``exec()`` is
  :func:`run_until`: AppKit's events through the modal pump
  (rust/built_app/src/native.rs ``pump``), the window's own input handed
  to the chrome only (the sheet's buttons, Enter/Esc/⌘D, a job's Cancel),
  Qt's posted events and timers, and a frame whenever one is wanted, until
  the answer comes.

The same turn (:func:`tick`, never blocking) is what a long job's progress
callback calls (opening, importing), so the capsule moves and its Cancel
works while the main thread is busy. Every other input waits: the model is
the sheet's, or the job's, as with Qt's window-modal dialogs.
"""
from __future__ import annotations

import time

from app_shell import native, qt_host

_HOST = None
#: How deep the waits are nested (a question asked while one waits).
DEPTH = 0
_LAST_TICK = [0.0]


def host():
    return _HOST


def install(host) -> None:
    """Point chrome_dialogs at this host (one per process)."""
    global _HOST
    _HOST = host
    from views import chrome_dialogs as cd
    if getattr(cd, "_shell_installed", False):
        return
    original_live = cd.live

    def live(parent):
        h = _HOST
        if h is None:
            return original_live(parent)
        win = cd.window_of(parent)
        if win is None or win is not h.window or getattr(win, "_closed", False):
            return None
        view = h.view
        if (view is None or not getattr(view, "_ui_attached", False)
                or getattr(view, "renderer", None) is None or not h.ns_view):
            return None
        if getattr(view, "_drawing_frame", False):
            return None
        return view

    cd.live = live
    cd.QEventLoop = ShellLoop
    cd._shell_installed = True


class ShellLoop:
    """``QEventLoop`` for chrome_dialogs.ask: exec() pumps until quit()."""

    def __init__(self, *_args):
        self._done = False

    def exec(self):
        answer = _scripted_answer()
        if answer is not None:
            _answer_when_drawn(self, answer)
        run_until(lambda: self._done)
        return 0

    exec_ = exec

    def quit(self):
        self._done = True

    def exit(self, *_):
        self._done = True

    def isRunning(self):
        return not self._done


def run_until(done, timeout_s: float | None = None) -> bool:
    """Turn the modal pump until ``done()``; False when it could not wait
    (no window: headless, tests) or ``timeout_s`` ran out."""
    global DEPTH
    h = _HOST
    if h is None or not h.ns_view:
        return False
    start = time.monotonic()
    DEPTH += 1
    try:
        while not done():
            if timeout_s is not None and time.monotonic() - start > timeout_s:
                return False
            _turn(h, 16.0)
            if getattr(h, "quit_now", False):
                return False
        return True
    finally:
        DEPTH -= 1


def tick(min_interval: float = 1 / 30) -> None:
    """One turn without waiting, at most ``1/min_interval`` a second: the
    progress callback of an open or an import."""
    h = _HOST
    if h is None or not h.ns_view:
        return
    now = time.monotonic()
    if now - _LAST_TICK[0] < min_interval:
        return
    _LAST_TICK[0] = now
    _turn(h, 0.0)


def _turn(h, timeout_ms):
    for ev in native.pump(h.ns_view, timeout_ms):
        dispatch(h, ev)
    qt_host.pump()
    h.modal_frame()


def dispatch(h, ev) -> None:
    """One input event during a wait: the chrome only."""
    from PySide6.QtCore import Qt
    from views import chrome_dialogs, wgpu_ui
    from app_shell.events import KeyEvent, MouseEvent
    kind, x, y, button, mods, key, text = ev
    view = h.view
    win = h.window
    if kind in ("press", "release", "move"):
        if view is None or not getattr(view, "_ui_attached", False):
            return
        held = h.input_state.modal_pointer(kind, x, y, button, mods)
        h._guard("modal pointer", wgpu_ui.pointer, view, kind,
                 MouseEvent(kind, x, y, button, held, mods))
        h.request_frame()
    elif kind == "key":
        h.input_state.modifiers = mods
        event = KeyEvent(True, key, mods, text)
        if chrome_dialogs.current_sheet(win) is not None:
            h._guard("sheet key", chrome_dialogs.sheet_key, win, event)
        elif key == Qt.Key_Escape.value:
            h._guard("job key", chrome_dialogs.job_key, win, event)
        h.request_frame()


# ---- scripted runs: answering a sheet the way a user does --------------------
#: AppKit key codes for the answers a script may give.
_KEYS = {"return": (36, "\r", False), "escape": (53, "\x1b", False), "cmd-d": (2, "d", True)}
_ASKED = [0]


def _scripted_answer():
    """The next answer of ``BUILT_ANSWERS`` (comma-separated: return,
    escape, cmd-d, or id:<button>), or None when none is scripted."""
    import os
    queue = os.environ.get("BUILT_ANSWERS")
    if not queue:
        return None
    items = [a.strip() for a in queue.split(",") if a.strip()]
    if not items:
        return None
    os.environ["BUILT_ANSWERS"] = ",".join(items[1:])
    return items[0]


def _answer_when_drawn(loop, answer):
    """Once the sheet has been on screen for two frames: a picture of it
    (``BUILT_SHOTS``), then the answer, as a key posted to this app's own
    queue (it comes back through the pump) or a button id."""
    from views import chrome_dialogs as cd
    h = _HOST
    win = h.window
    sheet = cd.current_sheet(win)
    frames = [0]
    _ASKED[0] += 1
    n = _ASKED[0]

    def ready():
        if loop._done:
            return True
        if sheet is not None and sheet.shown:
            frames[0] += 1
            h.request_frame()
        return frames[0] >= 3

    run_until(ready, timeout_s=5.0)
    if loop._done:
        return
    shots = __import__("os").environ.get("BUILT_SHOTS")
    if shots:
        title = (sheet.title if sheet is not None else "sheet").lower()
        slug = "".join(c if c.isalnum() else "-" for c in title).strip("-")[:40]
        capture(h, f"{shots}/sheet-{n}-{slug}.png")
    if answer.startswith("id:"):
        cd.answer(win, answer[3:])
        return
    code, chars, command = _KEYS.get(answer, _KEYS["escape"])
    if not native.post_key(h.ns_view, code, chars, command):
        cd.answer(win, sheet.default if answer == "return" else sheet.cancel)


def capture(h, path) -> bool:
    """The frame as a PNG (the shell's own capture: the renderer's surface,
    never the screen)."""
    renderer = h.view.renderer
    if renderer is None:
        return False
    renderer.request_capture()
    h.frame()
    try:
        return renderer.save_capture(path)
    except OSError:
        return False
