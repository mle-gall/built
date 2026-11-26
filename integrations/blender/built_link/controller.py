# SPDX-License-Identifier: GPL-3.0-or-later
"""The link's behaviour in Blender, independent of ``bpy``.

``Controller`` reads Built's messages and drives a *host* (in Blender:
``blender_scene.Host``; in tests: a fake) that imports geometry, places
objects and reads records. It sends what changed back as one ``changes``
batch. Geometry always flows from Built; Blender proposes materials,
lights, cameras and placement transforms, which Built reviews.
"""
import time

from . import sync as sync_mod
from . import wire
from .client import Client

IMAGE_FIELDS = ("texture", "normal_map", "roughness_map", "metallic_map", "ao_map", "emission_map")


class Controller:
    def __init__(self, host, client=None):
        self.host = host
        self.client = client or Client()
        self.sync = sync_mod.Sync()
        self.document_name = ""
        self.progress = None          # (done, total) while Built sends the model
        self.last_sync = None
        self.notice = ""
        self.refused_reason = ""
        self.received = 0

    # ------------------------------------------------------------ control

    def link(self, pairing, relink=False):
        port, token = wire.pairing(pairing)
        self.refused_reason = ""
        self.notice = ""
        self.client.start(port, token, self.hello(), relink=relink)

    def unlink(self):
        self.client.stop()
        self.progress = None

    def hello(self):
        return {"document": self.host.document() or None, "file": self.host.file_name(), "path": self.host.file_path(),
                "blender": self.host.version(), "have": self.host.have()}

    @property
    def state(self):
        return self.client.state

    def status(self):
        """One line for the sidebar."""
        s = self.client.state
        if s == "linked":
            if self.progress:
                return f"Receiving the model · {self.progress[0]} / {self.progress[1]}"
            return f"Linked to {self.document_name or 'Built'}"
        if s == "connecting":
            return "Connecting to Built…"
        if s == "waiting":
            return "Built closed · reconnecting"
        if s == "refused":
            return self.client.message or "Built refused the link"
        return "Not linked"

    # ------------------------------------------------------------ Built → Blender

    def tick(self, budget=0.05):
        """Handle queued messages for at most ``budget`` seconds."""
        end = time.monotonic() + budget
        handled = 0
        if self.client.state in ("connecting", "waiting"):
            self.client.hello = self.hello()
        if self.client.state == "linked":
            self.host.assign_ids()
        while time.monotonic() < end:
            batch = self.client.drain(1)
            if not batch:
                break
            header, payload = batch[0]
            self.handle(header, payload)
            handled += 1
        return handled

    def _apply(self, key, value, force=False):
        now = self.host.record(key)
        if not self.sync.should_apply(key, now, force):
            self.sync.hold(key, value)
            return False
        self.host.apply(key, value)
        self.sync.accepted(key, value, self.host.record(key))
        return True

    def handle(self, header, payload=b""):
        kind = header.get("type")
        if kind == "_linked":
            self.document_name = header.get("name", "")
            self.host.set_document(header.get("document") or "", self.document_name)
            self.notice = ""
        elif kind == "_refused":
            self.refused_reason = header.get("reason", "")
            self.notice = header.get("message", "")
        elif kind == "_closed":
            self.progress = None
            self.notice = header.get("reason", "")
        elif kind == "begin":
            self.progress = (0, int(header.get("count", 0)))
        elif kind == "end":
            self.progress = None
            self.last_sync = time.time()
        elif kind == "chunk":
            items = header.get("items") or []
            self.host.import_glb(payload, items)
            for it in items:
                key = "transform:" + it["uid"]
                self.sync.accepted(key, {"matrix": it["matrix"]}, self.host.record(key))
            if self.progress:
                self.progress = (min(self.progress[1], self.progress[0] + len(items)), self.progress[1])
            self.received += len(items)
        elif kind == "transform":
            self._apply("transform:" + header["uid"], {"matrix": header["matrix"]}, header.get("force", False))
        elif kind == "remove":
            self.host.remove(header["uid"])
            self.sync.forget("transform:" + header["uid"])
        elif kind == "material":
            self._apply("material:" + header["name"], header["value"], header.get("force", False))
        elif kind in ("lights", "cameras"):
            prefix = "light:" if kind == "lights" else "camera:"
            field = "id" if kind == "lights" else "name"
            keep = set()
            for it in header.get("items") or []:
                key = prefix + str(it[field])
                keep.add(key)
                self._apply(key, {k: v for k, v in it.items() if k != field})
            for key in self.host.keys(prefix):
                if key not in keep and key in self.sync.local and self.sync.should_apply(key, self.host.record(key)):
                    self.host.delete(key)
                    self.sync.forget(key)
        elif kind == "resolved":
            key, how = header["key"], header.get("resolution")
            if key.startswith("newmaterial:") and how == "applied" and (header.get("value") or {}).get("name"):
                # Built made it its own: from now on it is a Built material.
                value = dict(header["value"])
                built = value.pop("name")
                self.host.adopt(key.split(":", 1)[1], built)
                self.sync.forget(key)
                self.sync.accepted("material:" + built, value, self.host.record("material:" + built))
            elif how == "kept_built" and header.get("value") is not None:
                self._apply(key, header["value"], force=True)
            else:
                self.sync.resolved(key, how, header.get("value"), self.host.record(key))
            if header.get("message"):
                self.notice = header["message"]

    # ------------------------------------------------------------ Blender → Built

    def changes(self, kinds=sync_mod.KINDS):
        records, labels = self.host.records()
        return self.sync.proposals(records, kinds, labels), records

    def send(self, kinds=sync_mod.KINDS):
        """Send what changed as one batch; returns how many items went."""
        self.host.assign_ids()
        items, records = self.changes(kinds)
        if not items:
            return 0
        payload = bytearray()
        for item in items:
            if item["kind"] not in ("material", "newmaterial") or item.get("unsupported"):
                continue
            for field in IMAGE_FIELDS:
                new, old = item["value"].get(field), (item["base"] or {}).get(field)
                if not new or (old and new.get("crc") == old.get("crc")):
                    continue
                data = self.host.image_bytes(item["key"], field)
                if data:
                    item.setdefault("images", {})[field] = {
                        "offset": len(payload), "size": len(data), "name": new.get("name") or field + ".png"}
                    payload.extend(data)
        if not self.client.send({"type": "changes", "items": items}, bytes(payload)):
            return 0
        self.sync.mark_sent(items, records)
        return len(items)
