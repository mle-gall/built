# SPDX-License-Identifier: GPL-3.0-or-later
"""Soften / Smooth Edges…: the angle under which an edge goes soft (see
:mod:`core.soften`)."""
from __future__ import annotations

from PySide6.QtCore import QSettings, Qt
from PySide6.QtWidgets import (QCheckBox, QDialog, QDialogButtonBox,
                               QDoubleSpinBox, QHBoxLayout, QLabel, QSlider,
                               QVBoxLayout)

from core.i18n import tr

#: Where the last angle is remembered.
ANGLE_KEY = "tools/soften_angle"


def _remembered_angle() -> float:
    try:
        value = float(QSettings().value(ANGLE_KEY, 30.0))
    except (TypeError, ValueError):
        value = 30.0
    return min(180.0, max(0.0, value))


def remember_angle(angle: float) -> None:
    QSettings().setValue(ANGLE_KEY, float(angle))


class SoftenDialog(QDialog):
    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        self.setWindowTitle(tr("Soften / Smooth Edges"))
        self.setModal(True)
        layout = QVBoxLayout(self)
        hint = QLabel(tr(
            "Edges between faces at this angle or less become smooth; "
            "sharper edges stay hard. Selected groups include their nested "
            "geometry, and component copies share the change."))
        hint.setWordWrap(True)
        layout.addWidget(hint)
        row = QHBoxLayout()
        row.addWidget(QLabel(tr("Angle between faces")))
        self.slider = QSlider(Qt.Horizontal)
        self.slider.setRange(0, 1800)
        self.angle = QDoubleSpinBox()
        self.angle.setRange(0.0, 180.0)
        self.angle.setDecimals(1)
        self.angle.setSuffix("°")
        self.slider.valueChanged.connect(lambda v: self.angle.setValue(v / 10))
        self.angle.valueChanged.connect(
            lambda v: self.slider.setValue(round(v * 10)))
        self.angle.setValue(_remembered_angle())
        row.addWidget(self.slider)
        row.addWidget(self.angle)
        layout.addLayout(row)
        self.coplanar = QCheckBox(tr("Soften coplanar edges"))
        self.coplanar.setChecked(True)
        layout.addWidget(self.coplanar)
        self.materials = QCheckBox(tr("Soften between different materials"))
        self.materials.setChecked(False)
        layout.addWidget(self.materials)
        self.harden = QCheckBox(tr("Unsoften all selected edges"))
        for widget in (self.angle, self.slider, self.coplanar, self.materials):
            self.harden.toggled.connect(
                lambda on, w=widget: w.setEnabled(not on))
        layout.addWidget(self.harden)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok
                                   | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        layout.addWidget(buttons)
        self.resize(470, 220)
