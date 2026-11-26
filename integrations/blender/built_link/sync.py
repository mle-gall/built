# SPDX-License-Identifier: GPL-3.0-or-later
"""What changed on the Blender side since Built last sent each value.

Values are plain dicts in Built's terms, named by key:
``transform:<uid>`` ``{"matrix": [16 floats, column-major]}``,
``material:<name>`` ``{"color", "roughness", "metallic", "texture"}``,
``light:<id>`` ``{"kind", "pos", "dir", "color", "power", "angle", "name", "on"}``,
``camera:<name>`` ``{"eye", "target", "up", "fov_deg", "perspective"}``,
``newmaterial:<Blender name>``: a simple PBR material Built does not have
yet (the material fields plus ``name`` and ``assign``, where it sits).

For each key the add-on keeps Built's value (``base``) and what Blender
reads back right after applying it (``local``). A change is a record that
differs from ``local``; the proposal sent to Built is ``base`` with the
changed fields replaced, and names ``base`` so Built can tell when both
apps changed the same thing. Float noise from Blender's single precision
never counts as a change. No Blender import: tested on its own.
"""
KINDS = ("transform", "material", "newmaterial", "light", "camera")
TOLERANCE = 1e-4


def same(a, b):
    """Equal up to ``TOLERANCE`` for numbers; a missing key equals None."""
    if isinstance(a, bool) or isinstance(b, bool):
        return a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(a - b) <= TOLERANCE * max(1.0, abs(a), abs(b))
    if isinstance(a, (list, tuple)) and isinstance(b, (list, tuple)):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return all(same(a.get(k), b.get(k)) for k in set(a) | set(b))
    return a == b


def overlay(base, record, snapshot):
    """``base`` with the fields where ``record`` left ``snapshot``."""
    if not isinstance(base, dict) or not isinstance(snapshot, dict):
        return dict(record)
    out = dict(base)
    for k, v in record.items():
        if not same(v, snapshot.get(k)):
            out[k] = v
    return out


def kind_of(key):
    return key.split(":", 1)[0]


class Sync:
    def __init__(self):
        self.base = {}     # key → Built's value, as last received
        self.local = {}    # key → Blender's record right after applying it
        self.sent = {}     # key → the record last proposed to Built
        self.remote = {}   # key → Built's value that waited behind a local change

    def reset(self):
        self.__init__()

    # ------------------------------------------------------------ Built → Blender

    def should_apply(self, key, record_now, force=False):
        """Built sent a value for ``key``: apply it unless Blender changed
        that key since the last sync (then it waits: a conflict in Built)."""
        if force or key not in self.local or record_now is None:
            return True
        return same(record_now, self.local[key])

    def hold(self, key, value):
        self.remote[key] = value

    def accepted(self, key, value, record_after):
        """Built's ``value`` is now in Blender, which reads back ``record_after``."""
        self.base[key] = value
        self.local[key] = record_after
        self.sent.pop(key, None)
        self.remote.pop(key, None)

    def forget(self, prefix):
        """Drop every key starting with ``prefix`` (a removed placement)."""
        for table in (self.base, self.local, self.sent, self.remote):
            for k in [k for k in table if k.startswith(prefix)]:
                del table[k]

    # ------------------------------------------------------------ Blender → Built

    def proposals(self, records, kinds=KINDS, labels=None):
        """The changes to send: one item per key whose record moved."""
        labels = labels or {}
        out = []
        for key in sorted(records):
            kind = kind_of(key)
            if kind not in kinds:
                continue
            record = records[key]
            if key in self.sent and same(record, self.sent[key]):
                continue
            if key in self.local:
                if same(record, self.local[key]):
                    continue
                value = overlay(self.base.get(key), record, self.local[key])
                base = self.base.get(key)
            else:
                value, base = dict(record), None
            item = {"key": key, "kind": kind, "label": labels.get(key, key.split(":", 1)[-1]),
                    "base": base, "value": value}
            if isinstance(record, dict) and record.get("unsupported"):
                item["unsupported"] = record["unsupported"]
                item["value"] = {k: v for k, v in value.items() if k != "unsupported"}
            out.append(item)
        return out

    def mark_sent(self, items, records):
        for item in items:
            self.sent[item["key"]] = records.get(item["key"])

    def resolved(self, key, resolution, value, record_now):
        """Built answered for ``key``. ``applied``: its value is now ours;
        ``ignored`` / ``failed``: Blender keeps its own; ``kept_built`` is
        applied by the caller (``accepted``)."""
        self.sent.pop(key, None)
        if resolution == "applied":
            if value is not None:
                self.base[key] = value
            if record_now is not None:
                self.local[key] = record_now
            self.remote.pop(key, None)
        elif resolution in ("ignored", "failed"):
            if record_now is not None:
                self.local[key] = record_now
            if key in self.remote:
                self.base[key] = self.remote.pop(key)
