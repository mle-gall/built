# SPDX-License-Identifier: GPL-3.0-or-later
"""Blender panel model for the renderer-owned extension UI."""
from pathlib import Path

from core import render_blender as rb
from core.desktop_open import open_url
from core.settings import settings as QSettings

_SETTINGS = "render/"


def _state(app) -> dict:
    """``{"ambience", "sun_scale", "lights"}`` from the document, cleaned."""
    raw = app.document_data({}) or {}
    if not isinstance(raw, dict):
        raw = {}
    amb = raw.get("ambience", "day")
    try:
        sun = max(0.1, min(2.0, float(raw.get("sun_scale", 1.0))))
    except (TypeError, ValueError):
        sun = 1.0
    return {"ambience": amb if amb in rb.AMBIENCES else "day",
            "sun_scale": sun,
            "lights": rb.clean_lights(raw.get("lights", []))}


def _store(app, st: dict) -> None:
    """Write back — one undo step, the document becomes unsaved."""
    app.set_document_data({"ambience": st["ambience"],
                           "sun_scale": st["sun_scale"],
                           "lights": st["lights"]})


def _render_result(app, path, following, save, folder, follow):
    from ingetrazo_render import RenderResult
    from core.plugin_panels import show_utility
    return show_utility(app.window, app.key + ".result",
                        RenderResult(str(path), following, save, folder, follow))


class BlenderPanel:
    """Blender job controller for the host-drawn panel."""
    id, title, slot = "render", "Blender Render", "dock"

    def __init__(self, app, invalidate):
        self.app, self.invalidate = app, invalidate
        settings = QSettings()
        engine = settings.value(_SETTINGS + "engine", "eevee")
        quality = settings.value(_SETTINGS + "quality", 1, type=int)
        width = settings.value(_SETTINGS + "width", 1920, type=int)
        self.opts = dict(engine=engine if engine in ("cycles", "eevee") else "eevee",
                         quality=["Draft", "Preview", "Final"][max(0, min(2, quality))],
                         width=max(64, min(16384, width)), height=1080,
                         ground=str(settings.value(_SETTINGS + "ground", "1")) != "0",
                         keep_blend=str(settings.value(_SETTINGS + "blend", "0")) != "0",
                         binary=str(settings.value(rb.SETTINGS_KEY, "") or ""))
        self.proc = self.job = self.image = self.selected = None
        self.scene = app.scene
        host = vars(app.window).get("_host")
        if host is not None:
            self.sync_timer = host.timers.timer(self._sync_camera, 500, True)
        else:
            from PySide6.QtCore import QTimer
            self.sync_timer = QTimer(app.window)
            self.sync_timer.setInterval(500)
            self.sync_timer.timeout.connect(self._sync_camera)
        self.last_camera = self.observed_camera = None
        self.lines = []
        self.status = "Ready"
        self.fraction = 0.
        self.found = rb.find_blender(self.opts["binary"] or None)
        if not self.found:
            self.status = "Choose the Blender executable to render"

    def sections(self):
        from core.plugin_panels import field, item, section
        state = _state(self.app)
        opts = self.opts
        lights = state["lights"]
        light_rows = [dict(key=str(i), cells=[lt["name"] or lt["kind"]]) for i, lt in enumerate(lights)]
        selected = int(self.selected) if self.selected is not None else -1
        editing = 0 <= selected < len(lights)
        details = []
        if editing:
            light = lights[selected]
            details = [field("name", "Name", "text", light["name"]),
                       field("on", "Enabled", "toggle", light["on"]),
                       field("power", "Power", "number", light["power"], min=0, max=1e6, unit=" W"),
                       field("kelvin", "Temperature", "number", light["kelvin"] or rb.DEFAULT_KELVIN,
                             min=rb.MIN_KELVIN, max=rb.MAX_KELVIN, unit=" K")]
            if light["kind"] == "spot":
                details.append(field("angle", "Cone", "number", light["angle"], min=1, max=179, unit="°"))
            details.append(item("button", "remove", "Remove light"))
        result = [item("progress", "activity", self.status, fraction=self.fraction),
                  item("button", "render", "Render", enabled=self.found is not None and self.proc is None)]
        if self.proc:
            result.append(item("button", "cancel", "Cancel render"))
        if self.image:
            result.extend([item("image", "preview", path=str(self.image)),
                           item("button", "preview_result", "Open preview…"),
                           item("button", "save", "Save PNG…"),
                           field("sync", "Follow camera", "toggle", self.sync_timer.isActive())])
        if self.lines:
            result.append(item("log", "worker", lines=self.lines))
        return [section("output", "Output",
                        field("binary", "Blender", "path", opts["binary"] or (self.found.where if self.found else "")),
                        field("engine", "Engine", "choice", opts["engine"], options=["eevee", "cycles"]),
                        field("quality", "Quality", "choice", opts["quality"], options=["Draft", "Preview", "Final"]),
                        field("width", "Width", "number", opts["width"], min=64, max=16384, unit=" px"),
                        field("height", "Height", "number", opts["height"], min=64, max=16384, unit=" px"),
                        field("ground", "Ground plane", "toggle", opts["ground"]),
                        field("keep_blend", "Keep .blend", "toggle", opts["keep_blend"])),
                section("lighting", "Lighting",
                        field("ambience", "Ambience", "choice", state["ambience"], options=list(rb.AMBIENCES)),
                        field("sun_scale", "Sun intensity", "number", state["sun_scale"], min=0.1, max=2),
                        item("list", "lights", columns=["Scene lights"], rows=light_rows, selection=self.selected),
                        item("button", "point", "Place point"), item("button", "spot", "Place spot"),
                        item("button", "aim", "Aim…", enabled=editing), *details),
                section("job", "Render", *result)]

    def event(self, control, event, value):
        if event == "commit" and control in self.opts:
            self.opts[control] = int(value) if control in ("width", "height") else value
            if control == "binary":
                QSettings().setValue(rb.SETTINGS_KEY, value)
                self.found = rb.find_blender(value)
        elif event == "commit" and control in ("sun_scale", "ambience"):
            _store(self.app, {**_state(self.app), control: value})
        elif control == "sync":
            if value:
                self.last_camera = rb.camera_key(self.app.viewport.camera)
                self.sync_timer.start()
            else:
                self.sync_timer.stop()
        elif control in ("name", "power", "kelvin", "angle", "on", "remove"):
            st = _state(self.app)
            idx = int(self.selected)
            if control == "remove":
                del st["lights"][idx]
                self.selected = None
            else:
                st["lights"][idx][control] = value
            _store(self.app, st)
        elif control == "lights":
            self.selected = value
        elif control in ("point", "spot", "aim"):
            self._pick(control)
        elif control == "render" and self.proc is None and self.found:
            self._render()
        elif control == "cancel" and self.job:
            self.job.cancel()
        elif control == "preview_result" and self.image:
            self._show_result()
        elif control == "save" and self.image:
            import shutil
            from views.filedialogs import file_dialogs
            path, _ = file_dialogs.getSaveFileName(self.app.window, "Save image", "render.png", "PNG (*.png)")
            if path:
                shutil.copyfile(self.image, path)

    def _show_result(self):
        if self.app.scene is self.scene:
            _render_result(self.app, self.image, self.sync_timer.isActive(),
                           lambda: self.event("save", "invoke", None),
                           lambda: open_url(self.image.parent),
                           lambda on: self.event("sync", "commit", on))

    def _pick(self, control):
        def picked(point, on_face):
            st = _state(self.app)
            pos = [point.x(), point.y(), point.z()]
            if control == "aim":
                lt = st["lights"][int(self.selected)]
                lt["dir"] = [a - b for a, b in zip(pos, lt["pos"])]
            else:
                st["lights"].append(dict(kind=control, pos=pos))
                self.selected = str(len(st["lights"]) - 1)
            _store(self.app, st)
            self.invalidate()

        self.app.pick_point("Pick a point · Esc cancels", picked)

    def _sync_camera(self):
        if self.app.scene is not self.scene:
            self.sync_timer.stop()
            return
        camera = rb.camera_key(self.app.viewport.camera)
        if self.observed_camera == camera and self.last_camera != camera and self.proc is None:
            try:
                self._render()
            except Exception as exc:
                self.sync_timer.stop()
                self.status = str(exc)
            self.invalidate()
        self.observed_camera = camera

    def _render(self):
        import tempfile
        from ingetrazo_render.chrome_dialogs import Job
        rb.work_base().mkdir(parents=True, exist_ok=True)
        self.work = Path(tempfile.mkdtemp(prefix="built-render-", dir=rb.work_base()))
        args = {k: self.opts[k] for k in ("engine", "width", "height", "ground", "keep_blend")}
        args["quality"] = ["Draft", "Preview", "Final"].index(self.opts["quality"])
        from plugins.render_blender import QProcess, QProcessEnvironment
        proc = QProcess()
        self.job = Job(self.app.window, "Blender render", on_cancel=proc.kill)
        self.job.update(phase="Preparing scene")
        try:
            path = rb.write_job(self.app.scene, self.app.viewport.camera, self.work, **args, **_state(self.app))
            argv = rb.command(self.found, path)
        except Exception as exc:
            self.job.end(failed=True, detail=str(exc))
            self.job = None
            proc.deleteLater()
            raise
        env = QProcessEnvironment()
        for k, v in rb.clean_env().items():
            env.insert(k, v)
        proc.setProcessEnvironment(env)
        proc.setProcessChannelMode(QProcess.MergedChannels)
        proc.readyReadStandardOutput.connect(self._read)
        proc.finished.connect(self._finished)
        proc.errorOccurred.connect(self._error)
        self.proc = proc
        settings = QSettings()
        for key in ("engine", "width", "quality"):
            settings.setValue(_SETTINGS + key, args[key])
        settings.setValue(_SETTINGS + "ground", int(args["ground"]))
        settings.setValue(_SETTINGS + "blend", int(args["keep_blend"]))
        self.lines, self.image, self.fraction, self.status = [], None, None, "Preparing scene"
        self.partial = ""
        self.last_camera = rb.camera_key(self.app.viewport.camera)
        proc.start(argv[0], argv[1:])

    def _read(self, final=False):
        if self.proc is None:
            return
        data = self.partial + bytes(self.proc.readAllStandardOutput()).decode("utf8", "replace")
        lines = data.replace("\r", "\n").split("\n")
        self.partial = "" if final else lines.pop()
        self.lines = (self.lines + lines)[-200:]
        for line in lines:
            f = rb.progress_of(line)
            if f is not None:
                self.fraction, self.status = f, "Rendering"
                self.job.update(f, self.status)
        self.invalidate()

    def _error(self, error):
        from plugins.render_blender import QProcess
        if self.proc and error == QProcess.FailedToStart:
            self.lines.append(self.proc.errorString())
            self._finished(-1)

    def _finished(self, code, *_):
        if self.proc is None:
            return
        self._read(final=True)
        output = self.work / "render.png"
        cancelled = self.job.cancelled
        ok = code == 0 and output.is_file() and not cancelled
        if not ok:
            self.sync_timer.stop()
        self.image = output if ok else None
        self.status = "Complete" if ok else "Cancelled" if cancelled else "Failed"
        self.fraction = 1. if ok else 0.
        self.job.end(failed=not ok and not cancelled, detail=self.status)
        self.proc.deleteLater()
        self.proc = self.job = None
        self.invalidate()
        if ok:
            self._show_result()


create_panel = BlenderPanel
