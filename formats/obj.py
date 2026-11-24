# SPDX-License-Identifier: GPL-3.0-or-later
"""Wavefront OBJ model interchange, units and axis conventions."""
from ingetrazo_core import (
    obj_load as load_obj,
    obj_save as save_obj,
    obj_suggest_unit as suggest_unit,
    obj_tidy_name as _tidy_blender_name,
    read_obj_text,
)

OBJ_UNITS = {"m": 1.0, "cm": 0.01, "mm": 0.001, "in": 0.0254, "ft": 0.3048}
Y_UP_TO_Z_UP = (1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0)
