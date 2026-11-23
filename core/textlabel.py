# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Leader text — an annotation label pointing at the model.

A ``TextLabel`` records the world ``anchor`` it points at (snapped to
geometry when placed) and an ``offset`` to where the label floats; a leader
line joins them. Like ``Dimension`` it is an annotation, not geometry: it
lives in ``Scene.text_labels`` and is drawn as a screen-space overlay.
"""
from __future__ import annotations

from core.doc_model import Record, record_fields

from ingetrazo_core import Vec3 as QVector3D


@record_fields("_anchor", "offset", "text", "layer", "attachment")
class TextLabel(Record):
    def __init__(self, anchor, offset, text, layer=None, attachment=None):
        self._anchor = anchor
        self.offset = offset
        self.text = text
        self.layer = layer
        self.attachment = attachment

    @property
    def anchor(self):
        if self.attachment is not None:
            point = self.attachment.position()
            if point is not None and point != self._anchor:
                self._anchor = point
        return self._anchor

    @anchor.setter
    def anchor(self, value):
        self._anchor = value

    def position(self) -> QVector3D:
        """Where the label text floats (anchor + offset)."""
        return self.anchor + self.offset

    # ---- Serialisation ------------------------------------------------------
    def to_dict(self) -> dict:
        binding = self.attachment.archive() if self.attachment is not None else None
        return {
            "anchor": list(self.anchor.toTuple()),
            "offset": [self.offset.x(), self.offset.y(), self.offset.z()],
            "text": self.text,
            **({"attachment": binding} if binding else {}),
            **({"layer": self.layer} if self.layer else {}),
        }

    @classmethod
    def from_dict(cls, data: dict, scene=None) -> "TextLabel":
        from core.annotation_anchor import Attachment
        point = QVector3D(*data.get("anchor", [0, 0, 0]))
        binding = Attachment.restore(scene, data.get("attachment"), point) if scene is not None else None
        return cls(point,
                   QVector3D(*data.get("offset", [0, 0, 0])),
                   data.get("text", ""),
                   layer=data.get("layer") or None, attachment=binding)
