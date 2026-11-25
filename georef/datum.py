# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Geographic coordinates and the scene's local metre frame."""
from dataclasses import dataclass, field
from ingetrazo_core import SceneDatum as _Datum, utm_forward, utm_inverse, zone_for_lon


@dataclass(init=False)
class SceneDatum(_Datum):
    """The four public datum fields are also available to dataclass consumers."""

    lat: float = field()
    lon: float = field()
    alt: float = field()
    north: float = field()
