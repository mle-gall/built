# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Surface fill for a surveyed polygon."""
from ingetrazo_core import (
    ground_reference, fit_plane as _fit_plane, build_flat_surface, build_draped_surface, build_surface,
)
Tri = tuple
