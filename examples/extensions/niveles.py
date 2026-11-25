# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 José Castro Basso (FADU–UDELAR) — idea and first version.
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Niveles — building levels as a Built EXTENSION.

Levels (PB, PA, Azotea…) with their height; in an elevation or a section
seen in parallel projection, a dashed guide across the drawing at each
level, and the cursor snaps to those heights — «Nivel PA» on the tip.

José Castro Basso wrote the first version for teaching architectural
representation at FADU–UDELAR (Montevideo). It is an extension and not part
of Built on purpose: the core stays for everyone, and what a course or
an office needs is installed by whoever needs it — and can be maintained
by them. It is also the worked example of the extension API
(docs/plugins.md): document data, a side panel, a viewport overlay and an
inference, all through ``setup(app)``.

Install: copy this file into the plugins folder (Extensions ▸ Open plugins
folder) and restart Built. The «Niveles» tab appears in the side tray.
"""
from __future__ import annotations

import math

from built_core import Vec3 as QVector3D

from built_core.i18n import current_language

#: Guides and snapping only when the view looks sideways: |pitch| below this.
MAX_PITCH_DEG = 15.0
COLOR = (0.95, 0.45, 0.16)                    # Built's accent

# An extension brings its own words — the core catalogues stay the core's.
_TEXTS = {
    "es": {"Levels": "Niveles", "Level guides": "Guías de nivel",
           "Add": "Añadir", "Edit": "Editar", "Delete": "Eliminar",
           "Name:": "Nombre:", "Height:": "Altura:", "New level": "Nuevo nivel",
           "Edit level": "Editar nivel", "Level {n}": "Nivel {n}",
           "Show elevation": "Ver alzado",
           "Front view in parallel projection, where the level guides show.":
           "Vista frontal en proyección paralela, donde se ven las guías de "
           "nivel.",
           "Double-click a level to edit it. The guides show in elevations "
           "and sections in parallel projection.":
           "Doble clic en un nivel para editarlo. Las guías se ven en alzados "
           "y cortes en proyección paralela."},
    "pt-BR": {"Levels": "Níveis", "Level guides": "Guias de nível",
              "Add": "Adicionar", "Edit": "Editar", "Delete": "Excluir",
              "Name:": "Nome:", "Height:": "Altura:", "New level": "Novo nível",
              "Edit level": "Editar nível", "Level {n}": "Nível {n}",
              "Show elevation": "Ver elevação",
              "Front view in parallel projection, where the level guides show.":
              "Vista frontal em projeção paralela, onde as guias de nível "
              "aparecem.",
              "Double-click a level to edit it. The guides show in elevations "
              "and sections in parallel projection.":
              "Clique duas vezes num nível para editá-lo. As guias aparecem "
              "em elevações e cortes em projeção paralela."},
}


def _t(text: str, **kw) -> str:
    out = _TEXTS.get(current_language(), {}).get(text, text)
    return out.format(**kw) if kw else out


# ---- The data (the document's, through the API) --------------------------------

def _state(app) -> dict:
    """``{"levels": [{"name", "z"}...], "guides": bool}`` — what the
    document holds, sorted by height."""
    raw = app.document_data({}) or {}
    levels = []
    for lv in raw.get("levels", []) if isinstance(raw, dict) else []:
        try:
            levels.append({"name": str(lv["name"]), "z": float(lv["z"])})
        except (KeyError, TypeError, ValueError):
            continue
    levels.sort(key=lambda lv: lv["z"])
    return {"levels": levels, "guides": bool(raw.get("guides", True))
            if isinstance(raw, dict) else True}


def _sideways(viewport) -> bool:
    """An elevation or section in parallel projection — where heights read."""
    cam = viewport.camera
    if getattr(cam, "perspective", True):
        return False
    return abs(math.degrees(getattr(cam, "pitch", 0.0))) <= MAX_PITCH_DEG


def _units():
    """``core.units`` when the host has document units (0.5.8+), else None:
    an older Built keeps the metre labels and field it always had."""
    try:
        from core import units
    except ImportError:
        return None
    return units if hasattr(units, "format_signed_level") else None


def _fmt(z: float) -> str:
    """A level's height as a level mark, in the document's units: «+3.00»,
    «±0.00» in metres (the text printed on every sheet), «+9'10"» in feet."""
    units = _units()
    if units is None:
        return f"{z:+.2f}".replace("+0.00", "±0.00")
    return units.localized(units.format_signed_level(
        z, units.model_unit(), units.model_precision()))


def _fmt_listed(z: float) -> str:
    """:func:`_fmt` with the unit spelled out where the mark has none
    (metres): the list in the panel has no sheet around it to say so."""
    units = _units()
    if units is None or units.model_unit() == "m":
        return _fmt(z) + " m"
    return _fmt(z)


#: Floor-to-floor height offered for the second level, before there is a
#: previous one to repeat.
DEFAULT_STORY = 2.6


def _next_height(levels) -> float:
    """Where a new level goes: the first at ±0, then one floor-to-floor
    above the highest, repeating the spacing of the two highest levels (so
    10' stories keep coming at 10') or :data:`DEFAULT_STORY` before there
    are two."""
    zs = sorted(float(lv["z"]) for lv in levels)
    if not zs:
        return 0.0
    step = zs[-1] - zs[-2] if len(zs) >= 2 else 0.0
    return zs[-1] + (step if step > 1e-6 else DEFAULT_STORY)


# ---- Panel ------------------------------------------------------------------------

def _height_step(unit: str) -> float:
    """What the arrows of the height field add, in metres (#1075): 10 cm, or
    the 4 inches closest to it in a document in feet or inches."""
    return 0.10 if unit in ("m", "cm", "mm") else 4 * 0.0254


class LengthSpinBox:
    """Document-unit value for a native height editor, stored in metres."""

    def __init__(self, minimum=0.0, maximum=1000.0):
        self.units = _units()
        self.unit = self.units.model_unit()
        self.precision = self.units.fine_precision()
        self.minimum, self.maximum = minimum, maximum
        self._value = minimum
        self.step = self.units.display_step(self.unit, self.precision)

    def setValue(self, metres):
        self._value = min(self.maximum, max(self.minimum, metres))

    def value(self):
        return self._value

    def text(self):
        return self.units.localized(self.units.format_length(self._value, self.unit, self.precision))

    def read(self, text):
        result = self.units.parse(text, "length", self.unit)
        if not result.ok:
            return None, result.message()
        if not self.minimum <= result.value <= self.maximum:
            return None, _t("Out of range: {low} to {high}", low=self.minimum, high=self.maximum)
        return result.value, ""

    def stepBy(self, steps):
        self.setValue(self._value + steps * self.step)


class LevelsPanel:
    id, title, slot = "levels", _t("Levels"), "dock"

    def __init__(self, app, invalidate):
        self.app = app
        self.selected = None

    def sections(self):
        from core.plugin_panels import field, item, section
        state = _state(self.app)
        rows = [dict(key=str(i), cells=[lv["name"], _fmt_listed(lv["z"])])
                for i, lv in reversed(list(enumerate(state["levels"])))]
        editable = self.selected in {r["key"] for r in rows}
        return [section("levels", self.title,
                        item("list", "list", columns=[_t("Name:"), _t("Height:")],
                             rows=rows, selection=self.selected),
                        item("button", "add", _t("New level")),
                        item("button", "edit", _t("Edit level"), enabled=editable),
                        item("button", "delete", _t("Delete"), enabled=editable),
                        field("guides", _t("Level guides"), "toggle", state["guides"]),
                        item("button", "elevation", _t("Front elevation")))]

    def event(self, control, event, value):
        state = _state(self.app)
        levels = state["levels"]
        if control == "list":
            self.selected = value
        elif control == "guides":
            state["guides"] = value
            self.app.set_document_data(state)
        elif control == "elevation":
            vp = self.app.viewport
            vp.camera.set_view("front")
            vp.camera.perspective = False
            vp.update()
        elif control == "delete" and self.selected is not None:
            del levels[int(self.selected)]
            self.selected = None
            self.app.set_document_data(state)
        elif control in ("add", "edit"):
            from built_render.chrome_forms import Field, Form, show
            index = int(self.selected) if control == "edit" else None
            level = levels[index] if index is not None else dict(
                name=_t("Level {n}", n=len(levels) + 1), z=_next_height(levels))
            height = LengthSpinBox(-1000, 1000)
            height.setValue(level["z"])
            height.step = _height_step(height.unit)
            fields = [Field("name", "edit", _t("Name:"), level["name"]),
                      Field("height", "edit", _t("Height:"), height.text()),
                      Field("up", "button", _t("Raise level")),
                      Field("down", "button", _t("Lower level"))]

            def step(form, key):
                value, error = height.read(form.value("height"))
                if error:
                    form.error = error
                else:
                    height.setValue(value)
                    height.stepBy(1 if key == "up" else -1)
                    form.error = ""
                    form.set("height", height.text())

            scene, revision = self.app.scene, self.app.scene.version

            def validate(form, answer):
                _, error = height.read(form.value("height"))
                if error:
                    return error
                if not form.value("name").strip():
                    return _t("Enter a name")

            def finish(answer, values):
                if answer != "ok" or scene is not self.app.scene or scene.version != revision:
                    return
                changed = dict(name=values["name"].strip(),
                               z=height.read(values["height"])[0])
                if index is None:
                    levels.append(changed)
                else:
                    levels[index] = changed
                self.selected = None
                self.app.set_document_data(state)

            show(self.app.window, Form(self.title, fields, validate=validate, on_button=step, on_done=finish))

    def overlay(self, viewport):
        from views.wgpu_overlay import Primitive
        state = _state(self.app)
        if state["guides"] and _sideways(viewport):
            for level in state["levels"]:
                ends = [viewport._world_to_pixel(p) for p in _guide_ends(viewport, level["z"])]
                if None in ends:
                    continue
                segment = viewport._clip_pixel_line(*ends, margin=0.0)
                if segment:
                    yield Primitive("polyline", segment, "#f27329", dash=(4, 2))
                    x, y = min(segment)
                    yield Primitive("text", ((x + 8, y - 4),), "#f27329",
                                    text=f"{level['name']}  {_fmt(level['z'])}")


create_panel = LevelsPanel


def _guide_ends(viewport, z: float):
    """Two world points of a horizontal line at height ``z`` running across
    the view, through the model — or None when the view is not sideways."""
    cam = viewport.camera
    right = QVector3D(-math.sin(cam.yaw), math.cos(cam.yaw), 0.0)
    c = cam.target
    span = max(cam.distance * 10.0, 100.0)
    return (QVector3D(c.x(), c.y(), z) - right * span,
            QVector3D(c.x(), c.y(), z) + right * span)


def snap_to_levels(app, viewport, snap, px: float, py: float):
    st = _state(app)
    if not st["guides"] or not st["levels"] or not _sideways(viewport):
        return None
    p = snap.point
    here = viewport._world_to_pixel(p)
    if here is None:
        return None
    best, best_d = None, viewport.snap_threshold_px * 1.5
    for lv in st["levels"]:
        q = QVector3D(p.x(), p.y(), lv["z"])
        there = viewport._world_to_pixel(q)
        if there is None:
            continue
        d = abs(there[1] - here[1])
        if d <= best_d:
            best, best_d = (q, lv), d
    if best is None:
        return None
    from core.snap import SnapResult
    q, lv = best
    return SnapResult(q, "reference", COLOR, label=lv["name"])


# ---- Entry point --------------------------------------------------------------------

def setup(app) -> None:
    app.add_snap_provider(
        lambda vp, snap, px, py: snap_to_levels(app, vp, snap, px, py))
