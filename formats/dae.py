# SPDX-License-Identifier: GPL-3.0-or-later
"""COLLADA model interchange and the editable/reference import thresholds."""
from ingetrazo_core import (
    dae_add_fused as _add_fused,
    dae_load as load_dae,
    dae_placements as _placements,
    dae_save as save_dae,
)

_MAX_FUSE_LOOPS = 400
_MAX_PLACEMENTS = 1_000_000
_INST_MIN_SAVED = 400
_INST_MIN_POLYS = 60
_MAX_GROUPS = 250
_SPLIT_MIN = 4000
