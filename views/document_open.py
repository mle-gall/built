"""Bind the Rust document worker to the activity capsule and UI timer."""
from core.input_types import NATIVE
from core.timers import later

from core.i18n import tr
from ingetrazo_render import chrome_dialogs as cd


def start(win, path, install, *, target=None):
    from ingetrazo_core import DocumentOpen
    pending = win.__dict__.get("_pending_open")
    if pending is not None:
        request = (path, target)
        if request != (pending.path, pending.target) and request not in pending.requests:
            pending.requests.append(request)
            if target is None:
                pending.queue.append(path)
        return True

    worker = DocumentOpen(path) if target is None else DocumentOpen(path, insert=True)
    bar = cd.ChromeBar(win, tr("Opening {name}…", name=path.name), exclusive=target is not None)
    bar.path, bar.target = path, target
    bar.queue, bar.requests = [], []
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
                        phase = (tr("Adding the model to the drawing…") if target is not None
                                 else tr("Sending the model to the graphics card…"))
                        bar.job.update(0.99, phase)
                    return
                staged = worker.take()
                if staged is None:
                    return
                if timer is not None:
                    timer.stop()
                if target is not None:
                    if target not in win._documents:
                        raise ValueError(tr("The drawing was closed during import."))
                    if win._doc is not target:
                        win.activate_document(target)
                elif not win._doc_is_blank():
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
        failures = []
        if target is None:
            win._open_callbacks.pop(path, None)
            failures = win.__dict__.setdefault("_open_failures", {}).pop(path, [])
        if outcome[0] is not False:
            why = outcome[0] or (tr("cancelled") if bar.wasCanceled() or win._open_aborted
                                 else tr("could not be opened"))
            for callback in failures:
                callback(why)
        bar.close()
        bar.poll = None
        if not NATIVE:
            bar.deleteLater()
        win._pending_open = None
        win._open_in_progress = win._opening_document = False
        if win._open_aborted:
            win.__dict__.get("_open_callbacks", {}).clear()
            win.__dict__.get("_open_failures", {}).clear()
            if hasattr(type(win), "closeEvent"):
                later(win, 0, win.close)
        else:
            for queued, destination in bar.requests:
                if destination is None:
                    win.open_path(queued)
                elif destination in win._documents:
                    start(win, queued, install_insertion, target=destination)

    callbacks = (win.__dict__.setdefault("_open_callbacks", {}).setdefault(path, [])
                 if target is None else [])
    timer = None
    if NATIVE or (hasattr(win, "_host") and getattr(win._host, "ns_view", 0)):
        bar.poll = poll
        bar.next_poll = 0.
        win._host.request_frame()
    else:
        from PySide6.QtCore import QTimer
        timer = QTimer(bar)
        timer.timeout.connect(poll)
        timer.start(32)
    return True


def after_open(win, path, callback, failed=None):
    """Apply tab metadata once the matching request actually succeeds;
    ``failed(reason)`` runs instead when it does not (an error, Cancel)."""
    pending = win.__dict__.get("_pending_open")
    if pending and ((path == pending.path and getattr(pending, "target", None) is None)
                    or path in pending.queue):
        win.__dict__.setdefault("_open_callbacks", {}).setdefault(path, []).append(callback)
        if failed is not None:
            win.__dict__.setdefault("_open_failures", {}).setdefault(path, []).append(failed)
    else:
        callback(win._doc)


def pending(win, path) -> bool:
    """Whether ``path`` is opening or queued to open in ``win``."""
    bar = win.__dict__.get("_pending_open")
    return bool(bar) and ((path == bar.path and getattr(bar, "target", None) is None)
                          or path in bar.queue)


def install_skp(win, path, bar, staged):
    """Attach the worker-owned drawing, undo history and prepared geometry."""
    scene, prepared, history, empty = staged
    vp, doc = win.viewport, win._doc
    if getattr(doc, "_renderer_wgpu", False) and not hasattr(win, "_host"):
        doc._awaiting_picture = True
        doc._after_picture = []
    history.max_steps = vp.history.max_steps
    vp.set_document(scene, history)
    win._import_name = path.name
    win._autosaved_version = None
    win._saved_version = -1
    if not empty:
        win._on_zoom_extents()
    if hasattr(win, "_host"):
        win.view.set_scene(scene, opening=True, prepared=prepared)
        win.view._shell_resident_version = scene.version
        for callback in win.view.after_open:
            win._host.after_picture(callback)
        win.view.after_open = []
    elif getattr(doc, "_renderer_wgpu", False):
        display = doc.stand_in
        display._refresh_timer.stop()
        display._rebuild(progress=bar.job.update, prepared=prepared)
        doc._after_picture.extend(display.after_open)
        display.after_open = []
        display._refresh_timer.start(0)
    if getattr(win, "_session_path", None) and not win._opening_session:
        win._refresh_worksession()
    win._refresh_scene_tabs()
    win._update_title()
    vp.update()
    message = (tr("{name} has no geometry — nothing to import.") if empty
               else tr("Imported {name}"))
    win.statusBar().showMessage(message.format(name=path.name), 8000 if empty else 3000)
    return True


def install_insertion(win, path, bar, command):
    history = win.viewport.history
    history.execute(command)
    if history.last_error or history.last_rejected:
        raise ValueError(history.last_error or history.last_rejected)
    win._import_name = path.name
    if not command.empty:
        win._on_zoom_extents()
    win._refresh_scene_tabs()
    win._update_title()
    win.viewport.update()
    message = (tr("{name} has no geometry — nothing to import.") if command.empty
               else tr("Imported {name}"))
    win.statusBar().showMessage(message.format(name=path.name), 8000 if command.empty else 3000)
    return True
