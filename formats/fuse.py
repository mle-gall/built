# SPDX-License-Identifier: GPL-3.0-or-later
"""Coplanar fusion and smoothing for editable imported geometry."""
from ingetrazo_core import (
    drop_smoothing_groups,
    fuse_coplanar_loops,
    fuse_smooth_sig as _smooth_sig,
    simplify_mesh,
    soften_by_smoothing_group,
    soften_smooth_edges,
)

SMOOTH_KEY = "_s"
