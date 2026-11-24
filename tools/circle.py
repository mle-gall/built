# SPDX-License-Identifier: GPL-3.0-or-later
"""Compatibility import for extensions written against the Python Circle and
Polygon. The tools are a Rust state machine now
(rust/ingetrazo_core/src/tools/circle.rs); the commit's gates live in
core.arrangement."""
from core.arrangement import busy_plane, flat_drawing  # noqa: F401
from tools.rust_tools import CircleTool, PolygonTool  # noqa: F401
