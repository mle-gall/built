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
                    for callback in callbacks:
                        callback(win._doc)
        except Exception as exc:
            cd.close_failed(bar, exc)
            cd.notify(win, tr("Open failed"), str(exc), tone="error")
            import logging
            logging.exception("Opening %s", path)
        if timer is not None:
            timer.stop()
        win._open_callbacks.pop(path, None)
        bar.close()
        bar.poll = None
        bar.deleteLater()
        win._pending_open = None
        win._open_in_progress = win._opening_document = False
        if win._open_aborted:
            win._open_callbacks.clear()
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


def after_open(win, path, callback):
    """Apply tab metadata once the matching request actually succeeds."""
    pending = win.__dict__.get("_pending_open")
    if pending and (path == pending.path or path in pending.queue):
        win._open_callbacks.setdefault(path, []).append(callback)
    else:
        callback(win._doc)
