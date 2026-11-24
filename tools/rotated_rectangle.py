# SPDX-License-Identifier: GPL-3.0-or-later
"""Compatibility import for extensions written against the Python Rotated
Rectangle. The tool is a Rust state machine now
(rust/ingetrazo_core/src/tools/rotated_rectangle.rs)."""
from tools.rust_tools import RotatedRectangleTool, _degrees  # noqa: F401
