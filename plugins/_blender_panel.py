# SPDX-License-Identifier: GPL-3.0-or-later
"""Blender panel model for the renderer-owned extension UI."""
import math
import subprocess
import tempfile
from pathlib import Path

from built_core import Vec3

from built_render import blender_availability

from built_core import render_blender as rb
from built_core.desktop_open import open_url
from built_core.i18n import tr
from built_core.settings import settings as native_settings

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
    """Write back — one undo step, the document becomes unsaved. Other
    keys (the Blender link's id) are kept."""
    raw = app.document_data({}) or {}
    raw = raw if isinstance(raw, dict) else {}
    raw.update(ambience=st["ambience"], sun_scale=st["sun_scale"], lights=st["lights"])
    app.set_document_data(raw)


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
            if line.startswith("BUILT ready"):
                self.ready = True
            elif line.startswith("BUILT done ") and self.pending is not None:
                completed, self.pending = self.pending, None
                if completed.is_file():
                    p.image = completed
                    p.status = "Following camera"
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


class BlenderLink:
    """The two-way link with Blender for one document (Render drawer).

    Built listens on loopback; the Built add-on in Blender pairs with the
    code shown here (Open in Blender passes it on). Geometry goes out as
    GLB chunks per top-level placement, then only what the change feed
    names; Blender's changes wait in ``review`` for Apply / Ignore.
    """
    BATCH = 48          # placements per GLB message
    BUDGET = 0.04       # seconds of export per tick

    def __init__(self, panel):
        self.panel, self.app = panel, panel.app
        self.server = self.tracker = None
        self.review = rb.LinkReview()
        self.peer = None
        self.notice = ""
        self.queue = []
        self.sent_values = {}
        self.values_dirty = True
        self.progress = None
        self.textures = {}
        self.show_code = False
        self.process = None
        self.unsubscribe = None
        self.review_dirty = False
        self.timer = panel.host.timers.timer(self.tick, 100, True)

    # ------------------------------------------------------------ identity

    def document_id(self, create=False):
        raw = self.app.document_data({}) or {}
        raw = raw if isinstance(raw, dict) else {}
        if not raw.get("link_id") and create:
            raw["link_id"] = rb.link_new_id()
            self.app.set_document_data(raw)
        return raw.get("link_id", "")

    def document_name(self):
        path = getattr(self.app.window, "_current_path", "") or ""
        return Path(str(path)).name if path else tr("Untitled")

    @property
    def linked(self):
        return self.peer is not None

    def state(self):
        """(chip label, available) for the drawer."""
        if self.linked:
            n, conflicts = len(self.review), self.review.conflicts
            if conflicts:
                return tr("Linked · {n} conflict").format(n=conflicts) if conflicts == 1 else \
                    tr("Linked · {n} conflicts").format(n=conflicts), False
            if n:
                return (tr("1 change from Blender") if n == 1 else
                        tr("{n} changes from Blender").format(n=n)), True
            return tr("Linked · {file}").format(file=self.peer.get("file") or "Blender"), True
        if len(self.review):
            return tr("{n} changes waiting").format(n=len(self.review)), True
        if self.server is not None and self.server.running:
            return tr("Waiting for Blender"), True
        return tr("Not linked"), False

    # ------------------------------------------------------------ lifecycle

    def start(self):
        if self.server is None:
            self.server = rb.LinkServer()
            # One tracker for the drawer's life: every step is read while
            # Blender is away too, so a reconnect gets only what changed.
            self.tracker = rb.LinkTracker()
            self._listen()
        self.server.start(rb.LINK_PORT)
        self.server.set_document(self.document_id(create=True), self.document_name())
        self.timer.start()
        return self.server.pairing

    def open_blender(self):
        pairing = self.start()
        found = self.panel.found
        if found is None:
            raise RuntimeError(tr("Blender was not found. Set its path in Settings › Blender."))
        argv = list(found.command)
        blend = native_settings().value(_SETTINGS + "blend/" + self.document_id(), "") or ""
        if blend and Path(blend).is_file():
            argv.append(blend)
        argv += ["--python", str(Path(rb.addon_source()) / "launch.py")]
        env = rb.clean_env()
        env["BUILT_LINK"] = pairing
        self.process = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                        start_new_session=True)
        self.notice = tr("Opening Blender…")

    def install_addon(self):
        dest = rb.work_base().parent / "Blender add-on" / "built_link.zip"
        path = rb.write_addon_zip(dest)
        self.notice = tr("Add-on written to {path}").format(path=path)
        self.app.window.notify("success", tr("Blender add-on saved: install it in Blender › Preferences › Add-ons"),
                               [(tr("Show in Finder"), lambda: open_url(Path(path).parent))])
        return path

    def unlink(self):
        if self.server is not None:
            self.server.disconnect("unlinked")
        self.peer = None
        self.queue = []
        self.progress = None

    def _listen(self):
        """Read each History step from the change feed as it runs (the
        hook the renderer's incremental updates use). The listener holds
        the link weakly: a closed document is not kept alive through it."""
        import weakref
        me = weakref.ref(self)

        def after(command, phase, ran):
            link = me()
            if link is None or not ran or link.tracker is None:
                return
            history = link.app.viewport.history
            step = history.feed.step(history.revision)
            if step is not None:
                link.tracker.observe(step)
            link.values_dirty = True
            link.review_dirty = True
        self.unsubscribe = self.app.viewport.history.add_listener(after=after)

    def close(self):
        if self.unsubscribe is not None:
            self.unsubscribe()
            self.unsubscribe = None
        self.timer.stop()
        if self.server is not None:
            self.server.stop()
            self.server = None
        self.peer = None

    def copy_code(self):
        from built_render.shell_services import pasteboard_write
        if self.server is None:
            self.start()
        pasteboard_write([("public.utf8-plain-text", self.server.pairing.encode())])
        self.notice = tr("Pairing code copied. Paste it in Blender › Built › Link to Built.")

    # ------------------------------------------------------------ the loop

    def tick(self):
        if self.server is None:
            self.timer.stop()
            return
        changed = False
        for msg in self.server.poll():
            self.handle(msg)
            changed = True
        if self.review_dirty:
            self.review_dirty = False
            if len(self.review):
                self.review.revalidate(rb.link_values(self.app.scene, self.app.key, self.review.keys()))
                changed = True
        if self.linked:
            if self.tracker.pending():
                self.queue.extend(self.tracker.plan(self.app.scene))
            if self.queue:
                self.send_queue()
                changed = True
            elif self.progress is not None:
                self.server.send({"type": "end"})
                self.progress = None
                changed = True
            if self.values_dirty:
                self.send_values()
        if changed:
            self.panel.invalidate()

    def handle(self, msg):
        kind = msg.get("type")
        if kind == "_linked":
            self.peer = msg
            self.notice = ""
            self.tracker.reset(msg.get("have") or {})
            self.queue, self.sent_values, self.values_dirty = [], {}, True
            if msg.get("path"):
                native_settings().setValue(_SETTINGS + "blend/" + self.document_id(), msg["path"])
        elif kind == "_closed":
            if self.peer is not None:
                self.notice = tr("Blender closed the link. It reconnects when Blender opens the file again.")
            self.peer, self.queue, self.progress = None, [], None
        elif kind == "_refused":
            self.notice = {
                "token": tr("A Blender session used an old pairing code."),
                "document": tr("{file} is linked to another Built model.").format(file=msg.get("file") or "A .blend"),
                "busy": tr("Another Blender session tried to link while one is linked."),
                "protocol": tr("Blender's add-on is from another Built version: install it again."),
            }.get(msg.get("reason"), tr("A Blender session was refused."))
        elif kind == "changes":
            self.receive(msg.get("items") or [], msg.get("_payload") or b"")

    def send_queue(self):
        import time
        end = time.monotonic() + self.BUDGET
        if self.progress is None:
            self.progress = [0, len(self.queue)]
            self.server.send({"type": "begin", "count": len(self.queue)})
        else:
            self.progress[1] = max(self.progress[1], self.progress[0] + len(self.queue))
        while self.queue and time.monotonic() < end:
            if self.server.queued > 64 << 20:
                break  # Blender is still reading: wait for the socket
            head = self.queue[0]
            if head["op"] != "chunk":
                self.queue.pop(0)
                if head["op"] == "transform":
                    self.server.send({"type": "transform", "uid": head["uid"], "matrix": head["matrix"]})
                else:
                    self.server.send({"type": "remove", "uid": head["uid"]})
                self.progress[0] += 1
                continue
            batch = []
            while self.queue and self.queue[0]["op"] == "chunk" and len(batch) < self.BATCH:
                batch.append(self.queue.pop(0))
            work = rb.work_base()
            work.mkdir(parents=True, exist_ok=True)
            fd, name = tempfile.mkstemp(suffix=".glb", prefix="built-link-", dir=work)
            import os
            os.close(fd)
            path = Path(name)
            try:
                written = set(rb.export_chunks(self.app.scene, [a["uid"] for a in batch], path))
                data = path.read_bytes() if written else b""
            except Exception as exc:
                for a in batch:
                    self.tracker.forget(a["uid"])
                self.notice = tr("Could not send part of the model: {error}").format(error=exc)
                written, data = set(), b""
            finally:
                path.unlink(missing_ok=True)
            items = [{k: a[k] for k in ("uid", "name", "matrix", "fp")} for a in batch if a["uid"] in written]
            if items:
                self.server.send({"type": "chunk", "items": items}, data)
            self.progress[0] += len(batch)

    def send_values(self):
        self.values_dirty = False
        values = rb.link_values(self.app.scene, self.app.key)
        groups = {"material:": [], "light:": [], "camera:": []}
        for key, value in values.items():
            for prefix in groups:
                if key.startswith(prefix):
                    groups[prefix].append((key, value))
        for key, value in groups["material:"]:
            if self.sent_values.get(key) != value:
                self.server.send({"type": "material", "name": key.split(":", 1)[1], "value": value})
                self.sent_values[key] = value
        for prefix, kind, field in (("light:", "lights", "id"), ("camera:", "cameras", "name")):
            now = dict(groups[prefix])
            before = {k: v for k, v in self.sent_values.items() if k.startswith(prefix)}
            if now != before:
                items = [{field: k.split(":", 1)[1], **v} for k, v in now.items()]
                self.server.send({"type": kind, "items": items})
                for k in before:
                    self.sent_values.pop(k, None)
                self.sent_values.update(now)

    # ------------------------------------------------------------ review

    def receive(self, items, payload):
        textures = rb.work_base().parent / "Blender textures"
        for it in items:
            images = it.pop("images", None) or {}
            if it.get("kind") not in ("material", "newmaterial"):
                continue
            for field, image in images.items():
                data = payload[image["offset"]:image["offset"] + image["size"]]
                if len(data) != image["size"]:
                    continue
                textures.mkdir(parents=True, exist_ok=True)
                safe = "".join(c if c.isalnum() or c in "-_." else "_" for c in image.get("name", "image.png"))
                crc = ((it.get("value") or {}).get(field) or {}).get("crc") or "image"
                path = textures / f"{crc}-{safe}"
                path.write_bytes(data)
                self.textures.setdefault(it["key"], {})[field] = str(path)
        current = rb.link_values(self.app.scene, self.app.key, [it["key"] for it in items if "key" in it])
        got = self.review.receive(items, current)
        for key in got["same"]:
            self.answer(key, "applied", current.get(key))
        if got["added"]:
            n = len(self.review)
            self.app.window.notify("info", tr("1 change from Blender") if n == 1 else
                                   tr("{n} changes from Blender").format(n=n))

    def answer(self, key, resolution, value=None, message=None):
        if self.server is not None and self.linked:
            self.server.send({"type": "resolved", "key": key, "resolution": resolution,
                              "value": value, "message": message})

    def rows(self):
        out = []
        for r in self.review.rows():
            if r["unsupported"]:
                actions = [["ignore", tr("Dismiss")]]
            elif r["conflict"]:
                actions = [["take", tr("Take Blender")], ["keep", tr("Keep Built")]]
            else:
                actions = [["apply", tr("Apply")], ["ignore", tr("Ignore")]]
            out.append(dict(key=r["key"], cells=[r["label"], r["built"], r["blender"]],
                            actions=actions, flag=bool(r["conflict"])))
        return out

    def resolve(self, keys, how):
        """Apply (one undo step), ignore, keep Built's or take Blender's."""
        if keys is None:
            keys = [i["key"] for i in self.review.items()
                    if (how == "ignore" or not i["conflict"]) and not (how == "apply" and i["unsupported"])]
        if not keys:
            return
        current = rb.link_values(self.app.scene, self.app.key, keys)
        for key in self.review.revalidate(current):
            self.answer(key, "applied", current.get(key))
        items = self.review.take([k for k in keys if k in self.review.keys()])
        if how in ("apply", "take"):
            textures = {k: p for k, p in self.textures.items() if any(i["key"] == k for i in items)}
            command, applied, failed, created = rb.link_apply(self.app.scene, items, self.app.key, textures)
            if command is not None:
                vp = self.app.viewport
                vp.history.execute(command)
                notify = getattr(vp, "notify_scene_changed", None)
                if callable(notify):
                    notify()
                vp.update()
            names = {k: "material:" + c["name"] for k, c in created.items()}
            now = rb.link_values(self.app.scene, self.app.key, [names.get(k, k) for k in applied]) if applied else {}
            notes = []
            for key in applied:
                value = now.get(names.get(key, key))
                if key in created:
                    # Blender's material is Built's now, under this name.
                    value = dict(value or {}, name=created[key]["name"])
                    if created[key].get("note"):
                        notes.append(f'{created[key]["name"]}: {created[key]["note"]}')
                self.answer(key, "applied", value)
                self.textures.pop(key, None)
            if notes:
                self.notice = "; ".join(notes)
            for key, why in failed.items():
                self.answer(key, "failed", None, why)
            if failed:
                self.notice = "; ".join(failed.values())
        elif how == "keep":
            for it in items:
                self.answer(it["key"], "kept_built", current.get(it["key"]))
        else:
            for it in items:
                self.answer(it["key"], "ignored")
                self.textures.pop(it["key"], None)
        self.panel.invalidate()

    def sections(self):
        from built_core.plugin_panels import field, item
        label, available = self.state()
        out = [item("chip", "link_state", label, available=available)]
        if self.progress is not None:
            done, total = self.progress
            out.append(item("progress", "link_progress", tr("Sending the model · {done} / {total}").format(
                done=min(done, total), total=total), fraction=min(1.0, done / max(1, total))))
        if self.notice:
            out.append(item("log", "link_notice", lines=[self.notice]))
        if self.linked:
            out.append(item("button", "link_unlink", tr("Unlink")))
        else:
            out.append(item("button", "link_open", tr("Open in Blender"), enabled=self.panel.found is not None))
            out.append(item("button", "link_pair", tr("Pairing code…")))
            out.append(item("button", "link_install", tr("Install Blender add-on…")))
        if self.show_code and self.server is not None and not self.linked:
            out.append(field("link_code", tr("Pairing code"), "text", self.server.pairing, enabled=False))
            out.append(item("button", "link_copy", tr("Copy code")))
        rows = self.rows()
        if rows:
            out.append(item("table", "link_changes", columns=[tr("Changed"), tr("Built"), tr("Blender")], rows=rows))
            plain = [r for r in self.review.items() if not r["conflict"] and not r["unsupported"]]
            if plain:
                out.append(item("button", "link_apply_all",
                                tr("Apply {n} changes").format(n=len(plain)) if len(plain) > 1 else tr("Apply 1 change")))
            out.append(item("button", "link_ignore_all", tr("Ignore all")))
        return out

    def event(self, control, event, value):
        if control == "link_open":
            self.open_blender()
        elif control == "link_pair":
            self.start()
            self.show_code = not self.show_code
        elif control == "link_copy":
            self.copy_code()
        elif control == "link_install":
            self.install_addon()
        elif control == "link_unlink":
            self.unlink()
        elif control == "link_apply_all":
            self.resolve(None, "apply")
        elif control == "link_ignore_all":
            self.resolve(None, "ignore")
        elif control == "link_changes" and event == "row_action":
            self.resolve([value["row"]], value["action"])
        else:
            return False
        return True


class BlenderPanel:
    """Blender job controller for the host-drawn panel."""
    id, title, slot = "render", "Render", "utility"
    notify_close = True

    def __init__(self, app, invalidate):
        self.app, self.invalidate = app, invalidate
        settings = native_settings()
        engine = settings.value(_SETTINGS + "engine", "eevee")
        quality = settings.value(_SETTINGS + "quality", 1, type=int)
        width = settings.value(_SETTINGS + "width", 1920, type=int)
        self.opts = dict(engine=engine if engine in ("cycles", "eevee") else "eevee",
                         quality="Final" if quality == 2 else "Draft",
                         width=max(64, min(16384, width)),
                         height=max(64, min(16384, settings.value(_SETTINGS + "height", 1080, type=int))),
                         ground=str(settings.value(_SETTINGS + "ground", "1")) != "0",
                         keep_blend=str(settings.value(_SETTINGS + "blend", "0")) != "0",
                         binary=str(settings.value(rb.SETTINGS_KEY, "") or ""))
        self.proc = self.job = self.image = self.selected = None
        self.render_count = 0
        self.recent = []
        self.recent_path = None
        self.camera = "Current view"
        self.selected_camera_key = None
        self.lights_open = bool(int(settings.value(_SETTINGS + "open_lights", 0)))
        self.scene = app.scene
        self.host = app.window._host
        self.sync_timer = self.host.timers.timer(self._sync_camera, 500, True)
        self.process_timer = self.host.timers.timer(self._poll_process, 100, True)
        from built_core.plugin_panels import on_document_close
        on_document_close(app, self._close)
        self.camera_session = CameraSession(self)
        self.observed_camera = None
        self.lines = []
        self.fraction = 0.
        self.found, self.status = blender_availability(self.opts["binary"])

        self.link = BlenderLink(self)
        self.view_pick = None
        app.add_pickable(self.pick_light, on_select=self.on_light_picked, delete=self.delete_light)
        self.pickables = app.viewport._ext_pickables

    def _settings(self, *, detect=False):
        settings = native_settings()
        engine = settings.value(_SETTINGS + "engine", "eevee")
        self.opts["engine"] = engine if engine in ("eevee", "cycles") else "eevee"
        binary = str(settings.value(rb.SETTINGS_KEY, "") or "")
        if detect or binary != self.opts["binary"]:
            self.opts["binary"] = binary
            self.found, availability = blender_availability(binary)
            if not self.proc and not self.image:
                self.status = availability

    def on_open(self):
        self._settings(detect=True)

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
        light = dict(kind=kind, pos=[point.x(), point.y(), height], kelvin=rb.DEFAULT_KELVIN, angle=60,
                     id=rb.link_new_id()[:12])
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
        from built_core.plugin_panels import field, item, section
        state = _state(self.app)
        opts = self.opts
        lights = state["lights"]
        light_rows = [dict(key=str(i), cells=[lt["name"] or lt["kind"]]) for i, lt in enumerate(lights)]
        selected = int(self.selected) if self.selected is not None else -1
        editing = 0 <= selected < len(lights)
        details = []
        if editing:
            light = lights[selected]
            details = [field("name", tr("Name"), "text", light["name"]),
                       field("on", tr("Enabled"), "toggle", light["on"]),
                       field("power", tr("Power"), "number", light["power"], min=0, max=1e6, unit=" W"),
                       field("kelvin", tr("Temperature"), "number", light["kelvin"] or rb.DEFAULT_KELVIN,
                             min=rb.MIN_KELVIN, max=rb.MAX_KELVIN, unit=" K")]
            if light["kind"] == "spot":
                heading, tilt = rb.angles_from_aim(light["dir"])
                details.extend([field("heading", tr("Heading"), "number", heading, min=0, max=360),
                                field("tilt", tr("Tilt"), "number", tilt, min=-90, max=90)])
                details.append(field("angle", tr("Cone"), "number", light["angle"], min=1, max=179, unit="°"))
            details.append(item("button", "remove", tr("Remove light")))
        cameras = [tr("Current view"), *[f"{i + 1}. {v.name}" for i, v in enumerate(self.scene.saved_views)]]
        self._settings()
        if self.camera not in cameras or self.selected_camera_key != rb.camera_key(self.app.viewport.camera):
            self.camera = cameras[0]
        result = [field("quality", tr("Preset"), "choice", tr(opts["quality"]), options=[tr("Draft"), tr("Final")]),
                  field("camera", tr("Camera"), "choice", self.camera, options=cameras),
                  item("progress", "activity", tr(self.status), fraction=self.fraction),
                  item("button", "render", tr("Render"), primary=True, enabled=self.found is not None and self.proc is None)]
        if not self.found:
            result.insert(0, item("chip", "availability", tr("Blender unavailable"), available=False))
            result.append(item("button", "settings", tr("Blender settings…")))
        if self.proc:
            result.append(item("button", "cancel", tr("Cancel render")))
        preview = []
        if self.image:
            preview.append(item("image", "preview", path=str(self.image)))
        if self.image and not self.proc:
            preview.extend([item("button", "save", tr("Save PNG…")),
                           item("button", "folder", tr("Open folder")),
                           field("sync", tr("Follow camera"), "toggle", self.sync_timer.isActive())])
        if self.recent:
            preview.extend([item("list", "recent", columns=[tr("Recent renders"), tr("Preset"), tr("Resolution")],
                                rows=[dict(key=str(path), cells=[name, quality, size])
                                      for path, name, quality, size in self.recent], selection=self.recent_path),
                           item("button", "open_image", tr("Open image"))])
        if self.lines and self.status == "Failed":
            result.append(item("log", "worker", lines=self.lines))
        lighting = [field("lights_open", tr("Show light controls"), "toggle", self.lights_open)]
        if self.lights_open:
            lighting.extend([field("ambience", tr("Ambience"), "choice", tr(state["ambience"]), options=[tr(k) for k in rb.AMBIENCES]),
                             field("sun_scale", tr("Sun intensity"), "number", state["sun_scale"], min=0.1, max=2),
                             item("button", "shadows", tr("Change date and time…")),
                             item("list", "lights", columns=[tr("Scene lights")], rows=light_rows, selection=self.selected),
                             item("button", "point", tr("Place point")), item("button", "spot", tr("Place spot")),
                             item("button", "aim", tr("Aim…"), enabled=editing), *details])
        return [section("link", tr("Blender link"), *self.link.sections()),
                section("job", "", *result),
                section("preview_output", "", *preview),
                section("output", tr("Output"),
                        field("width", tr("Width"), "number", opts["width"], min=64, max=16384, unit=" px"),
                        field("height", tr("Height"), "number", opts["height"], min=64, max=16384, unit=" px"),
                        field("ground", tr("Ground plane"), "toggle", opts["ground"]),
                        field("keep_blend", tr("Keep .blend"), "toggle", opts["keep_blend"])),
                section("lighting", tr("Lighting"), *lighting)]

    def event(self, control, event, value):
        if control.startswith("link_") and self.link.event(control, event, value):
            return
        if event == "commit" and control in ("quality", "ambience"):
            options = ("Draft", "Final") if control == "quality" else rb.AMBIENCES
            value = next((key for key in options if tr(key) == value), value)
        if event == "commit" and control in ("quality", "width", "height", "ground", "keep_blend"):
            self.opts[control] = int(value) if control in ("width", "height") else value
        elif event == "commit" and control in ("sun_scale", "ambience"):
            _store(self.app, {**_state(self.app), control: value})
        elif control == "shadows":
            self.app.window.run_command("view:form-shadows")
        elif control == "lights_open":
            self.lights_open = bool(value)
            native_settings().setValue(_SETTINGS + "open_lights", int(self.lights_open))
        elif control == "settings":
            from built_render.settings_sheet import show
            show(self.app.window, "blender")
        elif control == "folder" and self.image:
            open_url(self.image.parent)
        elif control == "_panel" and event == "closed":
            self.sync_timer.stop()
            self.camera_session.close()
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
        elif control == "camera":
            if value != tr("Current view"):
                self.scene.saved_views[int(value.split(".", 1)[0]) - 1].apply_camera(self.app.viewport.camera)
                self.app.viewport.update()
            self.selected_camera_key = rb.camera_key(self.app.viewport.camera)
            self.camera = value
        elif control == "recent":
            self.recent_path = value
        elif control == "open_image" and self.recent_path:
            open_url(Path(self.recent_path))
        elif control == "lights":
            self.selected = value
        elif control in ("point", "spot", "aim"):
            self._pick(control)
        elif control == "render" and self.proc is None:
            self._settings()
            if self.found:
                self._render()
        elif control == "cancel" and self.job:
            self.job.cancel()
        elif control == "save" and self.image:
            import shutil
            from built_render.shell_files import ShellFileDialog
            path, _ = ShellFileDialog.getSaveFileName(self.app.window, "Save image", "render.png", "PNG (*.png)")
            if path:
                shutil.copyfile(self.image, path)

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
        self.link.close()
        self.pickables[:] = [p for p in self.pickables if p["pick"] != self.pick_light]
        self.sync_timer.stop()
        self.camera_session.close()
        if self.proc is not None:
            self.job.cancel()
            self._finished(self.proc.wait())

    def _render(self):
        from built_render.chrome_dialogs import Job
        rb.work_base().mkdir(parents=True, exist_ok=True)
        self.work = Path(tempfile.mkdtemp(prefix="built-render-", dir=rb.work_base()))
        args = {k: self.opts[k] for k in ("engine", "width", "height", "ground", "keep_blend")}
        args["quality"] = ["Draft", "Preview", "Final"].index(self.opts["quality"])
        self.job = Job(self.app.window, "Blender render", on_cancel=lambda: self.proc.kill() if self.proc else None)
        self.job.update(phase="Preparing scene")
        try:
            args["sample_override"] = native_settings().value(_SETTINGS + "samples", 0, type=int)
            args["preview"] = True
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
        for key in ("width", "height", "quality"):
            settings.setValue(_SETTINGS + key, ["Draft", "Preview", "Final"].index(self.opts[key])
                              if key == "quality" else self.opts[key])
        settings.setValue(_SETTINGS + "ground", int(args["ground"]))
        settings.setValue(_SETTINGS + "blend", int(args["keep_blend"]))
        self.lines, self.image, self.fraction, self.status = [], None, None, "Preparing scene"
        self.partial = ""
        self.render_count += 1
        self.job_description = (self.opts["quality"], f'{args["width"]} × {args["height"]}')
        self.process_timer.start()

    def _read(self, final=False):
        if self.proc is None:
            return
        data = self.partial + self.output.read().decode("utf8", "replace")
        lines = data.replace("\r", "\n").split("\n")
        self.partial = "" if final else lines.pop()
        self.lines = (self.lines + lines)[-200:]
        for line in lines:
            if line.startswith("BUILT preview "):
                self.image = self.work / "render.png.preview.png"
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
        if ok:
            self.job.end(result_title="Render complete", action_label="Open image",
                         action=lambda: open_url(output))
        else:
            self.job.end(failed=not cancelled, detail=self.status)
        self.process_timer.stop()
        self.output.close()
        self.proc = self.job = None
        if ok:
            self.recent_path = str(output)
            preset, size = self.job_description
            self.recent.insert(0, (output, f"Render {self.render_count}", preset, size))
            del self.recent[12:]
        self.invalidate()


create_panel = BlenderPanel
