"""Deliver dock, one-off export form, and the window's serial export queue."""
from collections import deque
from pathlib import Path

from PySide6.QtCore import QTimer, QUrl
from PySide6.QtGui import QDesktopServices

from core import bim_registry, deliveries
from core.protocol import commands
from views import chrome_dialogs, chrome_forms


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


def dock(view, scene, model):
    values = deliveries.records(scene)
    key = getattr(view, "_delivery_key", None)
    if key not in values:
        key = next(iter(values), None)
    view._delivery_key = key
    rows = [([k, r["name"], r["format"].upper(), r["scope"].capitalize() + (" model" if r["scope"] == "whole" else ""), ""],
             [0, 0, 0, int(k == key), 1, -1, -1, 0, 0, 0]) for k, r in values.items()]
    view.renderer.ui_set_tree(rows, 0)
    view.renderer.ui_set_mode("Deliveries")
    if key is None:
        return
    from views.wgpu_ui import _problem_rules
    from core import structure
    tree = model["tree"]
    nodes = structure._nodes(model["registry"])
    cache_key = (id(scene), scene.content_version)
    cache = getattr(view, "_ui_rules_cache", None)
    if not cache or cache[0] != cache_key:
        cache = cache_key, _problem_rules(scene, model["registry"], nodes, model["recipes"], tree.unfiled)
        view._ui_rules_cache = cache
    r = values[key]
    checks = []
    eligible = None
    if r["scope"] == "node":
        eligible = set(tree.members(r.get("node")))
    elif r["scope"] == "selection":
        eligible = set(scene.selection)
    for severity, rule, label, groups in cache[1]:
        if severity == "Info":
            continue
        count = sum(1 for group in groups if (eligible is None or group in eligible)
                    and (r["scope"] != "visible" or scene.entity_visible(group)))
        if count:
            if rule == "unfiled":
                label = "unfiled object" if count == 1 else "unfiled objects"
            elif rule == "solid":
                label = "open solid" if count == 1 else "open solids"
            checks.append((label, str(count)))
    details = [("Format", r["format"].upper()), ("Scope", tree.nodes.get(r.get("node"), {}).get("name", r["scope"])),
               ("Destination", r["destination"]), ("Last run", r.get("last_run") or "Never")]
    details.extend((k.title(), str(v)) for k, v in r["options"].items())
    actions = [("delivery:edit", "Edit…"), ("delivery:delete", "Delete")]
    sections = [("Delivery", details)]
    if checks:
        sections.append(("Pre-run checks", checks))
    sections.append(("Actions", actions))
    view.renderer.ui_set_inspector(r["name"], sections)


def export_form(win, key=None, one_off=False):
    from core.link_structure import Tree
    from views.chrome_forms import Form, Field
    history = win.viewport.history
    scene = history.scene
    record = deliveries.records(scene)[key] if key else {}
    options = record.get("options", {})
    registry = bim_registry.get(scene)
    nodes = Tree(scene, registry).nodes
    fmt = record.get("format", "ifc")

    def scopes(fmt):
        if fmt == "pdf":
            return [("sheets", "Sheets")]
        choices = [("whole", "Whole model"), ("visible", "Visible"), ("selection", "Selection")]
        if nodes:
            choices.append(("node", "Project tree node"))
        if fmt in ("dxf", "svg"):
            choices.append(("scenes", "Scenes"))
        return choices

    def svg(form):
        return form.value("format") == "svg"

    def changed(form, fid, value):
        if fid == "fill" and value == "picture":
            form.set("scope", "visible")
        if fid == "format":
            choices = scopes(value)
            form.field("scope").options = choices
            if form.value("scope") not in dict(choices):
                form.set("scope", choices[0][0])
            form.field("destination").filters = f"{value.upper()} (*.{value})"
        if fid in ("format", "scope"):
            form.changed()

    def unpack(v):
        opts = {}
        if v["format"] == "ifc" and v["profile"]:
            opts["profile"] = v["profile"]
        if v["format"] == "glb":
            opts["textures"] = v["textures"]
        if v["format"] == "svg":
            opts["scale"] = v["scale"]
            if v["fill"] == "picture":
                opts.update(picture=True, axes=v["axes"])
            elif v["fill"] != "lines":
                opts["faces"] = {"materials": v["fill"] != "default", "textures": v["fill"] == "full",
                                 "as_view": v["as_view"], "shadows": v["shadows"]}
            if v["size"] == "screen":
                opts["paper"] = "screen"
            if v["framed"]:
                opts["frame"] = [v["pan_x"], v["pan_y"], v["zoom"]]
            if v["size"] == "custom":
                opts["paper"] = [v["width"], v["height"]]
        if v["format"] == "stl":
            opts["unit"] = v["unit"]
        if v["scope"] in ("sheets", "scenes") and v["names"].strip():
            opts["names"] = v["names"].splitlines()
        r = {k: v[k] for k in ("name", "format", "scope", "destination")}
        if r["scope"] == "node":
            r["node"] = v["node"]
        return dict(r, options=opts)

    def check(form, answer):
        try:
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

    faces = options.get("faces", {})
    fill = "lines"
    if "faces" in options:
        fill = "full" if faces.get("textures") else "colours" if faces.get("materials") else "default"
    if options.get("picture"):
        fill = "picture"
    paper = options.get("paper")
    size = "screen" if paper == "screen" else "custom" if paper else "scale"
    if size != "custom":
        paper = [297, 210]
    frame = options.get("frame") or [0, 0, 1]
    fields = [
        Field("name", "edit", "Name", record.get("name", "New delivery"), focus=True),
        Field("format", "choice", "Format", fmt, options=[(k, k.upper()) for k in deliveries.FORMATS]),
        Field("scope", "choice", "Scope", record.get("scope", "whole"), options=scopes(fmt)),
        Field("node", "choice", "Project node", record.get("node", next(iter(nodes), "")),
              options=[(k, n["name"]) for k, n in nodes.items()], visible=lambda f: f.value("scope") == "node"),
        Field("names", "edit", "Names (one per line, blank for all)", "\n".join(options.get("names", [])),
              multiline=True, visible=lambda f: f.value("scope") in ("sheets", "scenes")),
        Field("profile", "choice", "IFC profile", options.get("profile", ""),
              options=[("", "Default IFC4")] + [(k, v.get("name", k)) for k, v in registry.get("export_profiles", {}).items()],
              visible=lambda f: f.value("format") == "ifc"),
        Field("textures", "check", "Embed textures", options.get("textures", True),
              visible=lambda f: f.value("format") == "glb"),
        Field("fill", "choice", "Faces", fill, visible=svg,
              options=[("lines", "Lines only"), ("picture", "Viewport picture"), ("full", "Materials and textures"),
                       ("colours", "Material colours"), ("default", "Default colour")]),
        Field("axes", "check", "Show axes", options.get("axes", True),
              visible=lambda f: svg(f) and f.value("fill") == "picture"),
        Field("as_view", "check", "Viewport lighting", faces.get("as_view", True),
              visible=lambda f: svg(f) and f.value("fill") not in ("lines", "picture")),
        Field("shadows", "check", "Shadows", faces.get("shadows", False),
              visible=lambda f: svg(f) and f.value("fill") not in ("lines", "picture")),
        Field("size", "choice", "Paper size", size, visible=svg,
              options=[("scale", "Drawing scale"), ("screen", "Screen size"), ("custom", "Custom paper")]),
        Field("framed", "check", "Adjust framing", "frame" in options, visible=svg),
        Field("pan_x", "number", "Pan across", frame[0], lo=-100, hi=100, decimals=2,
              visible=lambda f: svg(f) and f.value("framed")),
        Field("pan_y", "number", "Pan up", frame[1], lo=-100, hi=100, decimals=2,
              visible=lambda f: svg(f) and f.value("framed")),
        Field("zoom", "number", "Zoom", frame[2], lo=0.01, hi=100, decimals=2,
              visible=lambda f: svg(f) and f.value("framed")),
        Field("width", "number", "Paper width", paper[0], lo=1, hi=10000, suffix=" mm",
              visible=lambda f: svg(f) and f.value("size") == "custom"),
        Field("height", "number", "Paper height", paper[1], lo=1, hi=10000, suffix=" mm",
              visible=lambda f: svg(f) and f.value("size") == "custom"),
        Field("scale", "number", "Scale 1 ∶", options.get("scale", 100), lo=1, hi=100000,
              visible=lambda f: f.value("format") == "svg"),
        Field("unit", "choice", "Units", options.get("unit", "mm"), options=["mm", "cm", "m", "in", "ft"],
              visible=lambda f: f.value("format") == "stl"),
        Field("destination", "path", "Destination", record.get("destination", ""), pick="save",
              filters=f"{fmt.upper()} (*.{fmt})")]
    buttons = [("cancel", "Cancel")]
    if one_off:
        buttons += [("keep", "Keep as delivery"), ("export", "Export")]
    else:
        buttons.append(("save", "Save delivery"))
    form = Form("Export" if one_off else "Delivery", fields, buttons=buttons, default=buttons[-1][0],
                on_done=done, on_change=changed, validate=check)
    from ingetrazo_render import show_delivery_form
    show_delivery_form(win, form, one_off)
    return form


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
        from views.wgpu_sheets import composer_for
        ctx["pdf_writer"] = lambda p, sheets: composer_for(win).export_all_pdf(p, sheets)
    if record["format"] == "svg" and record["options"].get("paper") == "screen":
        viewport = chrome_dialogs.view_of(win) or active
        ctx["screen_mm"] = [viewport.width() * 25.4 / 96, viewport.height() * 25.4 / 96]
    if record["format"] == "svg" and record["options"].get("picture"):
        def picture(axes, frame):
            from core.vista_caras import view_rect
            from views.svg_export import EXPORT_IMAGE_PX, framed
            from contextlib import nullcontext
            renderer = chrome_dialogs.view_of(win) or active
            scene = history.scene
            old = scene.show_axes
            scene.show_axes = old and axes
            try:
                with framed(active.camera, frame) if frame else nullcontext():
                    image = renderer.render_image(EXPORT_IMAGE_PX, overlays=False, sharp_shadows=True)
                    return image, view_rect(active.camera)
            finally:
                scene.show_axes = old
        ctx["svg_picture"] = picture
    steps = deliveries.write_steps(history.scene, record, **ctx)

    def report(fraction):
        job.update(fraction)
        if chrome_dialogs.in_shell():
            from app_shell.modal import tick
            tick()
        else:
            from PySide6.QtWidgets import QApplication
            QApplication.processEvents()
        if job.cancelled or win.viewport.history is not history:
            raise InterruptedError("Delivery cancelled")

    def advance():
        if not job.cancelled and win.viewport.history is not history:
            win.viewport.flash_status(f"Cancelled {record['name']} after the active document changed")
        if not job.cancelled and win.viewport.history is history:
            try:
                from core import progress
                with progress.reporting(report):
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
