# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Evidence for the next "it just disappeared", in the crash log.

faulthandler (main.py) covers a fatal signal in Python's own process. What
it does not see, and what this module writes beside it, cheaply:

* a Rust panic in a native module (``install_panic_log``): PyO3
  turns it into an exception, but a packaged app has no stderr, so the
  message went nowhere — and a panic on a thread of the module's own never
  reaches Python at all;
* the application ending on purpose (``note``, from ``main._exit_now``)
  or through an interpreter exit nobody planned (``note_unplanned_exit``:
  a ``SystemExit`` raised inside a Qt handler ends the process silently);
* a main thread stuck in Python (``StallWatch``): the window stops
  answering, the user force-quits it, and SIGKILL leaves no trace. On
  2026-11-18 a box selection over the ride composite kept the main thread
  in Python for 30 and 95 s; macOS only wrote a hang report without a
  Python frame. The watch dumps every thread's Python stack once per stall.
"""
from __future__ import annotations

import sys
import threading
import time

#: Seconds the main thread may stay busy in Python before its stack is
#: written down (macOS shows the spinning cursor after a few seconds; the
#: force quits came after about 30).
STALL_SECONDS = 15.0

_log = None


def _stamp() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%S")


def attach(log) -> None:
    """Use ``log`` (an open text file, main.py's crash log) from now on."""
    global _log
    _log = log


def note(text: str) -> None:
    """Append one timestamped line to the crash log; never raises."""
    log = _log
    if log is None:
        return
    try:
        log.write(f"[{_stamp()}] {text}\n")
        log.flush()
    except Exception:  # noqa: BLE001 — a black box must not break the app
        pass


def install_panic_log(module) -> bool:
    """Have native ``module`` (ingetrazo_core, ingetrazo_render) append its
    panics to the crash log. False when there is no log or the module is an
    older build without the hook; never raises."""
    path = getattr(_log, "name", None)
    install = getattr(module, "install_panic_log", None)
    if not isinstance(path, str) or install is None:
        return False
    try:
        install(path)
        return True
    except Exception:  # noqa: BLE001 — never stop the start for this
        return False


def note_unplanned_exit() -> None:
    """Register an ``atexit`` line: the normal quit leaves through
    ``os._exit`` (no atexit), so this only speaks when the interpreter
    finalises some other way — a ``SystemExit`` out of a Qt handler, or an
    exception out of ``main``."""
    import atexit
    atexit.register(lambda: note("the interpreter is exiting without the "
                                 "normal quit (SystemExit in a handler?)"))


class StallWatch:
    """A heartbeat on the main thread's event loop and a thread that looks
    at it every few seconds. When the heartbeat is older than ``seconds``
    while the main thread is running Python code (not waiting in the event
    loop — a hidden window's timers may be held back by App Nap), every
    thread's stack goes to the log, once per stall."""

    def __init__(self, log, seconds: float = STALL_SECONDS,
                 idle_code=None, period: float = 1.0):
        self.log = log
        self.seconds = seconds
        self.idle_code = idle_code
        self.period = period
        self.beat = time.monotonic()
        self.main = threading.main_thread().ident
        self.reported = False
        self._stop = threading.Event()
        self.timer = None
        self.thread = None

    def start(self, parent=None):
        from PySide6.QtCore import QTimer
        self.timer = QTimer(parent)
        self.timer.setInterval(int(self.period * 1000))
        self.timer.timeout.connect(self.tick)
        self.timer.start()
        self.thread = threading.Thread(target=self._run, name="stall-watch",
                                       daemon=True)
        self.thread.start()
        return self

    def stop(self):
        self._stop.set()
        if self.timer is not None:
            self.timer.stop()

    def tick(self):
        self.beat = time.monotonic()
        self.reported = False

    def _busy(self) -> bool:
        """The main thread is inside Python code other than the frame that
        runs the event loop."""
        frame = sys._current_frames().get(self.main)
        if frame is None:
            return False
        return self.idle_code is None or frame.f_code is not self.idle_code

    def check(self) -> bool:
        """One look; True when it wrote a report."""
        late = time.monotonic() - self.beat
        if self.reported or late < self.seconds or not self._busy():
            return False
        self.reported = True
        try:
            import faulthandler
            self.log.write(f"\n[{_stamp()}] main thread busy for {late:.0f} s "
                           "(no event loop turn): Python stacks follow\n")
            self.log.flush()
            faulthandler.dump_traceback(self.log, all_threads=True)
            self.log.flush()
        except Exception:  # noqa: BLE001
            pass
        return True

    def _run(self):
        step = max(0.5, min(5.0, self.seconds / 3))
        while not self._stop.wait(step):
            self.check()
