# SPDX-License-Identifier: GPL-3.0-or-later
"""Compatibility imports for extensions written against the Python arcs. The
tools are Rust state machines now (rust/ingetrazo_core/src/tools/arc.rs); the
commit is the protocol's ``add_arc``."""
import json

from tools.rust_tools import (COLOR_FILLET, ArcTool, CenterArcTool,  # noqa: F401
                              PieTool, ThreePointArcTool)


def commit_arc(viewport, pts, close_to=None, trim=None):
    """Draw an arc polyline (Freehand's commit): the protocol's ``add_arc``."""
    from tools.rust_tools import _Host, _tuple
    args = {"points": [list(_tuple(p)) for p in pts]}
    if close_to is not None:
        args["center"] = list(_tuple(close_to))
    if trim:
        args["trim"] = [[list(_tuple(a)), list(_tuple(b))] for a, b in trim]
    return _Host(viewport).run("add_arc", json.dumps(args))
