"""Saved deliveries, one-off export form, and the window's serial export queue."""
import json
from collections import deque
from contextlib import contextmanager
from pathlib import Path

from PySide6.QtCore import QTimer, QUrl
from PySide6.QtGui import QDesktopServices

from core import bim_registry, deliveries
from core.protocol import commands
from views import chrome_dialogs, chrome_forms


PREVIEW_JOB_DELAY = 0.5


def bind(win):
    """Bind protocol Run to the open document's job capsule once."""
    from weakref import ref
    history = win.viewport.history
    if getattr(history, "delivery_runner", None) is not None:
        return
    window_ref, history_ref = ref(win), ref(history)

    def submit(key, record):
        window = window_ref()
        if window is None or window.viewport.history is not history_ref():
            raise ValueError("Activate this document to run its delivery")
        return enqueue(window, key, record).id

    history.delivery_runner = submit


def export_form(win, key=None, one_off=False, initial_format="ifc"):
    from core.link_structure import Tree
    from views.chrome_forms import Form, Field
    history = win.viewport.history
    scene = history.scene
    record = deliveries.records(scene)[key] if key else {}
    options = record.get("options", {})
    registry = bim_registry.get(scene)
    nodes = Tree(scene, registry).nodes
    fmt = record.get("format", initial_format)

    from ingetrazo_core import delivery_field_visible, delivery_record, delivery_scopes

    def scopes(fmt):
        return delivery_scopes(fmt, bool(nodes))

    def shows(fid):
        return lambda f: delivery_field_visible(fid, f.value("format"), f.value("scope"))

    from ingetrazo_render import SvgOptions
    view = chrome_dialogs.view_of(win)
    screen = (view.width() * 25.4 / 96, view.height() * 25.4 / 96) if view else (297., 210.)
    svg = SvgOptions(json.dumps(options), screen)

    def changed(form, fid, value):
        if fid == "format":
            choices = scopes(value)
            form.field("scope").options = choices
            if form.value("scope") not in dict(choices):
                form.set("scope", choices[0][0])
            form.field("destination").filters = f"{value.upper()} (*.{value})"
        if fid in ("format", "scope"):
            form.changed()

    def unpack(v):
        return json.loads(delivery_record(json.dumps(v, default=str),
                                          svg.options_json() if v.get("format") == "svg" else "{}"))

    def check(form, answer):
        try:
            if form.value("format") == "svg" and (error := svg.validation_error()):
                return error
            deliveries.validate(unpack(form.values()))
        except ValueError as exc:
            return str(exc)

    def done(answer, v):
        if answer == "cancel" or win.viewport.history is not history:
            return
        r = unpack(v)
        delivery_key = key
        if key:
            commands.run(history, "edit_delivery", key=key, changes=r)
        elif answer != "export":
            delivery_key = commands.run(history, "create_delivery", delivery=r)["key"]
        if one_off:
            enqueue(win, delivery_key, r)
        view = chrome_dialogs.view_of(win)
        if view:
            view._delivery_key = delivery_key
            view._ui_tree_sig = None
            view.update()

    fields = [
        Field("name", "edit", "Name", record.get("name", "New delivery"), focus=True),
        Field("format", "choice", "Format", fmt, options=[(k, k.upper()) for k in deliveries.FORMATS]),
        Field("scope", "choice", "Scope", record.get("scope", "whole"), options=scopes(fmt)),
        Field("node", "choice", "Project node", record.get("node", next(iter(nodes), "")),
              options=[(k, n["name"]) for k, n in nodes.items()], visible=shows("node")),
        Field("names", "edit", "Names (one per line, blank for all)", "\n".join(options.get("names", [])),
              multiline=True, visible=shows("names")),
        Field("profile", "choice", "IFC profile", options.get("profile", ""),
              options=[("", "Default IFC4")] + [(k, v.get("name", k)) for k, v in registry.get("export_profiles", {}).items()],
              visible=shows("profile")),
        Field("textures", "check", "Embed textures", options.get("textures", True),
              visible=shows("textures")),
        Field("unit", "choice", "Units", options.get("unit", "mm"), options=["mm", "cm", "m", "in", "ft"],
              visible=shows("unit")),
        Field("destination", "path", "Destination", record.get("destination", ""), pick="save",
              filters=f"{fmt.upper()} (*.{fmt})")]
    buttons = [("cancel", "Cancel")]
    if one_off:
        buttons += [("keep", "Keep as delivery"), ("export", "Export")]
    else:
        buttons.append(("save", "Save delivery"))
    form = Form("Export" if one_off else "Delivery", fields, buttons=buttons, default=buttons[-1][0],
                on_done=done, on_change=changed, validate=check)

    def preview(ticket):
        from views.svg_export import preview_pixels
        active = win._active_view() if hasattr(win, "_active_view") else win.viewport
        record = unpack(form.values())
        with svg_progress(win, history, "SVG preview",
                          lambda: not form.closed and svg.preview_current(ticket)):
            text = deliveries.preview_svg(history.scene, record, camera=active.camera,
                                          **_svg_context(win, active, history.scene, record, preview=True))
        return preview_pixels(text)

    from ingetrazo_render import show_delivery_form
    show_delivery_form(win, form, one_off, svg, preview)
    return form


def _svg_context(win, active, scene, record, preview=False):
    ctx = {}
    if record["options"].get("paper") == "screen":
        viewport = chrome_dialogs.view_of(win) or active
        ctx["screen_mm"] = [viewport.width() * 25.4 / 96, viewport.height() * 25.4 / 96]
    if record["options"].get("picture"):
        def picture(axes, frame):
            from core.vista_caras import view_rect
            from views.svg_export import EXPORT_IMAGE_PX, PREVIEW_IMAGE_PX, framed
            from contextlib import nullcontext
            renderer = chrome_dialogs.view_of(win) or active
            old = scene.show_axes
            scene.show_axes = old and axes
            try:
                with framed(active.camera, frame) if frame else nullcontext():
                    image = renderer.render_image(PREVIEW_IMAGE_PX if preview else EXPORT_IMAGE_PX, overlays=False, sharp_shadows=True)
                    return image, view_rect(active.camera)
            finally:
                scene.show_axes = old
        ctx["svg_picture"] = picture
    return ctx


@contextmanager
def svg_progress(win, history, label, current=lambda: True):
    """Show slow exports in the job capsule and stop superseded previews."""
    import time
    from core import progress
    started = time.monotonic()
    job = None

    def report(fraction):
        nonlocal job
        if not current() or win.viewport.history is not history:
            raise InterruptedError
        if job is None:
            if time.monotonic() - started < PREVIEW_JOB_DELAY:
                return
            job = chrome_dialogs.Job(win, label, on_cancel=lambda: None)
        _report_job(win, history, job, fraction)
        if not current():
            raise InterruptedError

    try:
        report(0)
        with progress.reporting(report):
            yield
        report(1)
    except InterruptedError:
        if job is not None:
            job.cancel()
        raise
    except Exception as exc:
        if job is not None:
            job.end(failed=True, detail=str(exc))
        raise
    finally:
        if job is not None:
            job.end()


def _report_job(win, history, job, fraction):
    job.update(fraction)
    if chrome_dialogs.in_shell():
        from app_shell.modal import tick
        tick()
    else:
        from PySide6.QtWidgets import QApplication
        QApplication.processEvents()
    if job.cancelled or win.viewport.history is not history:
        raise InterruptedError("Delivery cancelled")


def _write_pdf(win, path, sheets) -> list:
    """The sheets' PDF from the Rust writer (sheets::pdf), the frames
    rendered by the window's view; the composer's only without one."""
    view = getattr(getattr(win, "_doc", None), "stand_in", None)
    if view is None or getattr(view, "renderer", None) is None:
        from views.wgpu_sheets import composer_for
        return composer_for(win).export_all_pdf(path, sheets)
    from views import wgpu_sheet_frames
    scene = win.viewport.history.scene
    wanted = None if sheets is None else {s._doc_key for s in sheets}
    comps = [c for c in scene.compositions if wanted is None or c._doc_key in wanted]
    return wgpu_sheet_frames.export_pdf(view, comps, path, wgpu_sheet_frames.Ground(lambda: None),
                                        getattr(win, "_current_path", None))


def enqueue(win, key, record):
    history = win.viewport.history
    queue = win.__dict__.setdefault("_delivery_queue", deque())
    job = chrome_dialogs.Job(win, record["name"], on_cancel=lambda: None)
    job.update(0, "Queued")
    active = win._active_view() if hasattr(win, "_active_view") else win.viewport
    from copy import copy
    ctx = {"camera": copy(active.camera)}
    current = getattr(win, "_current_path", None)
    if current:
        ctx["project_directory"] = Path(current).parent
    if record["format"] == "pdf":
        ctx["pdf_writer"] = lambda p, sheets: _write_pdf(win, p, sheets)
    if record["format"] == "svg":
        ctx.update(_svg_context(win, active, history.scene, record))
    steps = deliveries.write_steps(history.scene, record, **ctx)

    def advance():
        if not job.cancelled and win.viewport.history is not history:
            win.viewport.flash_status(f"Cancelled {record['name']} after the active document changed")
        if not job.cancelled and win.viewport.history is history:
            try:
                from core import progress
                with progress.reporting(lambda f: _report_job(win, history, job, f)):
                    fraction, phase = next(steps)
            except StopIteration:
                if key in deliveries.records(history.scene):
                    deliveries.finish(history, key)
                dest = Path(record["destination"]).expanduser()
                folder = (Path(current).parent / dest if current else dest).resolve().parent
                win._delivery_result = (record["name"], folder)
                from views import chrome_dialogs
                chrome_dialogs._record(
                    win, f"Delivered · {record['name']}", str(folder), "info", "Open folder",
                    lambda f=folder: QDesktopServices.openUrl(QUrl.fromLocalFile(str(f))))
            except InterruptedError:
                pass
            except Exception as exc:
                from views import chrome_dialogs
                chrome_dialogs._record(win, f"Failed · {record['name']}", str(exc), "error")
            else:
                job.update(fraction, phase)
                job.draw_now()
                QTimer.singleShot(0, advance)
                return
        steps.close()
        job.end()
        queue.popleft()
        if queue:
            QTimer.singleShot(0, queue[0])

    queue.append(advance)
    if len(queue) == 1:
        QTimer.singleShot(0, advance)
    return job


def action(win, view, arg):
    verb, _, given = arg.partition(":")
    key = given or getattr(view, "_delivery_key", None)
    if verb == "new":
        export_form(win)
    elif verb == "folder":
        result = win.__dict__.pop("_delivery_result", None)
        if result:
            QDesktopServices.openUrl(QUrl.fromLocalFile(str(result[1])))
    elif key in deliveries.records(win.viewport.scene):
        if verb == "select":
            view._delivery_key = key
        elif verb == "run":
            view._delivery_key = key
            commands.run(win.viewport.history, "run_delivery", key=key)
        elif verb == "edit":
            export_form(win, key)
        elif verb == "delete":
            commands.run(win.viewport.history, "delete_delivery", key=key)
    if view:
        view._ui_tree_sig = None
        view.update()
