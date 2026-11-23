# SPDX-License-Identifier: GPL-3.0-or-later
"""Bind the Rust first-run units draft to document settings and the chrome."""
import json

from PySide6.QtCore import QSettings

from core import units
from core.i18n import tr
from views import chrome_forms
from views.settings_apply import _UNITS_FOR_NEW_KEY, apply_document_units


class UnitsQuestion(chrome_forms.Form):
    def __init__(self, model, done):
        super().__init__(tr("Units"), [], key="start_units", width=520, on_done=done)
        self.model = model

    def receive(self, fid, raw):
        self.model.receive(fid, raw)
        self.changed()

    def wire(self):
        seq, title, note, width, _, buttons = super().wire()
        settings = json.loads(self.model.settings())
        length = settings["length"]
        auto = tuple(tr("By the length ({unit})", unit=units.UNITS[fn(length)].symbol)
                     for fn in (units.area_unit_for, units.volume_unit_for))
        rows = self.model.rows(tr, units.unit_label, units.preview_text(settings), auto, units.FRACTION_LABELS)
        return seq, title, note, width, rows, buttons


def offer_at_start(window):
    from ingetrazo_render import UnitsDraft
    if not units.ask_at_start():
        return
    view = window.viewport
    scene = view.scene
    model = UnitsDraft(json.dumps(units.new_document_units()), json.dumps(units.model_units_of(scene)),
                       json.dumps(units.PROFILES), [units.UNIT_CHOICES, units.AREA_CHOICES,
                                                   units.VOLUME_CHOICES, units.ANGLE_CHOICES])

    def done(answer, _):
        if window.viewport is not view or view.scene is not scene:
            return
        custom = model.custom
        operation = model.answer(answer == "ok")
        if not custom and not model.ask_again:
            units.set_ask_at_start(False)
        if operation == "custom":
            chrome_forms.show(window, UnitsQuestion(model, done))
        elif operation == "apply":
            chosen = json.loads(model.settings())
            if custom:
                untouched = scene.version == window._saved_version
                apply_document_units(window, chosen)
                if untouched:
                    window._saved_version = scene.version
                    window._update_title()
                QSettings().setValue(_UNITS_FOR_NEW_KEY, "1" if model.for_new else "0")
                units.set_ask_at_start(model.ask_again)
            else:
                units.apply_units(scene, chosen)
                view.update()
            if model.for_new:
                units.remember_new_document_units(chosen)

    chrome_forms.show(window, UnitsQuestion(model, done))
