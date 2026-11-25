# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Geo-referencing: satellite tiles, DEM (terrain elevation), projections."""

import sys
from ingetrazo_core import register_survey as _register_survey

_register_survey(sys.modules[__name__])
