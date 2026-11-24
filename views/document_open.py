"""Bind the Rust document worker to the activity capsule and UI timer."""
from PySide6.QtCore import QTimer

from core.i18n import tr
from views import chrome_dialogs as cd


def start(win, path, install):
    from ingetrazo_core import DocumentOpen
    pending = win.__dict__.get("_pending_open")
    if pending is not None:
        if path != pending.path and path not in pending.queue:
            pending.queue.append(path)
        return True

    worker = DocumentOpen(path)
    bar = cd.ChromeBar(win, tr("Opening {name}…", name=path.name), exclusive=False)
    bar.path, bar.queue = path, []
    bar.canceled.connect(worker.cancel)
    bar.set_cancellable(True)
    win._pending_open = bar
    win._open_in_progress = True
    win._open_aborted = False

    announced = False
    outcome = [None]       # why it did not come in, for the failure callbacks

    def poll():
        nonlocal announced
        if win._open_aborted:
            bar.job.cancel()
        try:
            if not bar.wasCanceled():
                if not announced:
                    bar.job.update(*worker.progress())
                    if worker.ready():
                        announced = True
                        bar.job.update(0.99, tr("Sending the model to the graphics card…"))
                    return
                staged = worker.take()
                if staged is None:
                    return
                if timer is not None:
                    timer.stop()
                if not win._doc_is_blank():
                    win._new_document()
                win._opening_document = True
                if install(win, path, bar, staged):
                    win._dismiss_home()
                    outcome[0] = False
                    for callback in callbacks:
                        callback(win._doc)
        except Exception as exc:
            outcome[0] = str(exc) or type(exc).__name__
            cd.close_failed(bar, exc)
            cd.notify(win, tr("Open failed"), str(exc), tone="error")
            import logging
            logging.exception("Opening %s", path)
        if timer is not None:
            timer.stop()
        win._open_callbacks.pop(path, None)
        failures = win.__dict__.setdefault("_open_failures", {}).pop(path, [])
        if outcome[0] is not False:
            why = outcome[0] or (tr("cancelled") if bar.wasCanceled() or win._open_aborted
                                 else tr("could not be opened"))
            for callback in failures:
                callback(why)
        bar.close()
        bar.poll = None
        bar.deleteLater()
        win._pending_open = None
        win._open_in_progress = win._opening_document = False
        if win._open_aborted:
            win._open_callbacks.clear()
            win._open_failures.clear()
            if hasattr(type(win), "closeEvent"):
                QTimer.singleShot(0, win, win.close)
        else:
            for queued in bar.queue:
                win.open_path(queued)

    callbacks = win.__dict__.setdefault("_open_callbacks", {}).setdefault(path, [])
    timer = None
    if hasattr(win, "_host") and getattr(win._host, "ns_view", 0):
        bar.poll = poll
        bar.next_poll = 0.
        win._host.request_frame()
    else:
        timer = QTimer(bar)
        timer.timeout.connect(poll)
        timer.start(32)
    return True


def after_open(win, path, callback, failed=None):
    """Apply tab metadata once the matching request actually succeeds;
    ``failed(reason)`` runs instead when it does not (an error, Cancel)."""
    pending = win.__dict__.get("_pending_open")
    if pending and (path == pending.path or path in pending.queue):
        win._open_callbacks.setdefault(path, []).append(callback)
        if failed is not None:
            win.__dict__.setdefault("_open_failures", {}).setdefault(path, []).append(failed)
    else:
        callback(win._doc)


def pending(win, path) -> bool:
    """Whether ``path`` is opening or queued to open in ``win``."""
    bar = win.__dict__.get("_pending_open")
    return bool(bar) and (path == bar.path or path in bar.queue)
