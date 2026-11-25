# SPDX-License-Identifier: GPL-3.0-or-later
"""Blender panel model for the renderer-owned extension UI."""
import math
import subprocess
import tempfile
from pathlib import Path

from ingetrazo_core import Vec3

from ingetrazo_render import blender_availability

from core import render_blender as rb
from core.desktop_open import open_url
from core.settings import settings as native_settings

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


class CameraSession:
    """Keep one background Blender for consecutive views of the same scene."""

    def __init__(self, panel):
        self.panel = panel
        self.process = None
        self.scene_key = None
        self.last_camera = None
        self.pending = None
        self.ready = False
        self.count = 0
        self.partial = ""

    def close(self):
        if self.process is not None:
            try:
                self.process.stdin.write(b'{"quit":true}\n')
                self.process.stdin.flush()
            except (BrokenPipeError, OSError):
                pass
            try:
                self.process.stdin.close()
            except BrokenPipeError:
                pass
            try:
                self.process.wait(timeout=1.5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
            self.output.close()
            self.process = None
        self.scene_key = self.last_camera = self.pending = None
        self.ready = False
        self.partial = ""

    def _load(self, scene_key):
        self.close()
        panel = self.panel
        base = rb.work_base()
        base.mkdir(parents=True, exist_ok=True)
        self.work = Path(tempfile.mkdtemp(prefix="built-sync-", dir=base))
        path = rb.write_job(panel.scene, panel.app.viewport.camera, self.work,
                            engine=panel.opts["engine"], ground=panel.opts["ground"],
                            quality=0, width=self.width, height=self.height,
                            serve=True, **_state(panel.app))
        log_path = self.work / "blender.log"
        with log_path.open("wb") as log:
            self.process = subprocess.Popen(rb.command(panel.found, path), stdin=subprocess.PIPE,
                                            stdout=log, stderr=subprocess.STDOUT, env=rb.clean_env())
        self.output = log_path.open("rb")
        self.scene_key = scene_key

    def poll(self, settled):
        p = self.panel
        scene_key = p._sync_scene_key()
        if self.scene_key != scene_key:
            if settled:
                ratio = min(1, rb.SYNC_WIDTH / max(p.opts["width"], p.opts["height"]))
                self.width = max(64, round(p.opts["width"] * ratio))
                self.height = max(64, round(p.opts["height"] * ratio))
                self._load(scene_key)
            return
        if self.process.poll() is not None:
            raise RuntimeError("Blender stopped while following the camera")
        text = self.partial + self.output.read().decode("utf8", "replace")
        *lines, self.partial = text.split("\n")
        for line in lines:
            if line.startswith("INGETRAZO ready"):
                self.ready = True
            elif line.startswith("INGETRAZO done ") and self.pending is not None:
                completed, self.pending = self.pending, None
                if completed.is_file():
                    p.image = completed
                    p.status = "Following camera"
                    p._show_result()
                    p.invalidate()
        camera = rb.camera_key(p.app.viewport.camera)
        if settled and self.ready and self.pending is None and camera != self.last_camera:
            self.count += 1
            self.pending = self.work / f"sync-{self.count:04d}.png"
            (self.work / f"sync-{self.count - 3:04d}.png").unlink(missing_ok=True)
            message = rb.sync_request(p.app.viewport.camera, self.pending, self.width,
                                      self.height, p.opts["engine"])
            self.process.stdin.write(message.encode())
            self.process.stdin.flush()
            self.last_camera = camera


class BlenderPanel:
    """Blender job controller for the host-drawn panel."""
    id, title, slot = "render", "Blender Render", "dock"

    def __init__(self, app, invalidate):
        self.app, self.invalidate = app, invalidate
        settings = native_settings()
        engine = settings.value(_SETTINGS + "engine", "eevee")
        quality = settings.value(_SETTINGS + "quality", 1, type=int)
        width = settings.value(_SETTINGS + "width", 1920, type=int)
        self.opts = dict(engine=engine if engine in ("cycles", "eevee") else "eevee",
                         quality=["Draft", "Preview", "Final"][max(0, min(2, quality))],
                         width=max(64, min(16384, width)), height=1080,
                         ground=str(settings.value(_SETTINGS + "ground", "1")) != "0",
                         keep_blend=str(settings.value(_SETTINGS + "blend", "0")) != "0",
                         binary=str(settings.value(rb.SETTINGS_KEY, "") or ""))
        self.proc = self.job = self.image = self.selected = self.viewer = None
        self.lights_open = bool(int(settings.value(_SETTINGS + "open_lights", 1)))
        self.scene = app.scene
        self.host = app.window._host
        self.sync_timer = self.host.timers.timer(self._sync_camera, 500, True)
        self.process_timer = self.host.timers.timer(self._poll_process, 100, True)
        from core.plugin_panels import on_document_close
        on_document_close(app, self._close)
        self.camera_session = CameraSession(self)
        self.observed_camera = None
        self.lines = []
        self.fraction = 0.
        self.found, self.status = blender_availability(self.opts["binary"])

        self.view_pick = None
        app.add_pickable(self.pick_light, on_select=self.on_light_picked, delete=self.delete_light)
        self.pickables = app.viewport._ext_pickables

    def pick_light(self, viewport, x, y):
        if viewport.scene is not self.scene:
            return None
        closest, distance = None, 12.0
        for index, light in enumerate(_state(self.app)["lights"]):
            pixel = viewport._world_to_pixel(Vec3(*light["pos"]))
            if pixel is not None:
                delta = math.hypot(pixel[0] - x, pixel[1] - y)
                if delta <= distance:
                    closest, distance = (index, self.scene.version), delta
        return closest

    def on_light_picked(self, picked):
        self.view_pick = picked
        if picked is not None:
            self.selected = str(picked[0])
        self.invalidate()

    def delete_light(self, picked):
        index, revision = picked
        if self.app.scene is self.scene and revision == self.scene.version:
            state = _state(self.app)
            del state["lights"][index]
            self.selected = None
            _store(self.app, state)
        self.view_pick = None
        self.invalidate()

    def add_light(self, kind, point, on_face=False):
        state = _state(self.app)
        height = point.z() + (0.1 if on_face and kind == "point" else 0)
        light = dict(kind=kind, pos=[point.x(), point.y(), height], kelvin=rb.DEFAULT_KELVIN, angle=60)
        state["lights"].extend(rb.clean_lights([light]))
        self.selected = str(len(state["lights"]) - 1)
        _store(self.app, state)
        self.invalidate()

    def overlay(self, viewport):
        from views.wgpu_overlay import Primitive
        for index, light in enumerate(_state(self.app)["lights"]):
            origin = Vec3(*light["pos"])
            pixel = viewport._world_to_pixel(origin)
            if pixel is None:
                continue
            x, y = pixel
            tint = ("#%02x%02x%02x" % tuple(round(255*c) for c in light["color"])
                    if light["on"] else "#969696")
            if light["kind"] == "spot":
                tip = viewport._world_to_pixel(origin + Vec3(*light["dir"]).normalized() * 0.8)
                if tip is not None:
                    yield Primitive("polyline", (pixel, tip), tint, dash=(4, 2))

            def ring(radius):
                return tuple((x + radius * math.cos(i * math.tau / 32),
                              y + radius * math.sin(i * math.tau / 32)) for i in range(33))

            yield Primitive("polygon", ring(6), "#282828", fill=tint)
            for i in range(8):
                angle = i * math.pi / 4
                ray = tuple((x + r * math.cos(angle), y + r * math.sin(angle)) for r in (8, 11))
                yield Primitive("polyline", ray, tint)
            if self.view_pick == (index, self.scene.version):
                yield Primitive("polyline", ring(15), "#f27329", 3)
            if self.selected == str(index):
                yield Primitive("polyline", ring(14), "#3689e6", 2)
                label = light["name"] or light["kind"]
                yield Primitive("text", ((x + 16, y - 10),), "#141414",
                                text=f"{label} · {int(light['power'])} W")

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
                heading, tilt = rb.angles_from_aim(light["dir"])
                details.extend([field("heading", "Heading", "number", heading, min=0, max=360),
                                field("tilt", "Tilt", "number", tilt, min=-90, max=90)])
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
        lighting = [field("lights_open", "Show light controls", "toggle", self.lights_open)]
        if self.lights_open:
            lighting.extend([field("ambience", "Ambience", "choice", state["ambience"], options=list(rb.AMBIENCES)),
                             field("sun_scale", "Sun intensity", "number", state["sun_scale"], min=0.1, max=2),
                             item("button", "shadows", "Change date and time…"),
                             item("list", "lights", columns=["Scene lights"], rows=light_rows, selection=self.selected),
                             item("button", "point", "Place point"), item("button", "spot", "Place spot"),
                             item("button", "aim", "Aim…", enabled=editing), *details])
        return [section("output", "Output",
                        field("binary", "Blender", "path", opts["binary"] or (self.found.where if self.found else "")),
                        field("engine", "Engine", "choice", opts["engine"], options=["eevee", "cycles"]),
                        field("quality", "Quality", "choice", opts["quality"], options=["Draft", "Preview", "Final"]),
                        field("width", "Width", "number", opts["width"], min=64, max=16384, unit=" px"),
                        field("height", "Height", "number", opts["height"], min=64, max=16384, unit=" px"),
                        field("ground", "Ground plane", "toggle", opts["ground"]),
                        field("keep_blend", "Keep .blend", "toggle", opts["keep_blend"])),
                section("lighting", "Lighting", *lighting),
                section("job", "Render", *result)]

    def event(self, control, event, value):
        if event == "commit" and control in self.opts:
            self.opts[control] = int(value) if control in ("width", "height") else value
            if control == "binary":
                native_settings().setValue(rb.SETTINGS_KEY, value)
                self.found, self.status = blender_availability(value)
        elif event == "commit" and control in ("sun_scale", "ambience"):
            _store(self.app, {**_state(self.app), control: value})
        elif control == "shadows":
            self.app.window.run_command("view:form-shadows")
        elif control == "lights_open":
            self.lights_open = bool(value)
            native_settings().setValue(_SETTINGS + "open_lights", int(self.lights_open))
        elif control == "sync":
            if value:
                self.observed_camera = None
                self.sync_timer.start()
            else:
                self.sync_timer.stop()
                self.camera_session.close()
        elif control in ("name", "power", "kelvin", "angle", "on", "remove", "heading", "tilt"):
            st = _state(self.app)
            if self.selected is None:
                return
            idx = int(self.selected)
            if not 0 <= idx < len(st["lights"]):
                self.selected = None
                return
            if control == "remove":
                del st["lights"][idx]
                self.selected = None
            elif control in ("heading", "tilt"):
                heading, tilt = rb.angles_from_aim(st["lights"][idx]["dir"])
                st["lights"][idx]["dir"] = rb.aim_from_angles(
                    value if control == "heading" else heading,
                    value if control == "tilt" else tilt)
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
            self.viewer = _render_result(self.app, self.image, self.sync_timer.isActive(),
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
                self.add_light(control, point, on_face)
                return
            _store(self.app, st)
            self.invalidate()

        self.app.pick_point("Pick a point · Esc cancels", picked)

    def _sync_camera(self):
        if self.app.scene is not self.scene or not self.found:
            self.sync_timer.stop()
            self.camera_session.close()
            return
        camera = rb.camera_key(self.app.viewport.camera)
        settled = camera == self.observed_camera and not self.host.input_state.buttons and self.proc is None
        self.observed_camera = None if self.host.input_state.buttons else camera
        try:
            self.camera_session.poll(settled)
        except Exception as exc:
            self.sync_timer.stop()
            self.camera_session.close()
            self.status = str(exc)
            self.invalidate()

    def _sync_scene_key(self):
        return self.scene.content_version, tuple(self.opts.items())

    def _close(self):
        self.pickables[:] = [p for p in self.pickables if p["pick"] != self.pick_light]
        self.sync_timer.stop()
        self.camera_session.close()
        if self.proc is not None:
            self.job.cancel()
            self._finished(self.proc.wait())

    def _render(self):
        from ingetrazo_render.chrome_dialogs import Job
        rb.work_base().mkdir(parents=True, exist_ok=True)
        self.work = Path(tempfile.mkdtemp(prefix="built-render-", dir=rb.work_base()))
        args = {k: self.opts[k] for k in ("engine", "width", "height", "ground", "keep_blend")}
        args["quality"] = ["Draft", "Preview", "Final"].index(self.opts["quality"])
        self.job = Job(self.app.window, "Blender render", on_cancel=lambda: self.proc.kill() if self.proc else None)
        self.job.update(phase="Preparing scene")
        try:
            path = rb.write_job(self.app.scene, self.app.viewport.camera, self.work, **args, **_state(self.app))
            argv = rb.command(self.found, path)
            log_path = self.work / "blender.log"
            with log_path.open("wb") as log:
                self.output = log_path.open("rb")
                try:
                    self.proc = subprocess.Popen(argv, env=rb.clean_env(), stdout=log,
                                                 stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
                except BaseException:
                    self.output.close()
                    raise
        except Exception as exc:
            self.job.end(failed=True, detail=str(exc))
            self.job = None
            raise
        settings = native_settings()
        for key in ("engine", "width", "quality"):
            settings.setValue(_SETTINGS + key, ["Draft", "Preview", "Final"].index(self.opts[key])
                              if key == "quality" else self.opts[key])
        settings.setValue(_SETTINGS + "ground", int(args["ground"]))
        settings.setValue(_SETTINGS + "blend", int(args["keep_blend"]))
        self.lines, self.image, self.fraction, self.status = [], None, None, "Preparing scene"
        self.partial = ""
        self.process_timer.start()

    def _read(self, final=False):
        if self.proc is None:
            return
        data = self.partial + self.output.read().decode("utf8", "replace")
        lines = data.replace("\r", "\n").split("\n")
        self.partial = "" if final else lines.pop()
        self.lines = (self.lines + lines)[-200:]
        for line in lines:
            f = rb.progress_of(line)
            if f is not None:
                self.fraction, self.status = f, "Rendering"
                self.job.update(f, self.status)
        self.invalidate()

    def _poll_process(self):
        code = self.proc.poll()
        if code is None:
            self._read()
        else:
            self._finished(code)

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
        self.process_timer.stop()
        self.output.close()
        self.proc = self.job = None
        self.invalidate()
        if ok:
            self._show_result()


create_panel = BlenderPanel
